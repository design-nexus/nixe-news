//! RSS, Atom, and JSON Feed via `feed-rs` 2.4. Bodies are sanitized before they
//! leave this module. A document larger than the cap is refused before parsing.

use crate::html::{excerpt, sanitize};
use crate::model::{ParsedFeed, ParsedItem};
use crate::normalize::canonical_url;
use anyhow::{Context, Result, bail};
use feed_rs::model::{Entry, Link};
use sha2::{Digest, Sha256};

pub const MAX_FEED_BYTES: usize = 2 * 1024 * 1024;

pub fn parse_feed(bytes: &[u8]) -> Result<ParsedFeed> {
    if bytes.len() > MAX_FEED_BYTES {
        bail!("the feed is larger than {} bytes", MAX_FEED_BYTES);
    }
    let normalized;
    let slice: &[u8] = if let Ok(s) = std::str::from_utf8(bytes) {
        if s.contains("<item>") && s.contains("<image>") {
            normalized = normalize_item_images(s);
            normalized.as_bytes()
        } else {
            bytes
        }
    } else {
        bytes
    };
    let feed = feed_rs::parser::parse(slice).context("the feed could not be parsed")?;
    let title = feed.title.as_ref().map(|t| t.content.trim().to_string()).unwrap_or_default();
    let site_url = feed.links.first().map(|l| l.href.clone()).unwrap_or_default();
    let mut items = Vec::new();
    for entry in &feed.entries {
        if let Some(item) = item(entry) {
            items.push(item);
        }
    }
    Ok(ParsedFeed { title, site_url, items })
}

fn normalize_item_images(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() + 256);
    let mut rest = xml;
    while let Some(start) = rest.find("<image>") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 7..];
        if let Some(end) = after.find("</image>") {
            let url = after[..end].trim();
            if url.starts_with("http://") || url.starts_with("https://") {
                out.push_str(&format!("<enclosure url=\"{url}\" type=\"image/jpeg\"/><image>{url}</image>"));
            } else {
                out.push_str("<image>");
                out.push_str(url);
                out.push_str("</image>");
            }
            rest = &after[end + 8..];
        } else {
            out.push_str("<image>");
            rest = after;
            break;
        }
    }
    out.push_str(rest);
    out
}

