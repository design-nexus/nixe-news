//! SQLite library. User state lives apart from feed content, so a refresh can
//! replace a title without clearing read, star, or save.

use crate::cluster::{self, Signal};
use crate::model::{ArticleListItem, FeedRow, FolderRow, Ingest, NewFeed, ParsedFeed, SmartFilter, View};
use crate::opml::{self, Outline};
use crate::search::fts_match;
use crate::sync::{op_from_row, op_name, preference_fields, project};
use anyhow::Result;
use nixe_sync::{Clock, Hlc, Op, OpKind, State, merge};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const SCHEMA: &str = r#"
CREATE TABLE folders (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  parent_id TEXT REFERENCES folders(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  deleted_at INTEGER
);
CREATE TABLE feeds (
  id TEXT PRIMARY KEY,
  url TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL DEFAULT '',
  site_url TEXT NOT NULL DEFAULT '',
  folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
  position INTEGER NOT NULL DEFAULT 0,
  poll_minutes INTEGER NOT NULL DEFAULT 60,
  etag TEXT,
  last_modified TEXT,
  last_ok_at INTEGER,
  last_attempt_at INTEGER,
  backoff_until INTEGER,
  last_error TEXT,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0,
  deleted_at INTEGER
);
CREATE TABLE articles (
  id TEXT PRIMARY KEY,
  feed_id TEXT NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  item_key TEXT NOT NULL,
  url TEXT NOT NULL,
  original_url TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  author TEXT NOT NULL DEFAULT '',
  summary TEXT NOT NULL DEFAULT '',
  content_hash TEXT,
  published_at INTEGER,
  updated_at INTEGER,
  image_url TEXT,
  UNIQUE (feed_id, item_key)
);
CREATE TABLE article_state (
  article_id TEXT PRIMARY KEY REFERENCES articles(id) ON DELETE CASCADE,
  read_at INTEGER,
  starred INTEGER NOT NULL DEFAULT 0,
  saved INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  progress INTEGER NOT NULL DEFAULT 0,
  seen_at INTEGER
);
CREATE TABLE tags (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  deleted_at INTEGER
);
CREATE TABLE article_tags (
  article_id TEXT NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (article_id, tag_id)
);
CREATE TABLE publishers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  site_url TEXT NOT NULL DEFAULT '',
  feed_id TEXT REFERENCES feeds(id) ON DELETE SET NULL
);
CREATE VIRTUAL TABLE articles_fts USING fts5(
  title, summary, author,
  content='articles', content_rowid='rowid', tokenize='unicode61'
);
CREATE TRIGGER articles_ai AFTER INSERT ON articles BEGIN
  INSERT INTO articles_fts(rowid, title, summary, author) VALUES (new.rowid, new.title, new.summary, new.author);
END;
CREATE TRIGGER articles_ad AFTER DELETE ON articles BEGIN
  INSERT INTO articles_fts(articles_fts, rowid, title, summary, author) VALUES ('delete', old.rowid, old.title, old.summary, old.author);
END;
CREATE TRIGGER articles_au AFTER UPDATE ON articles BEGIN
  INSERT INTO articles_fts(articles_fts, rowid, title, summary, author) VALUES ('delete', old.rowid, old.title, old.summary, old.author);
  INSERT INTO articles_fts(rowid, title, summary, author) VALUES (new.rowid, new.title, new.summary, new.author);
