//! Pull the article text out of a page the reader opened.
//!
//! This is the visible article, not a copy of a paywall and not a background crawl.

use scraper::{Html, Selector};
use sha2::{Digest, Sha256};

pub fn cache_key(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Paragraphs from the page the reader opened. Empty when the page has no article text.
pub fn article_text(html: &str) -> String {
    let from_json = json_article(html);
    let document = Html::parse_document(html);
    let mut text = paragraphs(&document, "article p");
    if text.len() < 400 {
        for sel in [
            "main p",
            "div[itemprop=\"articleBody\"] p",
            "div[class*=\"article-body\"] p",
            "div[class*=\"story-body\"] p",
            "div[class*=\"article__body\"] p",
            "div[class*=\"post-content\"] p",
            "div[class*=\"entry-content\"] p",
            "div[class*=\"article-content\"] p",
            "div[data-component=\"text-block\"]",
        ] {
            let candidate = paragraphs(&document, sel);
            if candidate.len() > text.len() {
                text = candidate;
                if text.len() >= 600 {
                    break;
                }
            }
        }
    }
    if text.is_empty() {
        text = paragraphs(&document, "p");
    }
    if from_json.len() > text.len() { from_json } else { text }
}

/// Returns true if the publisher is known to require reading on their website
/// (e.g. strict paywall, anti-scraping WAF, or video/audio-only streams).
pub fn requires_publisher_site(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("nytimes.com")
        || lower.contains("wsj.com")
        || lower.contains("bloomberg.com")
        || lower.contains("ft.com")
        || lower.contains("thetimes.co.uk")
        || lower.contains("barrons.com")
        || lower.contains("economist.com")
        || lower.contains("theatlantic.com")
        || lower.contains("newyorker.com")
        || lower.contains("nature.com")
        || lower.contains("science.org")
        || lower.contains("statnews.com")
        || lower.contains("bbc.co.uk/sounds/")
        || lower.contains("youtube.com")
        || lower.contains("vimeo.com")
}

fn json_article(html: &str) -> String {
    let key = "\"articleBody\"";
    let mut best = String::new();
    let mut rest = html;
    while let Some(at) = rest.find(key) {
        let after = rest[at + key.len()..].trim_start();
        if let Some(value) = after.strip_prefix(':').and_then(|value| json_string(value.trim_start())) {
            let text = plain_html(&value);
            if text.len() > best.len() {
                best = text;
            }
        }
        rest = &rest[at + key.len()..];
    }
    best
}

fn json_string(input: &str) -> Option<String> {
    let mut chars = input.chars();
    if chars.next() != Some('"') {
        return None;
    }
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16)
                        && let Some(decoded) = char::from_u32(code)
                    {
                        out.push(decoded);
                    }
                }
                other => out.push(other),
            },
            _ => out.push(ch),
        }
    }
    None
}

fn plain_html(input: &str) -> String {
    let broken = input.replace("</p>", "\n\n").replace("<br>", "\n\n").replace("<br/>", "\n\n").replace("<br />", "\n\n");
    let mut stripped = String::new();
    let mut in_tag = false;
    for ch in broken.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for block in stripped.split("\n\n") {
        let decoded = crate::html::decode_entities(block);
        let line = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.len() < 40 || junk(&line) || out.last().is_some_and(|prev: &String| prev == &line) {
            continue;
        }
        out.push(line);
        if out.len() == 40 {
            break;
        }
    }
    if out.is_empty() {
        let decoded = crate::html::decode_entities(&stripped);
        let line = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.len() >= 400 && !junk(&line) { line } else { String::new() }
    } else {
        out.join("\n\n")
    }
}

fn paragraphs(document: &Html, selector: &str) -> String {
    let Ok(selector) = Selector::parse(selector) else { return String::new() };
    let mut out = Vec::new();
    for node in document.select(&selector) {
        let raw = node.text().collect::<String>();
        let decoded = crate::html::decode_entities(&raw);
        let line = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.len() < 25 || junk(&line) {
            continue;
        }
        if out.last().is_some_and(|prev: &String| prev == &line) {
            continue;
        }
        out.push(line);
        if out.len() == 60 {
            break;
        }
    }
    out.join("\n\n")
}

fn junk(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("web browser is not fully supported")
        || line.contains("please enable js")
        || line.contains("disable any ad blocker")
}

/// Extract OpenGraph / Twitter card image from article HTML.
pub fn extract_og_image(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let sel = Selector::parse("meta[property='og:image'], meta[name='twitter:image'], meta[property='twitter:image'], meta[name='thumbnail']").ok()?;
    for meta in document.select(&sel) {
        if let Some(content) = meta.value().attr("content") {
            let content = content.trim();
            if (content.starts_with("https://") || content.starts_with("http://"))
                && !content.contains("tracking")
                && !content.contains("pixel")
                && !content.contains("1x1")
            {
                return Some(crate::parse::upgrade_image_url(content));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_article_body_is_kept_and_the_chrome_is_not() {
        let html = r#"<html><body>
            <nav><p>Sign in to continue reading this short line.</p></nav>
            <article>
              <p>The city opened the new bridge at dawn, and the first trains crossed before the speeches ended.</p>
              <p>Engineers said the span replaces a ferry that had run since the nineteen thirties.</p>
            </article>
            <footer><p>Copyright notice that is long enough to look like a paragraph but lives in the footer.</p></footer>
        </body></html>"#;
        let text = article_text(html);
        assert!(text.contains("new bridge"));
        assert!(text.contains("nineteen thirties"));
        assert!(!text.contains("Sign in"));
    }

    #[test]
    fn json_article_body_is_used_when_it_is_the_story() {
        let html = r#"<html><script type="application/ld+json">{"@type":"NewsArticle","articleBody":"The council voted to rebuild the harbor wall before the next storm season arrives.\n\nEngineers said the old stones had shifted after a decade of high water."}</script><p>Please enable JS and disable any ad blocker</p></html>"#;
        let text = article_text(html);
        assert!(text.contains("harbor wall"));
        assert!(text.contains("high water"));
        assert!(!text.contains("ad blocker"));
    }
}