fn item(entry: &Entry) -> Option<ParsedItem> {
    let original = entry_url(entry)?;
    let url = canonical_url(&original).unwrap_or(original.clone());
    let raw_id = entry.id.trim();
    // feed-rs fills a 32-character hex id when the feed has no guid or atom id.
    let generated = raw_id.len() == 32 && raw_id.chars().all(|c| c.is_ascii_hexdigit());
    let item_key = if raw_id.is_empty() || generated {
        url.clone()
    } else if raw_id.starts_with("http://") || raw_id.starts_with("https://") {
        canonical_url(raw_id).unwrap_or_else(|_| raw_id.to_string())
    } else {
        raw_id.to_string()
    };
    let title = entry.title.as_ref().map(|t| excerpt(&t.content, 300)).unwrap_or_default();
    let author = entry.authors.iter().map(|p| p.name.as_str()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ");
    let raw_content = entry.content.as_ref().and_then(|c| c.body.clone()).unwrap_or_default();
    let raw_summary = entry.summary.as_ref().map(|t| t.content.clone()).unwrap_or_default();
    let content_html = if raw_content.is_empty() { String::new() } else { sanitize(&raw_content) };
    let summary_source = if raw_summary.is_empty() { content_html.clone() } else { raw_summary };
    let summary = excerpt(&summary_source, 280);
    let content_hash = if content_html.is_empty() {
        None
    } else {
        let digest = Sha256::digest(content_html.as_bytes());
        Some(digest.iter().map(|b| format!("{b:02x}")).collect())
    };
    let published_at = entry.published.or(entry.updated).map(|t| t.timestamp());
    let updated_at = entry.updated.map(|t| t.timestamp());
    let image_url = image(entry);
    Some(ParsedItem {
        item_key,
        url,
        original_url: original,
        title,
        author,
        summary,
        content_html,
        content_hash,
        published_at,
        updated_at,
        image_url,
    })
}

fn entry_url(entry: &Entry) -> Option<String> {
    let link = entry.links.iter().find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate")).or_else(|| entry.links.first());
    link.map(|l| l.href.clone()).or_else(|| {
        if entry.id.starts_with("http://") || entry.id.starts_with("https://") { Some(entry.id.clone()) } else { None }
    })
}

fn image(entry: &Entry) -> Option<String> {
    let mut candidates: Vec<(u32, String)> = Vec::new();

    for media in &entry.media {
        for content in &media.content {
            if let Some(url) = &content.url {
                let text = url.as_str();
                let is_video = content.content_type.as_ref().is_some_and(|m| {
                    let s = m.to_string();
                    s.starts_with("video/") || s.starts_with("audio/")
                }) || text.contains("/video/") || text.ends_with(".mp4") || text.ends_with(".webm");
                if is_video {
                    continue;
                }
                let is_img = content.content_type.as_ref().is_none_or(|m| m.to_string().starts_with("image/"))
                    || url_looks_like_image(text);
                if (text.starts_with("https://") || text.starts_with("http://")) && is_img {
                    let w = content.width.unwrap_or(0);
                    let h = content.height.unwrap_or(0);
                    let dim = w.max(h);
                    candidates.push((dim, upgrade_image_url(text)));
                }
            }
        }
        for thumb in &media.thumbnails {
            let text = &thumb.image.uri;
            if text.starts_with("https://") || text.starts_with("http://") {
                let w = thumb.image.width.unwrap_or(0);
                let h = thumb.image.height.unwrap_or(0);
                let dim = w.max(h);
                candidates.push((dim, upgrade_image_url(text)));
            }
        }
    }

    for link in &entry.links {
        if let Some(url) = image_link(link) {
            candidates.push((0, upgrade_image_url(&url)));
        }
    }

    if candidates.is_empty() {
        if let Some(content) = entry.content.as_ref().and_then(|c| c.body.as_ref()) {
            if let Some(url) = extract_img_src(content) {
                candidates.push((0, upgrade_image_url(&url)));
            }
        }
    }

    if candidates.is_empty() {
        if let Some(summary) = &entry.summary {
            if let Some(url) = extract_img_src(&summary.content) {
                candidates.push((0, upgrade_image_url(&url)));
            }
        }
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.into_iter().next().map(|(_, url)| url)
}

fn image_link(link: &Link) -> Option<String> {
    let href = link.href.trim();
    if !href.starts_with("https://") && !href.starts_with("http://") {
        return None;
    }
    let is_img = link.media_type.as_ref().is_some_and(|m| m.starts_with("image/"))
        || link.rel.as_deref() == Some("enclosure")
        || url_looks_like_image(href);
    if is_img {
        Some(href.to_string())
    } else {
        None
    }
}

fn url_looks_like_image(url: &str) -> bool {
    let clean = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    clean.ends_with(".jpg")
        || clean.ends_with(".jpeg")
        || clean.ends_with(".png")
        || clean.ends_with(".webp")
        || clean.ends_with(".avif")
        || clean.ends_with(".gif")
}

fn extract_img_src(html: &str) -> Option<String> {
    let doc = scraper::Html::parse_fragment(html);
    let sel = scraper::Selector::parse("img").ok()?;
    for img in doc.select(&sel) {
        if let Some(src) = img.value().attr("src") {
            let src = src.trim();
            if (src.starts_with("https://") || src.starts_with("http://"))
                && !src.contains("tracking")
                && !src.contains("pixel")
                && !src.contains("1x1")
                && !src.contains("feedburner")
                && !src.contains("statcounter")
            {
                return Some(src.to_string());
            }
        }
    }
    None
}

pub fn upgrade_image_url(url: &str) -> String {
    let mut s = url.to_string();
    if s.starts_with("http://") {
        s = format!("https://{}", &s[7..]);
    }
    // BBC: upgrade 240, 144, 320, 480, 640 to 1024
    if s.contains("ichef.bbci.co.uk") {
        for small in ["/240/", "/144/", "/320/", "/480/", "/640/"] {
            if s.contains(small) {
                return s.replace(small, "/1024/");
            }
        }
    }
    // CBS News: remove /thumbnail/.../.../
    if s.contains("cbsnewsstatic.com") && s.contains("/thumbnail/") {
        if let Some(idx) = s.find("/thumbnail/") {
            let rest = &s[idx + 11..];
            if let Some(slash1) = rest.find('/') {
                let rest2 = &rest[slash1 + 1..];
                if let Some(slash2) = rest2.find('/') {
                    return format!("{}{}", &s[..idx], &rest2[slash2..]);
                }
            }
        }
    }
    // NYT: upgrade mediumSquareAt3X, thumbStandard, articleLarge to superJumbo
    if s.contains("static01.nyt.com") {
        for small in ["-mediumSquareAt3X.", "-thumbStandard.", "-articleLarge."] {
            if s.contains(small) {
                return s.replace(small, "-superJumbo.");
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>Example</title>
  <link>https://example.com</link>
  <item>
    <title>Rust release</title>
    <link>https://example.com/rust?utm_source=rss</link>
    <guid>rust-1</guid>
    <pubDate>Tue, 14 Oct 2025 12:00:00 GMT</pubDate>
    <author>Ada</author>
    <description><![CDATA[<p>Hello <script>alert(1)</script>world</p>]]></description>
  </item>
  <item>
    <title>No guid</title>
    <link>https://example.com/no-guid</link>
    <description>Plain</description>
  </item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom</title>
  <link href="https://atom.example/"/>
  <updated>2025-10-14T12:00:00Z</updated>
  <entry>
    <id>tag:atom.example,2025:1</id>
    <title>Atom item</title>
    <link href="https://atom.example/1"/>
    <updated>2025-10-14T12:00:00Z</updated>
    <summary>Short</summary>
  </entry>
</feed>"#;

    const JSON: &str = r#"{
      "version": "https://jsonfeed.org/version/1",
      "title": "JSON",
      "home_page_url": "https://json.example/",
      "items": [{
        "id": "json-1",
        "url": "https://json.example/1",
        "title": "JSON item",
        "content_text": "Hello from JSON",
        "date_published": "2025-10-14T12:00:00Z"
      }]
    }"#;

    #[test]
    fn rss_atom_and_json_feed_parse() {
        let rss = parse_feed(RSS.as_bytes()).unwrap();
        assert_eq!(rss.title, "Example");
        assert_eq!(rss.items.len(), 2);
        assert_eq!(rss.items[0].item_key, "rust-1");
        assert_eq!(rss.items[0].url, "https://example.com/rust");
        assert!(!rss.items[0].summary.to_ascii_lowercase().contains("script"));
        assert!(rss.items[0].summary.contains("Hello"));
        assert_eq!(rss.items[1].item_key, "https://example.com/no-guid");

        let atom = parse_feed(ATOM.as_bytes()).unwrap();
        assert_eq!(atom.items[0].title, "Atom item");
        assert_eq!(atom.items[0].item_key, "tag:atom.example,2025:1");

        let json = parse_feed(JSON.as_bytes()).unwrap();
        assert_eq!(json.title, "JSON");
        assert_eq!(json.items[0].summary, "Hello from JSON");
    }

    #[test]
    fn malformed_and_oversized_feeds_fail() {
        assert!(parse_feed(b"<rss").is_err());
        assert!(parse_feed(b"{").is_err());
        let big = vec![b'x'; MAX_FEED_BYTES + 1];
        assert!(parse_feed(&big).is_err());
    }

    #[test]
    fn an_empty_channel_is_a_valid_feed() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Empty</title></channel></rss>"#;
        let feed = parse_feed(xml.as_bytes()).unwrap();
        assert!(feed.items.is_empty());
        assert_eq!(feed.title, "Empty");
    }
}
