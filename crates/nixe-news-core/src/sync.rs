//! Apply a merged sync state to SQLite, and move one device's journal to a remote.
//!
//! Article HTML and summaries are not part of the change log, so they are not
//! uploaded. A remote that rejects the password is contacted before any write.

use crate::store::Db;
use anyhow::Result;
use nixe_sync::{Hlc, Op, OpKind, Record, Remote, State, sync_exchange};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeMap;

const PREFERENCE_FIELDS: &[&str] =
    &["retention_days", "layout", "poll_minutes", "mark_read_on_open", "recommendations", "summaries"];

pub fn synchronize(db: &mut Db, remote: &impl Remote, preferences: &[(&str, &str)]) -> Result<BTreeMap<String, String>> {
    // A refused password must leave the library untouched, so this read happens first.
    let _ = remote.fetch_manifest()?;
    let mut fields = BTreeMap::new();
    for (key, value) in preferences {
        if PREFERENCE_FIELDS.contains(key) {
            fields.insert((*key).to_string(), (*value).to_string());
        }
    }
    db.note_preferences(&fields)?;
    let (max_seq, pending) = db.pending_ops()?;
    let have = db.generation()?;
    let device = db.device_id()?;
    let exchange = sync_exchange(remote, &device, have, &pending)?;
    let applied = db.apply_exchange(&exchange.ops, exchange.generation, false)?;
    if exchange.pushed {
        db.mark_pushed(max_seq, exchange.generation)?;
    }
    Ok(applied)
}

pub(crate) fn op_name(kind: &OpKind) -> &'static str {
    match kind {
        OpKind::Upsert => "upsert",
        OpKind::Tombstone => "tombstone",
        OpKind::SetAdd => "set_add",
        OpKind::SetRemove => "set_remove",
    }
}

pub(crate) fn op_from_row(hlc: &str, collection: &str, record_id: &str, op: &str, fields: &str) -> Result<Op> {
    let hlc: Hlc = serde_json::from_str(hlc)?;
    let kind = match op {
        "upsert" => OpKind::Upsert,
        "tombstone" => OpKind::Tombstone,
        "set_add" => OpKind::SetAdd,
        "set_remove" => OpKind::SetRemove,
        other => anyhow::bail!("unknown sync op {other}"),
    };
    let value: serde_json::Value = serde_json::from_str(fields).unwrap_or(serde_json::Value::Null);
    Ok(Op {
        hlc,
        collection: collection.to_string(),
        record_id: record_id.to_string(),
        kind,
        fields: flatten(&value),
        member: None,
    })
}