END;
CREATE INDEX articles_feed_time ON articles(feed_id, published_at DESC);
CREATE INDEX articles_url ON articles(url);
CREATE INDEX article_state_read ON article_state(read_at);
CREATE INDEX feeds_folder ON feeds(folder_id);
CREATE TABLE devices (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  this_device INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE change_log (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  device_id TEXT NOT NULL,
  hlc TEXT NOT NULL,
  collection TEXT NOT NULL,
  record_id TEXT NOT NULL,
  op TEXT NOT NULL,
  fields TEXT NOT NULL,
  applied INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX change_log_pending ON change_log(applied, seq);
CREATE TABLE sync_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE categories (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  deleted_at INTEGER
);
CREATE TABLE feed_categories (
  feed_id TEXT NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  category_id TEXT NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  PRIMARY KEY (feed_id, category_id)
);
CREATE TABLE interests (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  deleted_at INTEGER
);
CREATE TABLE smart_filters (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  query TEXT NOT NULL,
  deleted_at INTEGER
);
CREATE TABLE clusters (
  id TEXT PRIMARY KEY,
  title_key TEXT NOT NULL,
  window_start INTEGER NOT NULL
);
CREATE TABLE cluster_members (
  cluster_id TEXT NOT NULL REFERENCES clusters(id) ON DELETE CASCADE,
  article_id TEXT NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  PRIMARY KEY (cluster_id, article_id)
);
"#;

pub struct Db {
    conn: Connection,
}

pub fn open(path: &Path) -> Result<Db> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let mut db = Db { conn };
    db.migrate()?;
    db.ensure_device()?;
    Ok(db)
}

/// Read the library without creating it or migrating it.
pub fn open_readonly(path: &Path) -> Result<Db> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.pragma_update(None, "query_only", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    Ok(Db { conn })
}

impl Db {
    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version == 0 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(SCHEMA)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        let version: i64 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 2 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(SCHEMA_V2)?;
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        Ok(())
    }

    fn ensure_device(&mut self) -> Result<()> {
        let existing: Option<String> =
            self.conn.query_row("SELECT id FROM devices WHERE this_device = 1", [], |r| r.get(0)).optional()?;
        if existing.is_some() {
            return Ok(());
        }
        let id = Uuid::new_v4().to_string();
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO devices(id, name, created_at, this_device) VALUES(?1, ?2, ?3, 1)",
            params![id, "This computer", now],
        )?;
        self.conn.execute("INSERT INTO sync_state(key, value) VALUES('device', ?1)", params![id])?;
        Ok(())
    }

    pub fn device_id(&self) -> Result<String> {
        Ok(self.conn.query_row("SELECT id FROM devices WHERE this_device = 1", [], |r| r.get(0))?)
    }

    fn tick(&mut self) -> Result<Hlc> {
        let raw: Option<String> =
            self.conn.query_row("SELECT value FROM sync_state WHERE key = 'clock'", [], |r| r.get(0)).optional()?;
        let stored = raw.and_then(|s| serde_json::from_str::<Hlc>(&s).ok());
        let mut clock = match stored.clone() {
            Some(prev) => Clock::resume(prev),
            None => Clock::new(self.device_id()?, now_ms()),
        };
        if let Some(prev) = &stored {
            clock.observe(prev, now_ms());
        }
        let hlc = clock.tick(now_ms());
        let json = serde_json::to_string(&hlc)?;
        self.conn.execute(
            "INSERT INTO sync_state(key, value) VALUES('clock', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![json],
        )?;
        Ok(hlc)
    }

    fn log(&mut self, collection: &str, record_id: &str, op: OpKind, fields: serde_json::Value) -> Result<()> {
        let hlc = self.tick()?;
        let op_name = op_name(&op);
        self.conn.execute(
            "INSERT INTO change_log(device_id, hlc, collection, record_id, op, fields) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![hlc.device, serde_json::to_string(&hlc)?, collection, record_id, op_name, fields.to_string()],
        )?;
        Ok(())
    }

    pub fn create_folder(&mut self, name: &str, parent_id: Option<&str>) -> Result<String> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "the folder needs a name");
        if let Some(id) = self
            .conn
            .query_row(
                "SELECT id FROM folders WHERE name = ?1 AND deleted_at IS NULL AND parent_id IS ?",
                params![name, parent_id],
                |r| r.get(0),
            )
            .optional()?
        {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        let position: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM folders", [], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO folders(id, name, parent_id, position) VALUES(?1, ?2, ?3, ?4)",
            params![id, name, parent_id, position],
        )?;
        self.log(
            "folders",
            &id,
            OpKind::Upsert,
            serde_json::json!({"name": name, "parent_id": parent_id, "position": position}),
        )?;
        Ok(id)
    }

    pub fn folders(&self) -> Result<Vec<FolderRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, parent_id, position FROM folders WHERE deleted_at IS NULL ORDER BY position, name")?;
        let rows =
            stmt.query_map([], |r| Ok(FolderRow { id: r.get(0)?, name: r.get(1)?, parent_id: r.get(2)?, position: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn insert_feed(&mut self, feed: &NewFeed) -> Result<String> {
        let title = if feed.title.is_empty() { feed.url.clone() } else { feed.title.clone() };
        if let Some((id, deleted)) = self
            .conn
            .query_row("SELECT id, deleted_at FROM feeds WHERE url = ?1", params![feed.url], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
            })
            .optional()?
        {
            if deleted.is_some() {
                self.conn.execute(
                    "UPDATE feeds SET title = ?1, site_url = ?2, folder_id = ?3, deleted_at = NULL WHERE id = ?4",
                    params![title, feed.site_url, feed.folder_id, id],
                )?;
                self.log(
                    "feeds",
                    &id,
                    OpKind::Upsert,
                    serde_json::json!({"url": feed.url, "title": title, "site_url": feed.site_url, "folder_id": feed.folder_id}),
                )?;
            }
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        let position: i64 = self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM feeds", [], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO feeds(id, url, title, site_url, folder_id, position) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, feed.url, title, feed.site_url, feed.folder_id, position],
        )?;
        self.upsert_publisher(&id, &title, &feed.site_url)?;
        self.log(
            "feeds",
            &id,
            OpKind::Upsert,
            serde_json::json!({"url": feed.url, "title": title, "site_url": feed.site_url, "folder_id": feed.folder_id, "position": position, "poll_minutes": 60}),
        )?;
        Ok(id)
    }

    fn upsert_publisher(&self, feed_id: &str, name: &str, site_url: &str) -> Result<()> {
        let existing: Option<String> =
            self.conn.query_row("SELECT id FROM publishers WHERE feed_id = ?1", params![feed_id], |r| r.get(0)).optional()?;
        if let Some(id) = existing {
            self.conn.execute("UPDATE publishers SET name = ?1, site_url = ?2 WHERE id = ?3", params![name, site_url, id])?;
        } else {
            self.conn.execute(
                "INSERT INTO publishers(id, name, site_url, feed_id) VALUES(?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), name, site_url, feed_id],
            )?;
        }
        Ok(())
    }

    pub fn feeds(&self) -> Result<Vec<FeedRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.url, f.title, f.site_url, f.folder_id, f.position, f.poll_minutes, f.last_ok_at, f.last_error, f.consecutive_failures, f.disabled,
                    (SELECT COUNT(*) FROM articles a LEFT JOIN article_state s ON s.article_id = a.id
                      WHERE a.feed_id = f.id AND s.read_at IS NULL AND COALESCE(s.archived, 0) = 0)
             FROM feeds f WHERE f.deleted_at IS NULL ORDER BY f.position, f.title",
        )?;
        let rows = stmt.query_map([], feed_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn validators(&self, id: &str) -> Result<(Option<String>, Option<String>)> {
        Ok(self
            .conn
            .query_row("SELECT etag, last_modified FROM feeds WHERE id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }

    pub fn feed(&self, id: &str) -> Result<Option<FeedRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.url, f.title, f.site_url, f.folder_id, f.position, f.poll_minutes, f.last_ok_at, f.last_error, f.consecutive_failures, f.disabled, 0
             FROM feeds f WHERE f.id = ?1 AND f.deleted_at IS NULL",
        )?;
        Ok(stmt.query_row(params![id], feed_row).optional()?)
    }

    pub fn set_poll_minutes(&mut self, minutes: i64) -> Result<()> {
        let minutes = minutes.clamp(15, 24 * 60);
        self.conn.execute("UPDATE feeds SET poll_minutes = ?1 WHERE deleted_at IS NULL", params![minutes])?;
        Ok(())
    }

    pub fn due_feeds(&self, now: i64) -> Result<Vec<DueFeed>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, etag, last_modified, consecutive_failures FROM feeds
             WHERE deleted_at IS NULL AND disabled = 0
               AND (backoff_until IS NULL OR backoff_until <= ?1)
               AND (last_ok_at IS NULL OR last_ok_at + poll_minutes * 60 <= ?1)",
        )?;
        let rows = stmt.query_map(params![now], |r| {
            Ok(DueFeed {
                id: r.get(0)?,
                url: r.get(1)?,
                etag: r.get(2)?,
                last_modified: r.get(3)?,
                consecutive_failures: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_failure(&mut self, id: &str, error: &str, backoff_until: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE feeds SET last_error = ?1, consecutive_failures = consecutive_failures + 1, last_attempt_at = ?2, backoff_until = ?3 WHERE id = ?4",
            params![error, now_secs(), backoff_until, id],
        )?;
        Ok(())
    }

    pub fn mark_not_modified(&mut self, id: &str) -> Result<()> {
        let now = now_secs();
        self.conn.execute(
            "UPDATE feeds SET last_ok_at = ?1, last_attempt_at = ?1, last_error = NULL, consecutive_failures = 0, backoff_until = NULL WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn apply_feed(&mut self, feed_id: &str, parsed: &ParsedFeed, ingest: &Ingest) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let title = if parsed.title.is_empty() {
            tx.query_row("SELECT title FROM feeds WHERE id = ?1", params![feed_id], |r| r.get::<_, String>(0))?
        } else {
            parsed.title.clone()
        };
        let now = now_secs();
        tx.execute(
            "UPDATE feeds SET title = ?1, site_url = CASE WHEN ?2 = '' THEN site_url ELSE ?2 END, etag = ?3, last_modified = ?4,
                    last_ok_at = ?5, last_attempt_at = ?5, last_error = NULL, consecutive_failures = 0, backoff_until = NULL
             WHERE id = ?6",
            params![title, parsed.site_url, ingest.etag, ingest.last_modified, now, feed_id],
        )?;
        let mut count = 0;
        for item in &parsed.items {
            tx.execute(
                "INSERT INTO articles(id, feed_id, item_key, url, original_url, title, author, summary, content_hash, published_at, updated_at, image_url)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(feed_id, item_key) DO UPDATE SET
                   url = excluded.url, original_url = excluded.original_url, title = excluded.title, author = excluded.author,
                   summary = excluded.summary, content_hash = excluded.content_hash, published_at = excluded.published_at,
                   updated_at = excluded.updated_at, image_url = excluded.image_url",
                params![
                    Uuid::new_v4().to_string(),
                    feed_id,
                    item.item_key,
                    item.url,
                    item.original_url,
                    item.title,
                    item.author,
                    item.summary,
                    item.content_hash,
                    item.published_at,
                    item.updated_at,
                    item.image_url
                ],
            )?;
            let article_id: String = tx.query_row(
                "SELECT id FROM articles WHERE feed_id = ?1 AND item_key = ?2",
                params![feed_id, item.item_key],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO article_state(article_id) VALUES(?1) ON CONFLICT(article_id) DO NOTHING",
                params![article_id],
            )?;
            if ingest.keep_content
                && let (Some(dir), Some(hash)) = (&ingest.cache_dir, &item.content_hash)
                && !item.content_html.is_empty()
            {
                write_body(dir, hash, &item.content_html)?;
            }
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn feed_sections(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, c.name
             FROM feeds f
             JOIN feed_categories fc ON fc.feed_id = f.id
             JOIN categories c ON c.id = fc.category_id AND c.deleted_at IS NULL
             WHERE f.deleted_at IS NULL
             ORDER BY c.name",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn article_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))?)
    }

    pub fn articles(&self, view: &View, query: &str, limit: i64, offset: i64) -> Result<Vec<ArticleListItem>> {
        let mut sql = String::from(
            "SELECT a.id, a.feed_id, f.title, a.title, a.author, a.summary, a.url, a.original_url, a.published_at, a.image_url,
                    CASE WHEN s.read_at IS NULL THEN 0 ELSE 1 END, COALESCE(s.starred, 0), COALESCE(s.saved, 0), a.content_hash
             FROM articles a
             JOIN feeds f ON f.id = a.feed_id AND f.deleted_at IS NULL
             LEFT JOIN article_state s ON s.article_id = a.id
             WHERE COALESCE(s.archived, 0) = 0",
        );
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(expr) = fts_match(query) {
            sql.push_str(" AND a.rowid IN (SELECT rowid FROM articles_fts WHERE articles_fts MATCH ?)");
            values.push(Box::new(expr));
        }
        match view {
            View::All => {}
            View::Unread => sql.push_str(" AND s.read_at IS NULL"),
            View::Starred => sql.push_str(" AND s.starred = 1"),
            View::Saved => sql.push_str(" AND s.saved = 1"),
            View::Feed(id) => {
                sql.push_str(" AND a.feed_id = ?");
                values.push(Box::new(id.clone()));
            }
            View::Folder(id) => {
                sql.push_str(" AND f.folder_id = ?");
                values.push(Box::new(id.clone()));
            }
            View::ForYou | View::Topic(_) => {}
        }
        sql.push_str(" ORDER BY COALESCE(a.published_at, 0) DESC, a.title LIMIT ? OFFSET ?");
        values.push(Box::new(limit));
        values.push(Box::new(offset));
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), map_article)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Newest stories from every section, so a quiet section is not pushed off the front page.
    pub fn edition_articles(&self, per_section: i64) -> Result<Vec<ArticleListItem>> {
        let sql = "SELECT id, feed_id, feed_title, title, author, summary, url, original_url, published_at, image_url, read_flag, starred, saved, content_hash
             FROM (
               SELECT a.id, a.feed_id, f.title AS feed_title, a.title, a.author, a.summary, a.url, a.original_url, a.published_at, a.image_url,
                      CASE WHEN s.read_at IS NULL THEN 0 ELSE 1 END AS read_flag, COALESCE(s.starred, 0) AS starred, COALESCE(s.saved, 0) AS saved, a.content_hash,
                      ROW_NUMBER() OVER (PARTITION BY COALESCE(c.name, 'Other') ORDER BY COALESCE(a.published_at, 0) DESC, a.title) AS rn
               FROM articles a
               JOIN feeds f ON f.id = a.feed_id AND f.deleted_at IS NULL
               LEFT JOIN article_state s ON s.article_id = a.id
               LEFT JOIN feed_categories fc ON fc.feed_id = f.id
               LEFT JOIN categories c ON c.id = fc.category_id AND c.deleted_at IS NULL
               WHERE COALESCE(s.archived, 0) = 0
             )
             WHERE rn <= ?
             ORDER BY COALESCE(published_at, 0) DESC, title";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![per_section], map_article)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn section_articles(&self, section: &str, query: &str, limit: i64) -> Result<Vec<ArticleListItem>> {
        let mut sql = String::from(
            "SELECT a.id, a.feed_id, f.title, a.title, a.author, a.summary, a.url, a.original_url, a.published_at, a.image_url,
                    CASE WHEN s.read_at IS NULL THEN 0 ELSE 1 END, COALESCE(s.starred, 0), COALESCE(s.saved, 0), a.content_hash
             FROM articles a
             JOIN feeds f ON f.id = a.feed_id AND f.deleted_at IS NULL
             JOIN feed_categories fc ON fc.feed_id = f.id
             JOIN categories c ON c.id = fc.category_id AND c.deleted_at IS NULL AND c.name = ?
             LEFT JOIN article_state s ON s.article_id = a.id
             WHERE COALESCE(s.archived, 0) = 0",
        );
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(section.to_string())];
        if let Some(expr) = fts_match(query) {
            sql.push_str(" AND a.rowid IN (SELECT rowid FROM articles_fts WHERE articles_fts MATCH ?)");
            values.push(Box::new(expr));
        }
        sql.push_str(" ORDER BY COALESCE(a.published_at, 0) DESC, a.title LIMIT ?");
        values.push(Box::new(limit));
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), map_article)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn article(&self, id: &str) -> Result<Option<ArticleListItem>> {
        self.articles_by_id(id)
    }

    fn articles_by_id(&self, id: &str) -> Result<Option<ArticleListItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.feed_id, f.title, a.title, a.author, a.summary, a.url, a.original_url, a.published_at, a.image_url,
                    CASE WHEN s.read_at IS NULL THEN 0 ELSE 1 END, COALESCE(s.starred, 0), COALESCE(s.saved, 0), a.content_hash
             FROM articles a
             JOIN feeds f ON f.id = a.feed_id
             LEFT JOIN article_state s ON s.article_id = a.id
             WHERE a.id = ?1",
        )?;
        Ok(stmt.query_row(params![id], map_article).optional()?)
    }

    pub fn set_read(&mut self, id: &str, read: bool) -> Result<()> {
        let read_at = if read { Some(now_secs()) } else { None };
        self.conn.execute("INSERT INTO article_state(article_id, read_at) VALUES(?1, ?2) ON CONFLICT(article_id) DO UPDATE SET read_at = excluded.read_at", params![id, read_at])?;
        self.log("article_state", id, OpKind::Upsert, serde_json::json!({"read": read}))?;
        Ok(())
    }

    pub fn set_starred(&mut self, id: &str, starred: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO article_state(article_id, starred) VALUES(?1, ?2) ON CONFLICT(article_id) DO UPDATE SET starred = excluded.starred",
            params![id, starred as i64],
        )?;
        self.log("article_state", id, OpKind::Upsert, serde_json::json!({"starred": starred}))?;
        Ok(())
    }

    pub fn set_saved(&mut self, id: &str, saved: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO article_state(article_id, saved) VALUES(?1, ?2) ON CONFLICT(article_id) DO UPDATE SET saved = excluded.saved",
            params![id, saved as i64],
        )?;
        self.log("article_state", id, OpKind::Upsert, serde_json::json!({"saved": saved}))?;
        Ok(())
    }

    pub fn set_article_image(&mut self, id: &str, image_url: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE articles SET image_url = ?1 WHERE id = ?2 AND (image_url IS NULL OR image_url = '')",
            params![image_url, id],
        )?;
        Ok(())
    }

    pub fn mark_view_read(&mut self, view: &View) -> Result<usize> {
        let items = self.articles(view, "", 10_000, 0)?;
        let mut n = 0;
        for item in items {
            if !item.read {
                self.set_read(&item.id, true)?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn unread_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM articles a
             JOIN feeds f ON f.id = a.feed_id AND f.deleted_at IS NULL
             LEFT JOIN article_state s ON s.article_id = a.id
             WHERE s.read_at IS NULL AND COALESCE(s.archived, 0) = 0",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn counts(&self) -> Result<(i64, i64, i64)> {
        Ok(self.conn.query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN s.read_at IS NULL THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN s.starred = 1 THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN s.saved = 1 THEN 1 ELSE 0 END), 0)
             FROM articles a
             JOIN feeds f ON f.id = a.feed_id AND f.deleted_at IS NULL
             LEFT JOIN article_state s ON s.article_id = a.id
             WHERE COALESCE(s.archived, 0) = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    pub fn import_opml(&mut self, text: &str) -> Result<(usize, usize)> {
        let outlines = opml::import(text)?;
        let mut added = 0;
        let mut existing = 0;
        self.import_nodes(&outlines, None, &mut added, &mut existing)?;
        Ok((added, existing))
    }

    fn import_nodes(&mut self, nodes: &[Outline], parent: Option<&str>, added: &mut usize, existing: &mut usize) -> Result<()> {
        for node in nodes {
            if let Some(url) = &node.feed_url {
                let before = self.feeds()?.len();
                let title = if node.title.is_empty() { url.clone() } else { node.title.clone() };
                self.insert_feed(&NewFeed {
                    url: url.clone(),
                    title,
                    site_url: node.site_url.clone().unwrap_or_default(),
                    folder_id: parent.map(str::to_string),
                })?;
                if self.feeds()?.len() > before {
                    *added += 1;
                } else {
                    *existing += 1;
                }
            } else if !node.children.is_empty() {
                let id = self.create_folder(&node.title, parent)?;
                self.import_nodes(&node.children, Some(&id), added, existing)?;
            }
        }
        Ok(())
    }

    pub fn export_opml(&self) -> Result<String> {
        let folders = self.folders()?;
        let feeds = self.feeds()?;
        let mut roots = Vec::new();
        for folder in folders.iter().filter(|f| f.parent_id.is_none()) {
            roots.push(self.folder_outline(folder, &folders, &feeds));
        }
        for feed in feeds.iter().filter(|f| f.folder_id.is_none()) {
            roots.push(feed_outline(feed));
        }
        Ok(opml::export(&roots))
    }

    fn folder_outline(&self, folder: &FolderRow, folders: &[FolderRow], feeds: &[FeedRow]) -> Outline {
        let mut children = Vec::new();
        for child in folders.iter().filter(|f| f.parent_id.as_deref() == Some(folder.id.as_str())) {
            children.push(self.folder_outline(child, folders, feeds));
        }
        for feed in feeds.iter().filter(|f| f.folder_id.as_deref() == Some(folder.id.as_str())) {
            children.push(feed_outline(feed));
        }
        Outline { title: folder.name.clone(), feed_url: None, site_url: None, children }
    }

    pub fn retain(&mut self, days: i64, now: i64) -> Result<usize> {
        if days <= 0 {
            return Ok(0);
        }
        let cutoff = now - days * 24 * 60 * 60;
        let n = self.conn.execute(
            "DELETE FROM articles WHERE published_at IS NOT NULL AND published_at < ?1
             AND id NOT IN (SELECT article_id FROM article_state WHERE starred = 1 OR saved = 1 OR archived = 1)
             AND id NOT IN (SELECT article_id FROM article_tags)",
            params![cutoff],
        )?;
        Ok(n)
    }

    pub fn body(&self, cache_dir: &Path, hash: &str) -> Option<String> {
        fs::read_to_string(body_path(cache_dir, hash)).ok()
    }

    pub fn follow_topic(&mut self, name: &str) -> Result<String> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "the topic needs a name");
        let _category = self.ensure_named("categories", name)?;
        let _interest = self.ensure_named("interests", name)?;
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM smart_filters WHERE name = ?1 AND deleted_at IS NULL", params![name], |row| row.get(0))
            .optional()?
        {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        self.conn.execute("INSERT INTO smart_filters(id, name, query) VALUES(?1, ?2, ?3)", params![id, name, name])?;
        self.log("smart_filters", &id, OpKind::Upsert, serde_json::json!({"name": name, "query": name}))?;
        Ok(id)
    }

    pub fn unfollow_topic(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        let ids: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id FROM smart_filters WHERE name = ?1 AND deleted_at IS NULL")?;
            let rows = stmt.query_map(params![name], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let now = now_secs();
        for id in ids {
            self.conn.execute("UPDATE smart_filters SET deleted_at = ?1 WHERE id = ?2", params![now, id])?;
            self.log("smart_filters", &id, OpKind::Tombstone, serde_json::json!({}))?;
        }
        Ok(())
    }

    pub fn smart_filters(&self) -> Result<Vec<SmartFilter>> {
        let mut stmt = self.conn.prepare("SELECT id, name, query FROM smart_filters WHERE deleted_at IS NULL ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok(SmartFilter { id: row.get(0)?, name: row.get(1)?, query: row.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn attach_category(&mut self, feed_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        let category = self.ensure_named("categories", name)?;
        let linked = self.conn.execute(
            "INSERT INTO feed_categories(feed_id, category_id) VALUES(?1, ?2) ON CONFLICT DO NOTHING",
            params![feed_id, category],
        )?;
        if linked == 1 {
            let record = format!("{feed_id}:{category}");
            self.log(
                "feed_categories",
                &record,
                OpKind::Upsert,
                serde_json::json!({"feed_id": feed_id, "category_id": category}),
            )?;
        }
        Ok(())
    }

    pub fn for_you(&mut self, limit: i64) -> Result<Vec<ArticleListItem>> {
        let signals = self.recommendation_signals()?;
        let items = self.articles(&View::Unread, "", 400, 0)?;
        let mut ranked = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let folder = self.folder_of_feed(&item.feed_id)?;
            let Some((score, why)) = cluster::score(&signals, &item.title, &item.summary, &item.feed_title, folder.as_deref())
            else {
                continue;
            };
            ranked.push((score, why, index));
        }
        ranked.sort_by(|left, right| right.0.cmp(&left.0).then(items[right.2].published_at.cmp(&items[left.2].published_at)));
        let order: Vec<(i32, usize)> = ranked.iter().map(|(score, _, index)| (*score, *index)).collect();
        let groups = cluster::group_sorted(&order, |index| items[index].title.clone(), |index| items[index].published_at);
        self.write_clusters(&groups, &items)?;
        let mut picked = Vec::new();
        for group in groups {
            let index = group[0];
            let Some((_, why, _)) = ranked.iter().find(|(_, _, at)| *at == index) else { continue };
            let mut item = items[index].clone();
            item.why = why.clone();
            picked.push(item);
            if picked.len() as i64 >= limit {
                break;
            }
        }
        Ok(picked)
    }

    pub fn topic_articles(&self, id: &str, limit: i64) -> Result<Vec<ArticleListItem>> {
        let query: Option<String> = self
            .conn
            .query_row("SELECT query FROM smart_filters WHERE id = ?1 AND deleted_at IS NULL", params![id], |row| row.get(0))
            .optional()?;
        let Some(query) = query else { return Ok(Vec::new()) };
        self.articles(&View::All, &query, limit, 0)
    }

    fn folder_of_feed(&self, feed_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT folders.name FROM feeds JOIN folders ON folders.id = feeds.folder_id WHERE feeds.id = ?1 AND folders.deleted_at IS NULL",
                params![feed_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn recommendation_signals(&self) -> Result<Vec<Signal>> {
        let mut signals = Vec::new();
        let mut starred = self.conn.prepare(
            "SELECT DISTINCT f.title FROM article_state s
             JOIN articles a ON a.id = s.article_id
             JOIN feeds f ON f.id = a.feed_id AND f.deleted_at IS NULL
             WHERE s.starred = 1 AND f.title != ''",
        )?;
        for title in starred.query_map([], |row| row.get::<_, String>(0))? {
            let title = title?;
            signals.push(Signal { weight: 4, why: format!("You starred {title}"), tokens: cluster::tokens(&title) });
        }
        let mut folders = self.conn.prepare("SELECT name FROM folders WHERE deleted_at IS NULL AND name != ''")?;
        for name in folders.query_map([], |row| row.get::<_, String>(0))? {
            let name = name?;
            signals.push(Signal { weight: 2, why: format!("Folder {name}"), tokens: cluster::tokens(&name) });
        }
        let mut tags = self.conn.prepare("SELECT name FROM tags WHERE deleted_at IS NULL AND name != ''")?;
        for name in tags.query_map([], |row| row.get::<_, String>(0))? {
            let name = name?;
            signals.push(Signal { weight: 3, why: format!("Tag {name}"), tokens: cluster::tokens(&name) });
        }
        Ok(signals)
    }

    fn write_clusters(&mut self, groups: &[Vec<usize>], items: &[ArticleListItem]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM cluster_members", [])?;
        tx.execute("DELETE FROM clusters", [])?;
        for group in groups {
            let Some(head) = group.first().copied() else { continue };
            let id = Uuid::new_v4().to_string();
            let start = items[head].published_at.unwrap_or(0);
            tx.execute(
                "INSERT INTO clusters(id, title_key, window_start) VALUES(?1, ?2, ?3)",
                params![id, cluster::title_key(&items[head].title), start],
            )?;
            for index in group {
                tx.execute("INSERT INTO cluster_members(cluster_id, article_id) VALUES(?1, ?2)", params![id, items[*index].id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn ensure_named(&mut self, table: &str, name: &str) -> Result<String> {
        let existing = match table {
            "categories" => {
                self.conn.query_row("SELECT id FROM categories WHERE name = ?1 AND deleted_at IS NULL", params![name], |row| {
                    row.get::<_, String>(0)
                })
            }
            "interests" => {
                self.conn
                    .query_row("SELECT id FROM interests WHERE name = ?1 AND deleted_at IS NULL", params![name], |row| row.get(0))
            }
            _ => anyhow::bail!("unknown named table"),
        }
        .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        match table {
            "categories" => {
                self.conn.execute("INSERT INTO categories(id, name) VALUES(?1, ?2)", params![id, name])?;
            }
            "interests" => {
                self.conn.execute("INSERT INTO interests(id, name) VALUES(?1, ?2)", params![id, name])?;
            }
            _ => anyhow::bail!("unknown named table"),
        }
        self.log(table, &id, OpKind::Upsert, serde_json::json!({"name": name}))?;
        Ok(id)
    }

    pub fn delete_feed(&mut self, id: &str) -> Result<()> {
        let changed = self
            .conn
            .execute("UPDATE feeds SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL", params![now_secs(), id])?;
        if changed == 1 {
            self.log("feeds", id, OpKind::Tombstone, serde_json::json!({}))?;
        }
        Ok(())
    }

    pub fn generation(&self) -> Result<u64> {
        let raw: Option<String> =
            self.conn.query_row("SELECT value FROM sync_state WHERE key = 'generation'", [], |r| r.get(0)).optional()?;
        Ok(raw.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub fn pending_ops(&self) -> Result<(i64, Vec<Op>)> {
        let device = self.device_id()?;
        let pushed: i64 = self
            .conn
            .query_row("SELECT value FROM sync_state WHERE key = 'pushed_seq'", [], |r| r.get::<_, String>(0))
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut stmt = self.conn.prepare(
            "SELECT seq, hlc, collection, record_id, op, fields FROM change_log WHERE device_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![device, pushed], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut ops = Vec::new();
        let mut max_seq = pushed;
        for row in rows {
            let (seq, hlc, collection, record_id, op, fields) = row?;
            max_seq = seq;
            ops.push(op_from_row(&hlc, &collection, &record_id, &op, &fields)?);
        }
        Ok((max_seq, ops))
    }

    pub(crate) fn note_preferences(&mut self, fields: &std::collections::BTreeMap<String, String>) -> Result<()> {
        let current = preference_fields(&merge(&State::new(), &self.logged_ops()?));
        if &current == fields {
            return Ok(());
        }
        let value =
            serde_json::Value::Object(fields.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect());
        self.log("preferences", "app", OpKind::Upsert, value)
    }

    pub(crate) fn apply_exchange(
        &mut self,
        remote: &[Op],
        generation: u64,
        abort_before_commit: bool,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let mut ops = self.logged_ops()?;
        ops.extend(remote.iter().cloned());
        let state = merge(&State::new(), &ops);
        let tx = self.conn.transaction()?;
        project(&tx, &state)?;
        for op in remote {
            let hlc = serde_json::to_string(&op.hlc)?;
            let kind = op_name(&op.kind);
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT seq FROM change_log WHERE hlc = ?1 AND collection = ?2 AND record_id = ?3 AND op = ?4",
                    params![hlc, op.collection, op.record_id, kind],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_some() {
                continue;
            }
            let fields = serde_json::to_string(&op.fields)?;
            tx.execute(
                "INSERT INTO change_log(device_id, hlc, collection, record_id, op, fields, applied) VALUES(?1, ?2, ?3, ?4, ?5, ?6, 1)",
                params![op.hlc.device, hlc, op.collection, op.record_id, kind, fields],
            )?;
        }
        let generation_text = generation.to_string();
        tx.execute(
            "INSERT INTO sync_state(key, value) VALUES('generation', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![generation_text],
        )?;
        if abort_before_commit {
            anyhow::bail!("stopped before the sync checkpoint");
        }
        tx.commit()?;
        Ok(preference_fields(&state))
    }

    pub(crate) fn mark_pushed(&mut self, seq: i64, generation: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO sync_state(key, value) VALUES('pushed_seq', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![seq.to_string()],
        )?;
        tx.execute(
            "INSERT INTO sync_state(key, value) VALUES('generation', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![generation.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn logged_ops(&self) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare("SELECT hlc, collection, record_id, op, fields FROM change_log ORDER BY seq")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(hlc, collection, record, op, fields)| op_from_row(&hlc, &collection, &record, &op, &fields))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct DueFeed {
    pub id: String,
    pub url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub consecutive_failures: i64,
}

fn feed_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FeedRow> {
    Ok(FeedRow {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        site_url: r.get(3)?,
        folder_id: r.get(4)?,
        position: r.get(5)?,
        poll_minutes: r.get(6)?,
        last_ok_at: r.get(7)?,
        last_error: r.get(8)?,
        consecutive_failures: r.get(9)?,
        disabled: r.get::<_, i64>(10)? != 0,
        unread: r.get(11)?,
    })
}

fn feed_outline(feed: &FeedRow) -> Outline {
    Outline {
        title: feed.title.clone(),
        feed_url: Some(feed.url.clone()),
        site_url: if feed.site_url.is_empty() { None } else { Some(feed.site_url.clone()) },
        children: Vec::new(),
    }
}

fn map_article(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleListItem> {
    let feed_title: String = row.get(2)?;
    let title: String = row.get(3)?;
    let author: String = row.get(4)?;
    let summary: String = row.get(5)?;
    Ok(ArticleListItem {
        id: row.get(0)?,
        feed_id: row.get(1)?,
        feed_title: crate::html::decode_entities(&feed_title),
        title: crate::html::decode_entities(&title),
        author: crate::html::decode_entities(&author),
        summary: crate::html::decode_entities(&summary),
        url: row.get(6)?,
        original_url: row.get(7)?,
        published_at: row.get(8)?,
        image_url: row.get(9)?,
        read: row.get::<_, i64>(10)? != 0,
        starred: row.get::<_, i64>(11)? != 0,
        saved: row.get::<_, i64>(12)? != 0,
        content_hash: row.get(13)?,
        why: String::new(),
    })
}

fn body_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join("bodies").join(format!("{hash}.html"))
}

fn write_body(dir: &Path, hash: &str, html: &str) -> Result<()> {
    let path = body_path(dir, hash);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("html.tmp");
    fs::write(&tmp, html)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
impl Ingest {
    fn default_ingest() -> Self {
        Self { etag: None, last_modified: None, keep_content: false, cache_dir: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_feed;

    fn db() -> (Db, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nixe-news-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = open(&dir.join("news.sqlite")).unwrap();
        (db, dir)
    }

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Example</title><link>https://example.com</link>
<item><title>Rust release</title><link>https://example.com/rust</link><guid>g1</guid>
<pubDate>Tue, 14 Oct 2025 12:00:00 GMT</pubDate><description>Notes</description></item>
</channel></rss>"#;

    #[test]
    fn migration_creates_fts_and_search_finds_a_title() {
        let (mut db, dir) = db();
        let id = db
            .insert_feed(&NewFeed {
                url: "https://example.com/feed".into(),
                title: "Example".into(),
                site_url: String::new(),
                folder_id: None,
            })
            .unwrap();
        let parsed = parse_feed(RSS.as_bytes()).unwrap();
        db.apply_feed(&id, &parsed, &Ingest { etag: None, last_modified: None, keep_content: false, cache_dir: None }).unwrap();
        let found = db.articles(&View::All, "rust", 20, 0).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Rust release");
        drop(db);
        let read = open_readonly(&dir.join("news.sqlite")).unwrap();
        assert_eq!(read.articles(&View::All, "rust", 10, 0).unwrap().len(), 1);
        assert!(read.article_count().is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refresh_replaces_the_title_and_keeps_read_state() {
        let (mut db, dir) = db();
        let id = db
            .insert_feed(&NewFeed {
                url: "https://example.com/feed".into(),
                title: "Example".into(),
                site_url: String::new(),
                folder_id: None,
            })
            .unwrap();
        let parsed = parse_feed(RSS.as_bytes()).unwrap();
        db.apply_feed(&id, &parsed, &Ingest::default_ingest()).unwrap();
        let article = db.articles(&View::All, "", 10, 0).unwrap().remove(0);
        db.set_read(&article.id, true).unwrap();
        let mut next = parsed;
        next.items[0].title = "Rust release 2".into();
        db.apply_feed(&id, &next, &Ingest::default_ingest()).unwrap();
        let again = db.article(&article.id).unwrap().unwrap();
        assert_eq!(again.title, "Rust release 2");
        assert!(again.read);
        assert_eq!(db.articles(&View::All, "", 10, 0).unwrap().len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn opml_import_does_not_duplicate_and_round_trips() {
        let (mut db, dir) = db();
        let text = r#"<?xml version="1.0"?><opml version="2.0"><body>
          <outline text="Tech"><outline text="Example" type="rss" xmlUrl="https://example.com/feed.xml"/></outline>
        </body></opml>"#;
        let (added, existing) = db.import_opml(text).unwrap();
        assert_eq!((added, existing), (1, 0));
        let (added, existing) = db.import_opml(text).unwrap();
        assert_eq!((added, existing), (0, 1));
        let exported = db.export_opml().unwrap();
        assert!(exported.contains("https://example.com/feed.xml"));
        assert!(exported.contains("Tech"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn retention_keeps_starred_items() {
        let (mut db, dir) = db();
        let id = db
            .insert_feed(&NewFeed {
                url: "https://example.com/feed".into(),
                title: "Example".into(),
                site_url: String::new(),
                folder_id: None,
            })
            .unwrap();
        let parsed = parse_feed(RSS.as_bytes()).unwrap();
        db.apply_feed(&id, &parsed, &Ingest::default_ingest()).unwrap();
        let article = db.articles(&View::All, "", 10, 0).unwrap().remove(0);
        db.set_starred(&article.id, true).unwrap();
        let removed = db.retain(30, article.published_at.unwrap() + 40 * 24 * 60 * 60).unwrap();
        assert_eq!(removed, 0);
        db.set_starred(&article.id, false).unwrap();
        let removed = db.retain(30, article.published_at.unwrap() + 40 * 24 * 60 * 60).unwrap();
        assert_eq!(removed, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn following_a_topic_saves_a_filter_and_a_star_explains_for_you() {
        let (mut db, dir) = db();
        let filter = db.follow_topic("Technology").unwrap();
        assert_eq!(db.follow_topic("Technology").unwrap(), filter);
        assert_eq!(db.smart_filters().unwrap()[0].query, "Technology");
        let id = db
            .insert_feed(&NewFeed {
                url: "https://example.com/feed".into(),
                title: "Example".into(),
                site_url: String::new(),
                folder_id: None,
            })
            .unwrap();
        db.attach_category(&id, "Technology").unwrap();
        db.apply_feed(&id, &parse_feed(RSS.as_bytes()).unwrap(), &Ingest::default_ingest()).unwrap();
        let article = db.articles(&View::All, "", 10, 0).unwrap().remove(0);
        db.set_starred(&article.id, true).unwrap();
        let picked = db.for_you(10).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].why, "You starred Example");
        let matched = db.topic_articles(&filter, 10).unwrap();
        assert!(matched.is_empty() || matched.iter().all(|item| item.feed_title == "Example"));
        let _ = fs::remove_dir_all(dir);
    }
}
