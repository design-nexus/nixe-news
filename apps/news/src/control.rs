//! Tools Nixe Assistant can call while News is open.

use crate::note;
use crate::prefs;
use crate::window;
use crate::worker;
use nixe_core::control::{Reply, text};
use nixe_news_core::View;
use serde_json::{Value, json};

pub fn tools() -> Value {
    json!([
        {"name": "news.search", "description": "Search subscribed articles by title and summary.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}},
        {"name": "news.headlines", "description": "Unread headlines, optionally from one feed id.", "inputSchema": {"type": "object", "properties": {"feed": {"type": "string"}}}},
        {"name": "news.add_feed", "description": "Subscribe to an RSS, Atom, or JSON feed. Refused unless Assistant subscriptions are turned on in News settings.", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}},
        {"name": "news.mark_read", "description": "Mark one article read by its id.", "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}},
        {"name": "news.export_markdown", "description": "Write one article as a Markdown file inside the home folder. This does not write Notes.", "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "path": {"type": "string"}, "overwrite": {"type": "boolean"}}, "required": ["id", "path"]}}
    ])
}

pub fn handle(name: &str, args: Value, reply: Reply) {
    let result = match name {
        "news.search" => search(&args),
        "news.headlines" => headlines(&args),
        "news.add_feed" => add_feed(&args),
        "news.mark_read" => mark_read(&args),
        "news.export_markdown" => export_markdown(&args),
        other => Err(format!("unknown tool {other}")),
    };
    reply(result);
}

fn search(args: &Value) -> Result<Value, String> {
    let query = text(args, "query")?;
    window::with_db(|db| db.articles(&View::All, &query, 20, 0)).map(list).map_err(|e| e.to_string())
}

fn headlines(args: &Value) -> Result<Value, String> {
    let view = match args["feed"].as_str() {
        Some(id) if !id.is_empty() => View::Feed(id.to_string()),
        _ => View::Unread,
    };
    window::with_db(|db| db.articles(&view, "", 20, 0)).map(list).map_err(|e| e.to_string())
}

fn list(items: Vec<nixe_news_core::ArticleListItem>) -> Value {
    json!(
        items
            .into_iter()
            .map(|a| json!({
                "id": a.id,
                "title": a.title,
                "feed": a.feed_title,
                "url": a.original_url,
                "unread": !a.read
            }))
            .collect::<Vec<_>>()
    )
}

fn add_feed(args: &Value) -> Result<Value, String> {
    if !prefs::get().assistant_can_subscribe {
        return Err("Assistant can't add feeds until you turn that on in News settings.".into());
    }
    let url = text(args, "url")?;
    worker::add_url(url.clone());
    Ok(json!({"started": true, "url": url}))
}

fn export_markdown(args: &Value) -> Result<Value, String> {
    let id = text(args, "id")?;
    let path = nixe_core::control::save_target(args)?;
    let article =
        window::with_db(|db| db.article(&id)).map_err(|err| err.to_string())?.ok_or("that article is not in the library")?;
    let markdown = note::markdown(&article);
    std::fs::write(&path, markdown).map_err(|err| err.to_string())?;
    Ok(json!({"path": path.display().to_string()}))
}

fn mark_read(args: &Value) -> Result<Value, String> {
    let id = text(args, "id")?;
    window::with_db(|db| db.set_read(&id, true)).map_err(|e| e.to_string())?;
    window::reload();
    Ok(json!({"id": id, "read": true}))
}
