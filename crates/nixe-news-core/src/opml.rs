//! OPML import and export. Import matches feeds by canonical URL.

use crate::normalize::canonical_url;
use anyhow::{Context, Result};
use roxmltree::{Document, Node};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outline {
    pub title: String,
    pub feed_url: Option<String>,
    pub site_url: Option<String>,
    pub children: Vec<Outline>,
}

pub fn parse(text: &str) -> Result<Vec<Outline>> {
    let doc = Document::parse(text).context("the OPML could not be parsed")?;
    let body = doc.descendants().find(|n| n.has_tag_name("body")).context("the OPML has no body")?;
    Ok(body.children().filter(|n| n.has_tag_name("outline")).map(outline).collect())
}

fn outline(node: Node<'_, '_>) -> Outline {
    let title = attr(&node, "text").or_else(|| attr(&node, "title")).unwrap_or_default();
    let feed_url = attr(&node, "xmlUrl").and_then(|u| canonical_url(&u).ok());
    let site_url = attr(&node, "htmlUrl");
    let children = node.children().filter(|n| n.has_tag_name("outline")).map(outline).collect();
    Outline { title, feed_url, site_url, children }
}

fn attr(node: &Node<'_, '_>, name: &str) -> Option<String> {
    node.attribute(name).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn write(outlines: &[Outline]) -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head><title>Nixe News</title></head>\n  <body>\n",
    );
    for outline in outlines {
        write_outline(&mut body, outline, 4);
    }
    body.push_str("  </body>\n</opml>\n");
    body
}

fn write_outline(body: &mut String, outline: &Outline, indent: usize) {
    let pad = " ".repeat(indent);
    body.push_str(&pad);
    body.push_str("<outline");
    body.push_str(&format!(" text=\"{}\"", escape(&outline.title)));
    if let Some(url) = &outline.feed_url {
        body.push_str(&format!(" type=\"rss\" xmlUrl=\"{}\"", escape(url)));
    }
    if let Some(site) = &outline.site_url {
        body.push_str(&format!(" htmlUrl=\"{}\"", escape(site)));
    }
    if outline.children.is_empty() {
        body.push_str("/>\n");
        return;
    }
    body.push_str(">\n");
    for child in &outline.children {
        write_outline(body, child, indent + 2);
    }
    body.push_str(&pad);
    body.push_str("</outline>\n");
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Re-export names the store uses. Parsing lives here so the database module
/// can stay about rows.
pub fn import(text: &str) -> Result<Vec<Outline>> {
    parse(text)
}

pub fn export(outlines: &[Outline]) -> String {
    write(outlines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_folders_and_feed_urls() {
        let text = r#"<?xml version="1.0"?><opml version="2.0"><body>
          <outline text="Tech">
            <outline text="Example" type="rss" xmlUrl="https://example.com/feed.xml" htmlUrl="https://example.com/"/>
          </outline>
        </body></opml>"#;
        let outlines = parse(text).unwrap();
        assert_eq!(outlines[0].title, "Tech");
        assert_eq!(outlines[0].children[0].feed_url.as_deref(), Some("https://example.com/feed.xml"));
        let again = parse(&write(&outlines)).unwrap();
        assert_eq!(again, outlines);
    }
}
