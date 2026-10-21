//! Glimpse plugin. It opens `news.sqlite` read-only and does not fetch feeds.

use anyhow::{Result, bail};
use glimpse_plugin_sdk::{Action, Hit, Request, serve};
use nixe_news_core::{library_query, open_readonly};
use serde_json::{Value, json};
use std::path::PathBuf;

fn main() {
    if let Err(error) = serve(&["search", "preview"], answer) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn library() -> PathBuf {
    if let Some(path) = std::env::var_os("NIXE_NEWS_DB") {
        return PathBuf::from(path);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data.unwrap_or_else(|| PathBuf::from(".local/share")).join("nixe-news/news.sqlite")
}

fn answer(request: &Request) -> Result<Value> {
    match request.op.as_str() {
        "search" => {
            let query = request.args["query"].as_str().unwrap_or("");
            let limit = request.args["limit"].as_i64().unwrap_or(10).clamp(1, 10);
            let asked = library_query(query);
            let db = open_readonly(&library()).map_err(|_| anyhow::anyhow!("News has no library yet."))?;
            let items = db.articles(&asked.view, &asked.text, limit, 0)?;
            let hits = items
                .into_iter()
                .map(|article| Hit {
                    id: article.id,
                    title: article.title,
                    subtitle: article.feed_title,
                    action: Action::Copy {
                        text: if article.original_url.is_empty() { article.url } else { article.original_url },
                    },
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_value(hits)?)
        }
        "preview" => {
            let id = request.args["result_id"].as_str().unwrap_or("");
            if id.is_empty() {
                bail!("missing article");
            }
            let db = open_readonly(&library()).map_err(|_| anyhow::anyhow!("News has no library yet."))?;
            let Some(article) = db.article(id)? else { bail!("that article is not in the library") };
            let mut lines = vec![article.title, article.feed_title];
            if !article.summary.is_empty() {
                lines.push(article.summary);
            }
            Ok(json!({"text": lines.join("\n")}))
        }
        other => bail!("unsupported operation {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixe_news_core::{Ingest, NewFeed, View, open, parse_feed};
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Example</title><link>https://example.com</link>
<item><title>Rust release</title><link>https://example.com/rust</link><guid>g1</guid>
<description>Notes from the feed</description></item>
<item><title>Old post</title><link>https://example.com/old</link><guid>g2</guid>
<description>Saved text</description></item>
</channel></rss>"#;

    #[test]
    fn search_and_preview_read_the_library() {
        let _guard = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("nixe-news-glimpse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("news.sqlite");
        let saved = {
            let mut db = open(&path).unwrap();
            let id = db
                .insert_feed(&NewFeed {
                    url: "https://example.com/feed".into(),
                    title: "Example".into(),
                    site_url: String::new(),
                    folder_id: None,
                })
                .unwrap();
            db.apply_feed(
                &id,
                &parse_feed(RSS.as_bytes()).unwrap(),
                &Ingest { etag: None, last_modified: None, keep_content: false, cache_dir: None },
            )
            .unwrap();
            let items = db.articles(&View::All, "", 10, 0).unwrap();
            let saved = items.iter().find(|article| article.title == "Old post").unwrap().id.clone();
            db.set_read(&saved, true).unwrap();
            db.set_saved(&saved, true).unwrap();
            drop(db);
            saved
        };
        // SAFETY: the test lock is held, and this process is the only reader of the variable.
        unsafe { std::env::set_var("NIXE_NEWS_DB", &path) };

        let hits = answer(&request("search", json!({"query": "news", "limit": 10}))).unwrap();
        assert_eq!(hits.as_array().unwrap().len(), 2);
        let unread = answer(&request("search", json!({"query": "news unread"}))).unwrap();
        assert_eq!(unread.as_array().unwrap().len(), 1);
        let saved_hits = answer(&request("search", json!({"query": "news saved"}))).unwrap();
        assert_eq!(saved_hits[0]["id"], saved);
        let preview = answer(&request("preview", json!({"result_id": saved}))).unwrap();
        let text = preview["text"].as_str().unwrap();
        assert!(text.contains("Old post"));
        assert!(text.contains("Example"));
        assert!(text.contains("Saved text"));

        unsafe { std::env::remove_var("NIXE_NEWS_DB") };
        let _ = std::fs::remove_dir_all(dir);
    }

    fn request(op: &str, args: Value) -> Request {
        Request { api_version: 1, id: 1, op: op.into(), args }
    }
}
