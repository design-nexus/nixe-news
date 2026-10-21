//! Markdown for one article. The file is whatever path the user picks.

use crate::html_text;
use crate::paths;
use crate::window;
use nixe_news_core::{ArticleListItem, article_markdown};

pub fn markdown(article: &ArticleListItem) -> String {
    let cached = article
        .content_hash
        .as_ref()
        .and_then(|hash| window::with_db(|db| Ok(db.body(&paths::content_cache(), hash))).ok().flatten());
    let body =
        cached.map(|html| html_text::plain(&html)).filter(|text| !text.is_empty()).unwrap_or_else(|| article.summary.clone());
    article_markdown(&article.title, &article.feed_title, &article.author, &article.original_url, &body)
}

pub fn filename(title: &str) -> String {
    let mut name = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(ch);
        } else if (ch.is_whitespace() || ch == '-') && !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-');
    let name = if name.is_empty() { "article" } else { name };
    let name = name.chars().take(80).collect::<String>();
    format!("{name}.md")
}
