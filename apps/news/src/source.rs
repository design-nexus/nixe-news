//! Publication name and date for a story. The address stays off the page.

use gtk::glib;

pub fn byline(url: &str, feed: &str, published_at: Option<i64>) -> String {
    let name = name(url, feed);
    let when = when(published_at);
    if when.is_empty() { name } else { format!("{name} · {when}") }
}

pub fn name(url: &str, feed: &str) -> String {
    let host = url.split("//").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let known = if host.contains("nytimes.com") {
        "The New York Times"
    } else if host.contains("bbc.") || host.contains("bbci.co.uk") {
        "BBC"
    } else if host.contains("theguardian.com") {
        "The Guardian"
    } else if host.contains("npr.org") {
        "NPR"
    } else if host.contains("espn.com") {
        "ESPN"
    } else if host.contains("aljazeera.com") {
        "Al Jazeera"
    } else if host.contains("nbcnews.com") {
        "NBC News"
    } else if host.contains("cbsnews.com") {
        "CBS News"
    } else if host.contains("abcnews.go.com") {
        "ABC News"
    } else if host.contains("wired.com") {
        "Wired"
    } else if host.contains("theverge.com") {
        "The Verge"
    } else if host.contains("arstechnica.com") {
        "Ars Technica"
    } else if host.contains("techcrunch.com") {
        "TechCrunch"
    } else if host.contains("technologyreview.com") {
        "MIT Technology Review"
    } else if host.contains("nature.com") {
        "Nature"
    } else if host.contains("science.org") {
        "Science"
    } else if host.contains("scientificamerican.com") {
        "Scientific American"
    } else if host.contains("newscientist.com") {
        "New Scientist"
    } else if host.contains("nasa.gov") {
        "NASA"
    } else if host.contains("statnews.com") {
        "STAT"
    } else if host.contains("variety.com") {
        "Variety"
    } else if host.contains("pitchfork.com") {
        "Pitchfork"
    } else if host.contains("bloomberg.com") {
        "Bloomberg"
    } else if host.contains("cnbc.com") {
        "CNBC"
    } else if host.contains("wsj.com") || host.contains("dj.com") {
        "The Wall Street Journal"
    } else if host.contains("thehill.com") {
        "The Hill"
    } else {
        ""
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let feed = feed.trim();
    let feed = feed.split_once('|').map(|(_, rest)| rest.trim()).unwrap_or(feed);
    let feed = feed.split_once(" - ").map(|(head, _)| head.trim()).unwrap_or(feed);
    let feed = feed.strip_prefix("RSS:").map(str::trim).unwrap_or(feed);
    if feed.is_empty() { "News".into() } else { feed.to_string() }
}

pub fn when(published_at: Option<i64>) -> String {
    let Some(ts) = published_at else { return String::new() };
    let now = glib::DateTime::now_local().ok().map(|d| d.to_unix()).unwrap_or(0);
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        return "Just now".to_string();
    }
    if diff < 3600 {
        let mins = diff / 60;
        return format!("{mins}m ago");
    }
    if diff < 86400 {
        let hours = diff / 3600;
        return format!("{hours}h ago");
    }
    if diff < 172800 {
        return "Yesterday".to_string();
    }
    glib::DateTime::from_unix_local(ts)
        .ok()
        .and_then(|d| d.format("%b %-e").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn read_time(text: &str) -> String {
    let words = text.split_whitespace().count();
    let mins = (words / 200).max(1);
    format!("{mins} min read")
}

pub fn masthead_date() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%A, %B %-e").ok())
        .map(|s| s.to_string().to_uppercase())
        .unwrap_or_else(|| "TODAY'S EDITION".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_comes_from_the_address() {
        assert_eq!(name("https://www.nytimes.com/2026/10/18/world/story.html", "NYT > World News"), "The New York Times");
        assert_eq!(name("https://www.bbc.co.uk/news/articles/abc", "BBC News"), "BBC");
        assert_eq!(name("https://www.theguardian.com/world/story", "World news | The Guardian"), "The Guardian");
    }
}