pub(crate) fn preference_fields(state: &State) -> BTreeMap<String, String> {
    let Some(record) = state.get(&("preferences".to_string(), "app".to_string())) else {
        return BTreeMap::new();
    };
    record
        .fields
        .iter()
        .filter(|(key, _)| PREFERENCE_FIELDS.contains(&key.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Write the merged records. This does not append to the change log.
pub(crate) fn project(conn: &rusqlite::Connection, state: &State) -> Result<()> {
    project_folders(conn, state)?;
    project_feeds(conn, state)?;
    project_article_state(conn, state)?;
    project_named(conn, state)?;
    Ok(())
}

fn flatten(value: &serde_json::Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(obj) = value.as_object() else { return out };
    for (key, value) in obj {
        let text = match value {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Bool(flag) => flag.to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => continue,
        };
        out.insert(key.clone(), text);
    }
    out
}

fn field<'a>(record: &'a Record, name: &str) -> Option<&'a str> {
    record.fields.get(name).map(String::as_str)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn project_folders(conn: &rusqlite::Connection, state: &State) -> Result<()> {
    let folders: Vec<(&String, &Record)> =
        state.iter().filter(|(key, _)| key.0 == "folders").map(|(key, record)| (&key.1, record)).collect();
    let mut pending = folders.clone();
    let mut guard = pending.len().saturating_add(1);
    while !pending.is_empty() && guard > 0 {
        guard -= 1;
        let batch = std::mem::take(&mut pending);
        for (id, record) in batch {
            let parent = field(record, "parent_id").filter(|value| !value.is_empty());
            let parent_ready = match parent {
                Some(parent) => row_exists(conn, "folders", parent)? || !folders.iter().any(|(fid, _)| fid.as_str() == parent),
                None => true,
            };
            if parent_ready {
                write_folder(conn, id, record)?;
            } else {
                pending.push((id, record));
            }
        }
    }
    for (id, record) in pending {
        write_folder(conn, id, record)?;
    }
    Ok(())
}

fn write_folder(conn: &rusqlite::Connection, id: &str, record: &Record) -> Result<()> {
    let name = field(record, "name").unwrap_or("Folder");
    let parent = field(record, "parent_id")
        .filter(|value| !value.is_empty())
        .filter(|parent| row_exists(conn, "folders", parent).unwrap_or(false));
    let position = field(record, "position").and_then(|value| value.parse().ok()).unwrap_or(0);
    let fresh_deleted = if record.deleted { Some(now_secs()) } else { None };
    let existing: Option<Option<i64>> =
        conn.query_row("SELECT deleted_at FROM folders WHERE id = ?1", params![id], |row| row.get(0)).optional()?;
    if let Some(current) = existing {
        let deleted_at = if record.deleted { current.or(fresh_deleted) } else { None };
        conn.execute(
            "UPDATE folders SET name = ?1, parent_id = ?2, position = ?3, deleted_at = ?4 WHERE id = ?5",
            params![name, parent, position, deleted_at, id],
        )?;
    } else {
        conn.execute(
            "INSERT INTO folders(id, name, parent_id, position, deleted_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, name, parent, position, fresh_deleted],
        )?;
    }
    Ok(())
}

fn project_named(conn: &rusqlite::Connection, state: &State) -> Result<()> {
    for (key, record) in state {
        match key.0.as_str() {
            "categories" => write_name(conn, "categories", &key.1, record)?,
            "interests" => write_name(conn, "interests", &key.1, record)?,
            "smart_filters" => write_filter(conn, &key.1, record)?,
            "feed_categories" => write_feed_category(conn, record)?,
            _ => {}
        }
    }
    Ok(())
}

fn write_name(conn: &rusqlite::Connection, table: &str, id: &str, record: &Record) -> Result<()> {
    let Some(name) = field(record, "name").filter(|value| !value.is_empty()) else { return Ok(()) };
    let deleted_at = if record.deleted { Some(now_secs()) } else { None };
    let sql = match table {
        "categories" => {
            "INSERT INTO categories(id, name, deleted_at) VALUES(?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET name = excluded.name, deleted_at = excluded.deleted_at"
        }
        "interests" => {
            "INSERT INTO interests(id, name, deleted_at) VALUES(?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET name = excluded.name, deleted_at = excluded.deleted_at"
        }
        _ => return Ok(()),
    };
    conn.execute(sql, params![id, name, deleted_at])?;
    Ok(())
}

fn write_filter(conn: &rusqlite::Connection, id: &str, record: &Record) -> Result<()> {
    let Some(name) = field(record, "name").filter(|value| !value.is_empty()) else { return Ok(()) };
    let query = field(record, "query").unwrap_or(name);
    let deleted_at = if record.deleted { Some(now_secs()) } else { None };
    conn.execute(
        "INSERT INTO smart_filters(id, name, query, deleted_at) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, query = excluded.query, deleted_at = excluded.deleted_at",
        params![id, name, query, deleted_at],
    )?;
    Ok(())
}

fn write_feed_category(conn: &rusqlite::Connection, record: &Record) -> Result<()> {
    let Some(feed) = field(record, "feed_id") else { return Ok(()) };
    let Some(category) = field(record, "category_id") else { return Ok(()) };
    if !row_exists(conn, "feeds", feed)? || !row_exists(conn, "categories", category)? {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO feed_categories(feed_id, category_id) VALUES(?1, ?2) ON CONFLICT DO NOTHING",
        params![feed, category],
    )?;
    Ok(())
}

fn project_feeds(conn: &rusqlite::Connection, state: &State) -> Result<()> {
    for (key, record) in state {
        if key.0 != "feeds" {
            continue;
        }
        write_feed(conn, &key.1, record)?;
    }
    Ok(())
}

fn write_feed(conn: &rusqlite::Connection, record_id: &str, record: &Record) -> Result<()> {
    let url = field(record, "url").unwrap_or("");
    let existing = conn
        .query_row("SELECT id FROM feeds WHERE id = ?1", params![record_id], |r| r.get::<_, String>(0))
        .optional()?
        .or(if url.is_empty() {
            None
        } else {
            conn.query_row("SELECT id FROM feeds WHERE url = ?1", params![url], |r| r.get::<_, String>(0)).optional()?
        });
    let Some(id) = existing else {
        if url.is_empty() {
            return Ok(());
        }
        let title = field(record, "title").filter(|value| !value.is_empty()).unwrap_or(url);
        let site = field(record, "site_url").unwrap_or("");
        let folder = folder_of(conn, record)?;
        let position = field(record, "position").and_then(|value| value.parse().ok()).unwrap_or(0);
        let poll = field(record, "poll_minutes").and_then(|value| value.parse().ok()).unwrap_or(60);
        let disabled = flag(field(record, "disabled")) as i64;
        let deleted_at = if record.deleted { Some(now_secs()) } else { None };
        conn.execute(
            "INSERT INTO feeds(id, url, title, site_url, folder_id, position, poll_minutes, disabled, deleted_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![record_id, url, title, site, folder, position, poll, disabled, deleted_at],
        )?;
        return Ok(());
    };
    let mut title: String = conn.query_row("SELECT title FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    let mut site: String = conn.query_row("SELECT site_url FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    let mut folder: Option<String> = conn.query_row("SELECT folder_id FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    let mut position: i64 = conn.query_row("SELECT position FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    let mut poll: i64 = conn.query_row("SELECT poll_minutes FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    let mut disabled: i64 = conn.query_row("SELECT disabled FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    let mut deleted_at: Option<i64> = conn.query_row("SELECT deleted_at FROM feeds WHERE id = ?1", params![id], |r| r.get(0))?;
    if let Some(value) = field(record, "title") {
        title = if value.is_empty() { url.to_string() } else { value.to_string() };
    }
    if let Some(value) = field(record, "site_url") {
        site = value.to_string();
    }
    if record.fields.contains_key("folder_id") {
        folder = folder_of(conn, record)?;
    }
    if let Some(value) = field(record, "position").and_then(|value| value.parse().ok()) {
        position = value;
    }
    if let Some(value) = field(record, "poll_minutes").and_then(|value| value.parse().ok()) {
        poll = value;
    }
    if record.fields.contains_key("disabled") {
        disabled = flag(field(record, "disabled")) as i64;
    }
    if record.deleted {
        if deleted_at.is_none() {
            deleted_at = Some(now_secs());
        }
    } else {
        deleted_at = None;
    }
    let url_update = if url.is_empty() { None } else { Some(url) };
    conn.execute(
        "UPDATE feeds SET url = COALESCE(?1, url), title = ?2, site_url = ?3, folder_id = ?4, position = ?5, poll_minutes = ?6, disabled = ?7, deleted_at = ?8 WHERE id = ?9",
        params![url_update, title, site, folder, position, poll, disabled, deleted_at, id],
    )?;
    Ok(())
}

fn project_article_state(conn: &rusqlite::Connection, state: &State) -> Result<()> {
    for (key, record) in state {
        if key.0 != "article_state" || record.deleted {
            continue;
        }
        let article: Option<String> =
            conn.query_row("SELECT id FROM articles WHERE id = ?1", params![key.1], |r| r.get(0)).optional()?;
        let Some(article) = article else { continue };
        conn.execute("INSERT INTO article_state(article_id) VALUES(?1) ON CONFLICT(article_id) DO NOTHING", params![article])?;
        if let Some(read) = field(record, "read") {
            if read == "true" || read == "1" {
                conn.execute(
                    "UPDATE article_state SET read_at = COALESCE(read_at, ?1) WHERE article_id = ?2",
                    params![now_secs(), article],
                )?;
            } else if read == "false" || read == "0" {
                conn.execute("UPDATE article_state SET read_at = NULL WHERE article_id = ?1", params![article])?;
            }
        }
        if record.fields.contains_key("starred") {
            let starred = flag(field(record, "starred")) as i64;
            conn.execute("UPDATE article_state SET starred = ?1 WHERE article_id = ?2", params![starred, article])?;
        }
        if record.fields.contains_key("saved") {
            let saved = flag(field(record, "saved")) as i64;
            conn.execute("UPDATE article_state SET saved = ?1 WHERE article_id = ?2", params![saved, article])?;
        }
    }
    Ok(())
}

fn folder_of(conn: &rusqlite::Connection, record: &Record) -> Result<Option<String>> {
    let Some(folder) = field(record, "folder_id").filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if row_exists(conn, "folders", folder)? { Ok(Some(folder.to_string())) } else { Ok(None) }
}

fn row_exists(conn: &rusqlite::Connection, table: &str, id: &str) -> Result<bool> {
    let sql = format!("SELECT 1 FROM {table} WHERE id = ?1");
    let found: Option<i64> = conn.query_row(&sql, params![id], |r| r.get(0)).optional()?;
    Ok(found.is_some())
}

fn flag(value: Option<&str>) -> bool {
    matches!(value, Some("true" | "1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NewFeed, ParsedFeed, ParsedItem, View};
    use crate::store::open;
    use nixe_sync::{OpKind, Secret, StubServer, WebDav, WebDavConfig};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn scratch() -> PathBuf {
        let path = std::env::temp_dir().join(format!("nixe-news-sync-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn feed(title: &str, url: &str) -> NewFeed {
        NewFeed { url: url.into(), title: title.into(), site_url: String::new(), folder_id: None }
    }

    fn titles(db: &Db) -> Vec<String> {
        db.feeds().unwrap().into_iter().map(|row| row.title).collect()
    }

    #[test]
    fn two_devices_converge_after_offline_edits_and_a_deletion() {
        let stub = StubServer::start("ken", "secret");
        let dir_a = scratch();
        let dir_b = scratch();
        let mut a = open(&dir_a.join("news.sqlite")).unwrap();
        let mut b = open(&dir_b.join("news.sqlite")).unwrap();
        let remote_a = stub.client().unwrap();
        let remote_b = stub.client().unwrap();
        a.insert_feed(&feed("Alpha", "https://alpha.example/feed")).unwrap();
        synchronize(&mut a, &remote_a, &[]).unwrap();
        synchronize(&mut b, &remote_b, &[]).unwrap();
        b.insert_feed(&feed("Beta", "https://beta.example/feed")).unwrap();
        synchronize(&mut b, &remote_b, &[]).unwrap();
        synchronize(&mut a, &remote_a, &[]).unwrap();
        let alpha = a.feeds().unwrap().into_iter().find(|row| row.title == "Alpha").unwrap();
        a.delete_feed(&alpha.id).unwrap();
        synchronize(&mut a, &remote_a, &[]).unwrap();
        synchronize(&mut b, &remote_b, &[]).unwrap();
        assert_eq!(titles(&a), ["Beta"]);
        assert_eq!(titles(&b), ["Beta"]);
    }

    #[test]
    fn an_interrupted_sync_leaves_the_library_unchanged() {
        let dir = scratch();
        let path = dir.join("news.sqlite");
        let mut db = open(&path).unwrap();
        db.insert_feed(&feed("Alpha", "https://alpha.example/feed")).unwrap();
        let ghost = Op {
            hlc: Hlc { wall_ms: 9_000, counter: 0, device: "other".into() },
            collection: "feeds".into(),
            record_id: "ghost".into(),
            kind: OpKind::Upsert,
            fields: [("url".into(), "https://ghost.example/feed".into()), ("title".into(), "Ghost".into())].into(),
            member: None,
        };
        let err = db.apply_exchange(std::slice::from_ref(&ghost), 9, true).unwrap_err();
        assert!(err.to_string().contains("checkpoint"), "{err}");
        drop(db);
        let db = open(&path).unwrap();
        assert_eq!(titles(&db), ["Alpha"]);
        assert_eq!(db.generation().unwrap(), 0);
    }

    #[test]
    fn a_wrong_password_writes_nothing() {
        let stub = StubServer::start("ken", "secret");
        let dir = scratch();
        let mut db = open(&dir.join("news.sqlite")).unwrap();
        db.insert_feed(&feed("Alpha", "https://alpha.example/feed")).unwrap();
        let remote = WebDav::new(WebDavConfig {
            server: stub.url().into(),
            username: "ken".into(),
            password: Secret::new("wrong").unwrap(),
            folder: "Nixe/News".into(),
            allow_insecure_loopback: true,
            recipient: None,
            identity: None,
        })
        .unwrap();
        let err = synchronize(&mut db, &remote, &[]).unwrap_err();
        assert!(err.to_string().contains("refused"), "{err}");
        assert_eq!(stub.puts(), 0);
        assert_eq!(db.generation().unwrap(), 0);
        assert_eq!(titles(&db), ["Alpha"]);
    }

    #[test]
    fn uploaded_batches_omit_article_bodies() {
        let stub = StubServer::start("ken", "secret");
        let dir = scratch();
        let mut db = open(&dir.join("news.sqlite")).unwrap();
        let id = db.insert_feed(&feed("Body", "https://body.example/feed")).unwrap();
        let marker = "UNIQUE_BODY_SHOULD_NOT_SYNC_9f3a";
        db.apply_feed(
            &id,
            &ParsedFeed {
                title: "Body".into(),
                site_url: String::new(),
                items: vec![ParsedItem {
                    item_key: "post".into(),
                    url: "https://body.example/post".into(),
                    original_url: "https://body.example/post".into(),
                    title: "Post".into(),
                    author: String::new(),
                    summary: marker.into(),
                    content_html: marker.into(),
                    content_hash: None,
                    published_at: Some(1),
                    updated_at: None,
                    image_url: None,
                }],
            },
            &crate::model::Ingest { etag: None, last_modified: None, keep_content: false, cache_dir: None },
        )
        .unwrap();
        let article = db.articles(&View::All, "", 10, 0).unwrap().remove(0);
        db.set_read(&article.id, true).unwrap();
        synchronize(&mut db, &stub.client().unwrap(), &[]).unwrap();
        let batch = String::from_utf8(stub.file("batches/1.json").unwrap()).unwrap();
        assert!(!batch.contains(marker), "{batch}");
        assert!(batch.contains("https://body.example/feed"), "{batch}");
    }
}
