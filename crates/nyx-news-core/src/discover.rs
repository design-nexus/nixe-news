//! Bundled publication directory and HTML feed autodiscovery.
//!
//! Browsing the directory does not open a socket. Finding a feed from a site
//! address is a separate fetch of that page's `<link rel="alternate">` tags.

use crate::normalize::canonical_url;
use crate::opml::{self, Outline};
use anyhow::Result;
use url::Url;

const DIRECTORY: &str = include_str!("../directory.opml");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredFeed {
    pub title: String,
    pub url: String,
}

/// Topics and feeds shipped with the app.
pub fn directory() -> Result<Vec<Outline>> {
    opml::import(DIRECTORY)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectorySource {
    pub section: String,
    pub title: String,
    pub url: String,
}

/// Publications in the bundled directory, in section order.
pub fn directory_sources() -> Vec<DirectorySource> {
    let Ok(topics) = directory() else { return Vec::new() };
    let mut sources = Vec::new();
    for topic in topics {
        for feed in topic.children {
            let Some(url) = feed.feed_url else { continue };
            sources.push(DirectorySource { section: topic.title.clone(), title: feed.title, url });
        }
    }
    sources
}

/// Feed links declared on an HTML page. `page_url` resolves relative hrefs.
pub fn discover_in_html(html: &str, page_url: &str) -> Result<Vec<DiscoveredFeed>> {
    let base = Url::parse(page_url.trim())?;
    let document = scraper::Html::parse_document(html);
    let selector = scraper::Selector::parse("link").map_err(|_| anyhow::anyhow!("the page could not be scanned"))?;
    let mut found = Vec::new();
    for element in document.select(&selector) {
        let rel = element.value().attr("rel").unwrap_or("");
        if !rel.split_whitespace().any(|part| part.eq_ignore_ascii_case("alternate")) {
            continue;
        }
        let kind = element.value().attr("type").unwrap_or("").split(';').next().unwrap_or("").trim();
        if !is_feed_type(kind) {
            continue;
        }
        let Some(href) = element.value().attr("href") else { continue };
        let Ok(joined) = base.join(href.trim()) else { continue };
        let Ok(url) = canonical_url(joined.as_str()) else { continue };
        if found.iter().any(|feed: &DiscoveredFeed| feed.url == url) {
            continue;
        }
        let title = element.value().attr("title").unwrap_or("").trim();
        let title = if title.is_empty() { url.clone() } else { title.to_string() };
        found.push(DiscoveredFeed { title, url });
    }
    Ok(found)
}

fn is_feed_type(kind: &str) -> bool {
    matches!(
        kind,
        "application/rss+xml" | "application/atom+xml" | "application/feed+json" | "application/rdf+xml" | "application/json"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_parses_without_a_network_call() {
        let topics = directory().unwrap();
        assert!(topics.iter().any(|topic| topic.title == "World"));
        let urls: Vec<&str> =
            topics.iter().flat_map(|topic| topic.children.iter()).filter_map(|feed| feed.feed_url.as_deref()).collect();
        assert!(urls.contains(&"https://feeds.bbci.co.uk/news/world/rss.xml"));
        assert!(urls.iter().all(|url| url.starts_with("https://")));
        let sources = directory_sources();
        assert!(sources.len() >= 40);
        assert_eq!(sources[0].section, "World");
        for section in ["World", "National", "Politics", "Business", "Technology", "Science", "Health", "Sports", "Culture"] {
            assert!(sources.iter().any(|source| source.section == section), "{section}");
        }
        assert!(sources.iter().any(|source| source.title == "ESPN"));
        assert!(sources.iter().all(|source| source.section != "Linux"));
    }

    #[test]
    fn alternate_feed_links_are_collected() {
        let html = r#"<html><head>
            <link rel="stylesheet" href="/app.css">
            <link rel="alternate" type="application/rss+xml" title="Example" href="/feed.xml">
            <link rel="alternate" type="text/html" href="/story">
            <link rel="alternate" type="application/atom+xml" href="https://other.example/atom.xml">
        </head></html>"#;
        let feeds = discover_in_html(html, "https://news.example/section/").unwrap();
        assert_eq!(
            feeds,
            vec![
                DiscoveredFeed { title: "Example".into(), url: "https://news.example/feed.xml".into() },
                DiscoveredFeed { title: "https://other.example/atom.xml".into(), url: "https://other.example/atom.xml".into() },
            ]
        );
    }
}
