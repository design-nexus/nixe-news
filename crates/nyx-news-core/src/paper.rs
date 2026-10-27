//! The front page: sections, one lead story, then a few more headlines.

use crate::cluster::same_story;
use crate::model::ArticleListItem;
use std::collections::{HashMap, HashSet};

pub const SECTION_ORDER: &[&str] =
    &["World", "National", "Politics", "Business", "Technology", "Science", "Health", "Sports", "Culture"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edition {
    pub name: String,
    pub stories: Vec<ArticleListItem>,
}

/// Group stories by section. The first story in each section is the lead.
/// `sections` is `(feed id, section name)`.
pub fn lay_paper(articles: &[ArticleListItem], sections: &[(String, String)]) -> Vec<Edition> {
    let by_feed: HashMap<&str, &str> = sections.iter().map(|(id, name)| (id.as_str(), name.as_str())).collect();
    let mut names: Vec<String> = SECTION_ORDER.iter().map(|name| (*name).to_string()).collect();
    let mut buckets: Vec<Vec<ArticleListItem>> = names.iter().map(|_| Vec::new()).collect();
    let mut other = Vec::new();

    let mut seen_urls: HashSet<String> = HashSet::new();
    let mut seen_titles: Vec<(String, Option<i64>)> = Vec::new();

    for article in articles {
        let clean_url = if !article.url.is_empty() {
            crate::normalize::canonical_url(&article.url).unwrap_or_else(|_| article.url.clone())
        } else if !article.original_url.is_empty() {
            crate::normalize::canonical_url(&article.original_url).unwrap_or_else(|_| article.original_url.clone())
        } else {
            String::new()
        };

        if !clean_url.is_empty() && seen_urls.contains(&clean_url) {
            continue;
        }

        if !article.title.trim().is_empty()
            && seen_titles.iter().any(|(t, when)| same_story(&article.title, article.published_at, t, *when))
        {
            continue;
        }

        if !clean_url.is_empty() {
            seen_urls.insert(clean_url);
        }
        seen_titles.push((article.title.clone(), article.published_at));

        let name = by_feed.get(article.feed_id.as_str()).copied().unwrap_or("Other");
        if name == "Other" {
            other.push(article.clone());
            continue;
        }
        if !names.iter().any(|have| have == name) {
            names.push(name.to_string());
            buckets.push(Vec::new());
        }
        let index = names.iter().position(|have| have == name).unwrap();
        buckets[index].push(article.clone());
    }
    let mut edition: Vec<Edition> = names
        .into_iter()
        .zip(buckets)
        .filter(|(_, stories)| !stories.is_empty())
        .map(|(name, stories)| Edition { name, stories })
        .collect();
    if !other.is_empty() {
        edition.push(Edition { name: "Other".into(), stories: other });
    }
    edition
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story(id: &str, feed: &str, title: &str) -> ArticleListItem {
        ArticleListItem {
            id: id.into(),
            feed_id: feed.into(),
            feed_title: feed.into(),
            title: title.into(),
            author: String::new(),
            summary: format!("{title} summary"),
            url: String::new(),
            original_url: String::new(),
            published_at: None,
            image_url: None,
            read: false,
            starred: false,
            saved: false,
            content_hash: None,
            why: String::new(),
        }
    }

    #[test]
    fn the_paper_keeps_section_order() {
        let mut articles = Vec::new();
        for n in 0..6 {
            articles.push(story(&format!("w{n}"), "bbc", &format!("World {n}")));
        }
        articles.push(story("t", "ars", "Chip"));
        articles.push(story("x", "blog", "Diary"));
        let sections = vec![("bbc".into(), "World".into()), ("ars".into(), "Technology".into())];
        let paper = lay_paper(&articles, &sections);
        assert_eq!(paper[0].name, "World");
        assert_eq!(paper[0].stories.len(), 6);
        assert_eq!(paper[0].stories[0].title, "World 0");
        assert_eq!(paper[1].name, "Technology");
        assert_eq!(paper[2].name, "Other");
        assert!(paper.iter().all(|section| section.name != "Science" && section.name != "Health"));
    }
}
