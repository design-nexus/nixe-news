//! This app's own preferences (`~/.config/nixe-news/settings.toml`).

pub use nixe_theme::ThemeMode;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: ThemeMode,
    pub theme: String,
    pub reduce_motion: bool,
    pub sidebar_collapsed: bool,
    pub glow: bool,
    pub last_section: String,
    /// The window has opened on Headlines at least once.
    pub saw_headlines: bool,
    /// Mark an article read when it opens.
    pub mark_read_on_open: bool,
    /// Days to keep articles that are not starred, saved, or tagged.
    pub retention_days: i64,
    /// Keep sanitized feed HTML in the cache.
    pub keep_content: bool,
    /// `list` or `compact`.
    pub layout: String,
    /// Minutes between refreshes. The minimum the reader uses is 15.
    pub poll_minutes: i64,
    pub refresh_on_startup: bool,
    /// Assistant may add a feed. Off until the user turns it on.
    pub assistant_can_subscribe: bool,
    /// Show For You. Scoring stays on this computer and does not load a model.
    pub recommendations: bool,
    /// Ask Assistant for an original brief. Off until the user turns it on.
    pub summaries: bool,
    /// Desktop notification when a refresh finds new articles.
    pub notify_articles: bool,
    /// Desktop notification when a feed refresh fails.
    pub notify_failures: bool,
    /// Send folders, feeds, and read state to Nextcloud. The app password is not stored here.
    pub sync_enabled: bool,
    pub server: String,
    pub username: String,
    /// Folder inside the Nextcloud account. Empty means `Nixe/News`.
    pub folder: String,
    /// Disabled categories that shouldn't appear in sidebar or paper.
    pub disabled_categories: Vec<String>,
    /// Reader font scaling: "small", "medium", "large".
    pub reader_font_scale: String,
    /// Reader font family: "serif", "mono", "sans".
    pub reader_font: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Omarchy,
            theme: "tokyo-night".into(),
            reduce_motion: false,
            sidebar_collapsed: false,
            glow: true,
            last_section: "all".into(),
            saw_headlines: false,
            mark_read_on_open: true,
            retention_days: 90,
            keep_content: true,
            layout: "list".into(),
            poll_minutes: 60,
            refresh_on_startup: true,
            assistant_can_subscribe: false,
            recommendations: true,
            summaries: true,
            notify_articles: false,
            notify_failures: false,
            sync_enabled: false,
            server: String::new(),
            username: String::new(),
            folder: "Nixe/News".into(),
            disabled_categories: Vec::new(),
            reader_font_scale: "medium".into(),
            reader_font: "serif".into(),
        }
    }
}

nixe_core::prefs_store!(Prefs);
