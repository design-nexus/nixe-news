use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedItem {
    pub item_key: String,
    pub url: String,
    pub original_url: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub content_html: String,
    pub content_hash: Option<String>,
    pub published_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub image_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedFeed {
    pub title: String,
    pub site_url: String,
    pub items: Vec<ParsedItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRow {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub position: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedRow {
    pub id: String,
    pub url: String,
    pub title: String,
    pub site_url: String,
    pub folder_id: Option<String>,
    pub position: i64,
    pub poll_minutes: i64,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
    pub consecutive_failures: i64,
    pub disabled: bool,
    pub unread: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArticleListItem {
    pub id: String,
    pub feed_id: String,
    pub feed_title: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub url: String,
    pub original_url: String,
    pub published_at: Option<i64>,
    pub image_url: Option<String>,
    pub read: bool,
    pub starred: bool,
    pub saved: bool,
    pub content_hash: Option<String>,
    /// Why For You picked this article. Empty on every other list.
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartFilter {
    pub id: String,
    pub name: String,
    pub query: String,
}

/// What the Following list is showing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    All,
    Unread,
    Starred,
    Saved,
    Feed(String),
    Folder(String),
    ForYou,
    Topic(String),
}

#[derive(Clone, Debug)]
pub struct NewFeed {
    pub url: String,
    pub title: String,
    pub site_url: String,
    pub folder_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Ingest {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub keep_content: bool,
    pub cache_dir: Option<std::path::PathBuf>,
}
