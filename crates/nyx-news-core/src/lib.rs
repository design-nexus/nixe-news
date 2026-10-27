//! The news library without GTK. The app opens a [`Db`] on the main thread and
//! another on the refresh worker. Article bodies stay in the cache directory.

mod ai;
mod cluster;
mod discover;
mod fetch;
mod html;
mod markdown;
mod model;
mod normalize;
mod opml;
mod paper;
mod parse;
mod readable;
mod search;
mod store;
mod sync;

pub use ai::{AiFailure, Assistant, Excerpt, Pane, Step, ask, finish, interpret, prompt};
pub use cluster::{Signal, same_story, score, title_key, tokens};
pub use discover::{DirectorySource, DiscoveredFeed, directory, directory_sources, discover_in_html};
pub use fetch::{FetchBody, FetchOpts, FetchResult, get, ip_allowed, validate_resolved, validate_url};
pub use html::{decode_entities, excerpt, sanitize};
pub use markdown::article_markdown;
pub use model::{ArticleListItem, FeedRow, FolderRow, Ingest, NewFeed, ParsedFeed, ParsedItem, SmartFilter, View};
pub use normalize::canonical_url;
pub use opml::{Outline, export, import};
pub use paper::{Edition, lay_paper};
pub use parse::{parse_feed, upgrade_image_url};
pub use readable::{article_text, cache_key, extract_og_image, requires_publisher_site};
pub use search::{LibraryQuery, fts_match, library_query};
pub use store::{Db, DueFeed, open, open_readonly};
pub use sync::synchronize;
