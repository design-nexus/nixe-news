//! Well-known locations. Every path honours the XDG overrides.

pub use nixe_core::paths::*;

use std::path::PathBuf;

pub fn library_db() -> PathBuf {
    data_dir().join("news.sqlite")
}

pub fn content_cache() -> PathBuf {
    cache_dir().join("content")
}

pub fn cover_file(url: &str) -> PathBuf {
    cache_dir().join("covers").join(nixe_news_core::cache_key(url))
}

pub fn story_file(id: &str) -> PathBuf {
    cache_dir().join("stories").join(format!("{id}.txt"))
}

pub fn unavailable_file(id: &str) -> PathBuf {
    cache_dir().join("unavailable").join(format!("{id}.flag"))
}

pub fn no_image_file(id: &str) -> PathBuf {
    cache_dir().join("no_image").join(format!("{id}.flag"))
}
