//! Feed refresh off the GTK thread. One Tokio runtime owns the work. Results
//! hop back to the main loop with `idle_add_once`.

use crate::paths;
use crate::prefs;
use crate::window;
use nixe_news_core::{AiFailure, Excerpt, FetchOpts, FetchResult, Ingest, NewFeed, ask, canonical_url, get, open, parse_feed};
use nixe_sync::delay;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

enum Job {
    Add { url: String, category: Option<String> },
    Find(String),
    RefreshDue,
    RefreshOne(String),
    Sync { manual: bool },
    Brief { task: String, articles: Vec<Excerpt>, generation: u64 },
    FillPaper,
    OpenSection(String),
    Covers(Vec<String>),
    DiscoverImages(Vec<(String, String)>),
    Story { id: String, url: String },
    Shutdown,
}

static TX: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();

pub fn start() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    if std::env::var_os("NEWS_SNAPSHOT").is_none() {
        let _ = tx.send(Job::FillPaper);
    }
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_time().build().expect("news worker");
        runtime.block_on(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_sync: Option<std::time::Instant> = None;
            loop {
                tokio::select! {
                    job = rx.recv() => {
                        match job {
                            Some(Job::Shutdown) | None => break,
                            Some(Job::Add { url, category }) => add_feed(url, category).await,
                            Some(Job::Find(url)) => find_feeds_now(url).await,
                            Some(Job::RefreshDue) => refresh_due_now().await,
                            Some(Job::RefreshOne(id)) => refresh_id(id).await,
                            Some(Job::Sync { manual }) => sync_job(manual, &mut last_sync).await,
                            Some(Job::Brief { task, articles, generation }) => {
                                tokio::spawn(async move { run_brief(task, articles, generation).await });
                            }
                            Some(Job::FillPaper) => {
                                tokio::spawn(async move { fill_paper().await });
                            }
                            Some(Job::OpenSection(name)) => {
                                tokio::spawn(async move { open_section_now(name).await });
                            }
                            Some(Job::Covers(urls)) => {
                                tokio::spawn(async move { fetch_covers(urls).await });
                            }
                            Some(Job::DiscoverImages(items)) => {
                                tokio::spawn(async move { discover_images_job(items).await });
                            }
                            Some(Job::Story { id, url }) => {
                                tokio::spawn(async move { fetch_story(id, url).await });
                            }
                        }
                    }
                    _ = tick.tick() => {
                        refresh_due_now().await;
                        sync_job(false, &mut last_sync).await;
                    }
                }
            }
        });
    });
}

pub fn shutdown() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Shutdown);
    }
}

pub fn add_url(url: String) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Add { url, category: None });
    }
}

pub fn add_in_category(url: String, category: String) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Add { url, category: Some(category) });
    }
}

pub fn find_feeds(url: String) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Find(url));
    }
}

pub fn refresh_due() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::RefreshDue);
    }
}

pub fn sync_now() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Sync { manual: true });
    }
}

pub fn sync_background() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Sync { manual: false });
    }
}

pub fn brief(task: String, articles: Vec<Excerpt>, generation: u64) {
    if !prefs::get().summaries {
        return;
    }
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Brief { task, articles, generation });
    }
}

pub fn covers(urls: Vec<String>) {
    if urls.is_empty() {
        return;
    }
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Covers(urls));
    }
}

pub fn discover_images(items: Vec<(String, String)>) {
    if items.is_empty() {
        return;
    }
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::DiscoverImages(items));
    }
}

pub fn story(id: String, url: String) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::Story { id, url });
    }
}

pub fn open_section(name: String) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Job::OpenSection(name));
    }
}

pub fn refresh_current() {
    let id = match window::view() {
        nixe_news_core::View::Feed(id) => Some(id),
        _ => None,
    };
    if let Some(tx) = TX.get() {
        match id {
            Some(id) => {
                let _ = tx.send(Job::RefreshOne(id));
            }
            None => {
                let _ = tx.send(Job::RefreshDue);
            }
        }
    }
}

fn on_main(f: impl FnOnce() + Send + 'static) {
    gtk::glib::idle_add_once(f);
}

fn opts(url: &str) -> FetchOpts {
    FetchOpts {
        allow_http: url.trim().starts_with("http://"),
        allow_private: std::env::var_os("NIXE_NEWS_ALLOW_PRIVATE").is_some(),
        ..FetchOpts::default()
    }
}

async fn add_feed(url: String, category: Option<String>) {
    let result = tokio::task::spawn_blocking(move || ingest_new(&url, category.as_deref())).await;
    let message = match result {
        Ok(Ok(title)) => format!("Subscribed to {title}"),
        Ok(Err(err)) => err.to_string(),
        Err(err) => err.to_string(),
    };
    on_main(move || {
        window::toast(&message);
        window::reload();
        if timeline_open() {
            crate::timeline::refresh_discover();
        }
    });
}

fn timeline_open() -> bool {
    crate::timeline::discover_visible()
}

async fn find_feeds_now(url: String) {
    let result = tokio::task::spawn_blocking(move || lookup_feeds(&url)).await;
    match result {
        Ok(Ok(feeds)) => on_main(move || crate::discover_page::show_found(feeds)),
        Ok(Err(err)) => {
            let message = err.to_string();
            on_main(move || window::toast(&message));
        }
        Err(err) => {
            let message = err.to_string();
            on_main(move || window::toast(&message));
        }
    }
}

fn lookup_feeds(url: &str) -> anyhow::Result<Vec<nixe_news_core::DiscoveredFeed>> {
    let fetched = get(url, &opts(url))?;
    let nixe_news_core::FetchResult::Body(body) = fetched else { anyhow::bail!("the page was empty") };
    let html = String::from_utf8_lossy(&body.bytes);
    nixe_news_core::discover_in_html(&html, &body.final_url)
}

fn ingest_new(url: &str, category: Option<&str>) -> anyhow::Result<String> {
    let mut options = opts(url);
    let fetched = get(url, &options)?;
    let FetchResult::Body(body) = fetched else { anyhow::bail!("the server said the feed was unchanged") };
    options.allow_http = options.allow_http || body.final_url.starts_with("http://");
    let parsed = parse_feed(&body.bytes)?;
    let feed_url = canonical_url(&body.final_url).unwrap_or_else(|_| body.final_url.clone());
    let mut db = open(&paths::library_db())?;
    let title = if parsed.title.is_empty() { feed_url.clone() } else { parsed.title.clone() };
    let id =
        db.insert_feed(&NewFeed { url: feed_url, title: title.clone(), site_url: parsed.site_url.clone(), folder_id: None })?;
    db.set_poll_minutes(prefs::get().poll_minutes)?;
    if let Some(category) = category {
        db.attach_category(&id, category)?;
    }
    db.apply_feed(&id, &parsed, &ingest_opts(&body.etag, &body.last_modified))?;
    retain(&mut db)?;
    Ok(title)
}

fn ingest_opts(etag: &Option<String>, last_modified: &Option<String>) -> Ingest {
    let prefs = prefs::get();
    Ingest {
        etag: etag.clone(),
        last_modified: last_modified.clone(),
        keep_content: prefs.keep_content,
        cache_dir: Some(paths::content_cache()),
    }
}

async fn fill_paper() {
    bring_in(None).await;
}

async fn open_section_now(name: String) {
    bring_in(Some(name)).await;
    on_main(window::reload);
}

async fn bring_in(section: Option<String>) {
    let wanted = nixe_news_core::directory_sources()
        .into_iter()
        .filter(|source| section.as_ref().is_none_or(|name| source.section == *name))
        .map(|source| (source.url, source.section))
        .collect::<Vec<_>>();
    let mut added = 0;
    for (url, section) in wanted {
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
            let db = open(&paths::library_db())?;
            if db.feeds()?.iter().any(|feed| feed.url == url) {
                return Ok(None);
            }
            ingest_new(&url, Some(&section)).map(Some)
        })
        .await;
        if matches!(result, Ok(Ok(Some(_)))) {
            added += 1;
            on_main(window::reload);
        }
    }
    if added > 0 {
        on_main(window::refresh_lists);
    }
}

fn cover_opts(url: &str) -> FetchOpts {
    FetchOpts {
        allow_http: url.trim().starts_with("http://"),
        allow_private: std::env::var_os("NIXE_NEWS_ALLOW_PRIVATE").is_some(),
        allow_host_change: true,
        max_bytes: 10 * 1024 * 1024,
        timeout: std::time::Duration::from_secs(12),
        user_agent: Some(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into(),
        ),
        ..FetchOpts::default()
    }
}

async fn fetch_covers(urls: Vec<String>) {
    let mut set = tokio::task::JoinSet::new();
    for url in urls {
        let path = paths::cover_file(&url);
        if path.exists() {
            continue;
        }
        set.spawn(async move {
            let u = url.clone();
            let fetched = tokio::task::spawn_blocking(move || get(&u, &cover_opts(&u))).await;
            if let Ok(Ok(FetchResult::Body(body))) = fetched {
                if is_image(&body.bytes) {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if std::fs::write(&path, &body.bytes).is_ok() {
                        return true;
                    }
                }
            }
            false
        });
    }
    let mut any_saved = false;
    while let Some(res) = set.join_next().await {
        if let Ok(true) = res {
            any_saved = true;
        }
    }
    if any_saved {
        on_main(window::reload);
    }
}

fn story_opts(url: &str) -> FetchOpts {
    FetchOpts {
        allow_http: url.trim().starts_with("http://"),
        allow_private: std::env::var_os("NIXE_NEWS_ALLOW_PRIVATE").is_some(),
        allow_host_change: true,
        max_bytes: 4 * 1024 * 1024,
        user_agent: Some(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into(),
        ),
        ..FetchOpts::default()
    }
}

async fn fetch_story(id: String, url: String) {
    let path = paths::story_file(&id);
    if let Ok(text) = std::fs::read_to_string(&path)
        && text.len() >= 400
    {
        on_main(move || crate::reader::replace_body(&id, &text));
        return;
    }
    let fetched = tokio::task::spawn_blocking(move || get(&url, &story_opts(&url))).await;
    let Ok(Ok(FetchResult::Body(body))) = fetched else {
        if let Some(parent) = paths::unavailable_file(&id).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(paths::unavailable_file(&id), b"");
        on_main(move || crate::reader::unavailable(&id));
        return;
    };
    let html = String::from_utf8_lossy(&body.bytes);
    if let Some(og_image) = nixe_news_core::extract_og_image(&html) {
        let upgraded = nixe_news_core::upgrade_image_url(&og_image);
        let id_c = id.clone();
        let og_c = upgraded.clone();
        on_main(move || {
            let _ = window::with_db(|db| db.set_article_image(&id_c, &og_c));
        });
        tokio::spawn(async move { fetch_covers(vec![upgraded]).await; });
    }
    let text = nixe_news_core::article_text(&html);
    if text.len() < 400 {
        if let Some(parent) = paths::unavailable_file(&id).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(paths::unavailable_file(&id), b"");
        on_main(move || crate::reader::unavailable(&id));
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &text);
    on_main(move || crate::reader::replace_body(&id, &text));
}

async fn discover_images_job(items: Vec<(String, String)>) {
    let mut discovered = Vec::new();
    let mut set = tokio::task::JoinSet::new();
    for (id, url) in items {
        if paths::no_image_file(&id).exists() {
            continue;
        }
        set.spawn(async move {
            let mut opts = story_opts(&url);
            opts.max_bytes = 64 * 1024;
            let fetched = tokio::task::spawn_blocking(move || get(&url, &opts)).await;
            if let Ok(Ok(FetchResult::Body(body))) = fetched {
                let html = String::from_utf8_lossy(&body.bytes);
                if let Some(img) = nixe_news_core::extract_og_image(&html) {
                    let upgraded = nixe_news_core::upgrade_image_url(&img);
                    return Some((id, upgraded));
                }
            }
            if let Some(parent) = paths::no_image_file(&id).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(paths::no_image_file(&id), b"");
            None
        });
    }
    while let Some(res) = set.join_next().await {
        if let Ok(Some((id, img_url))) = res {
            discovered.push((id, img_url));
        }
    }
    if !discovered.is_empty() {
        let disc = discovered.clone();
        on_main(move || {
            let _ = window::with_db(|db| {
                for (id, img) in &disc {
                    let _ = db.set_article_image(id, img);
                }
                Ok(())
            });
        });
        let urls: Vec<String> = discovered.into_iter().map(|(_, u)| u).collect();
        fetch_covers(urls).await;
    }
}

fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8])
        || bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47])
        || bytes.starts_with(b"GIF8")
        || (bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

async fn refresh_due_now() {
    let due = tokio::task::spawn_blocking(|| {
        let db = open(&paths::library_db())?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        db.due_feeds(now)
    })
    .await;
    let Ok(Ok(due)) = due else { return };
    let mut pending = Vec::new();
    for feed in due.into_iter().take(8) {
        pending.push(tokio::task::spawn_blocking(move || refresh_feed(feed)));
    }
    let mut notes = Vec::new();
    for job in pending {
        if let Ok(Ok(note)) = job.await {
            notes.push(note);
        }
    }
    on_main(move || {
        report_refresh(&notes);
        window::reload();
    });
}

async fn refresh_id(id: String) {
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<RefreshNote>> {
        let db = open(&paths::library_db())?;
        let Some(row) = db.feed(&id)? else { return Ok(None) };
        let (etag, last_modified) = db.validators(&id)?;
        Ok(Some(refresh_feed(nixe_news_core::DueFeed {
            id: row.id,
            url: row.url,
            etag,
            last_modified,
            consecutive_failures: row.consecutive_failures,
        })?))
    })
    .await;
    match result {
        Ok(Ok(Some(note))) => on_main(move || {
            report_refresh(std::slice::from_ref(&note));
            window::reload();
        }),
        Ok(Ok(None)) => on_main(window::reload),
        Ok(Err(err)) => {
            let message = err.to_string();
            on_main(move || {
                window::toast(&message);
                window::reload();
            });
        }
        Err(_) => on_main(window::reload),
    }
}

struct RefreshNote {
    added: i64,
    feed: String,
    error: Option<String>,
}

fn report_refresh(notes: &[RefreshNote]) {
    let added: i64 = notes.iter().map(|note| note.added).sum();
    if prefs::get().notify_articles && added > 0 {
        let message = if added == 1 {
            notes
                .iter()
                .find(|note| note.added > 0)
                .map(|note| format!("1 new article in {}", note.feed))
                .unwrap_or_else(|| "1 new article".into())
        } else {
            format!("{added} new articles")
        };
        window::notify("News", &message);
    }
    if prefs::get().notify_failures {
        let failed: Vec<_> = notes.iter().filter(|note| note.error.is_some()).map(|note| note.feed.as_str()).collect();
        if failed.len() == 1 {
            window::notify("News", &format!("{} could not be refreshed.", failed[0]));
        } else if failed.len() > 1 {
            window::notify("News", &format!("{} feeds could not be refreshed.", failed.len()));
        }
    }
}

fn refresh_feed(feed: nixe_news_core::DueFeed) -> anyhow::Result<RefreshNote> {
    let mut db = open(&paths::library_db())?;
    let feed_title = db.feed(&feed.id)?.map(|row| row.title).unwrap_or_else(|| feed.url.clone());
    let before = db.article_count()?;
    let mut options = opts(&feed.url);
    options.etag = feed.etag.clone();
    options.last_modified = feed.last_modified.clone();
    let mut error = None;
    match get(&feed.url, &options) {
        Ok(FetchResult::NotModified { .. }) => {
            db.mark_not_modified(&feed.id)?;
        }
        Ok(FetchResult::Body(body)) => match parse_feed(&body.bytes) {
            Ok(parsed) => {
                db.apply_feed(&feed.id, &parsed, &ingest_opts(&body.etag, &body.last_modified))?;
                retain(&mut db)?;
            }
            Err(err) => {
                note_failure(&mut db, &feed, &err.to_string())?;
                error = Some(err.to_string());
            }
        },
        Err(err) => {
            note_failure(&mut db, &feed, &err.to_string())?;
            error = Some(err.to_string());
        }
    }
    let added = db.article_count()?.saturating_sub(before);
    Ok(RefreshNote { added, feed: feed_title, error })
}

fn note_failure(db: &mut nixe_news_core::Db, feed: &nixe_news_core::DueFeed, message: &str) -> anyhow::Result<()> {
    let wait = delay((feed.consecutive_failures as u32).saturating_add(1), None).wait;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    db.mark_failure(&feed.id, message, now + wait.as_secs() as i64)?;
    Ok(())
}

async fn sync_job(manual: bool, last: &mut Option<std::time::Instant>) {
    if !manual {
        if !prefs::get().sync_enabled {
            return;
        }
        if last.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(15 * 60)) {
            return;
        }
    }
    *last = Some(std::time::Instant::now());
    let result = tokio::task::spawn_blocking(run_sync).await;
    match result {
        Ok(Ok(fields)) => on_main(move || window::apply_synced_prefs(fields)),
        Ok(Err(err)) if manual => {
            let message = sync_error(&err.to_string());
            on_main(move || window::toast(&message));
        }
        Err(err) if manual => {
            let message = sync_error(&err.to_string());
            on_main(move || window::toast(&message));
        }
        _ => {}
    }
}

fn run_sync() -> anyhow::Result<std::collections::BTreeMap<String, String>> {
    let prefs = prefs::get();
    if prefs.server.trim().is_empty() || prefs.username.trim().is_empty() {
        anyhow::bail!("Add the Nextcloud server and username first.");
    }
    let mut db = open(&paths::library_db())?;
    let device = db.device_id()?;
    let password = nixe_sync::lookup_secret("nixe-news", &device, "nextcloud")?;
    let folder = if prefs.folder.trim().is_empty() { "Nixe/News".to_string() } else { prefs.folder.clone() };
    let remote = nixe_sync::WebDav::new(nixe_sync::WebDavConfig {
        server: prefs.server.clone(),
        username: prefs.username.clone(),
        password,
        folder,
        allow_insecure_loopback: false,
        recipient: None,
        identity: None,
    })?;
    let retention = prefs.retention_days.to_string();
    let layout = prefs.layout.clone();
    let poll = prefs.poll_minutes.to_string();
    let mark = if prefs.mark_read_on_open { "true" } else { "false" };
    let recommendations = if prefs.recommendations { "true" } else { "false" };
    let summaries = if prefs.summaries { "true" } else { "false" };
    nixe_news_core::synchronize(
        &mut db,
        &remote,
        &[
            ("retention_days", retention.as_str()),
            ("layout", layout.as_str()),
            ("poll_minutes", poll.as_str()),
            ("mark_read_on_open", mark),
            ("recommendations", recommendations),
            ("summaries", summaries),
        ],
    )
}

async fn run_brief(task: String, articles: Vec<Excerpt>, generation: u64) {
    if !prefs::get().summaries {
        return;
    }
    let result = tokio::task::spawn_blocking(move || {
        let mut client = crate::assistant_client::Client;
        ask(&mut client, &task, &articles)
    })
    .await;
    let result = match result {
        Ok(result) => result,
        Err(_) => Err(AiFailure::Failed("The summary stopped.".into())),
    };
    if generation == 9999 {
        on_main(move || crate::timeline::apply_daily_brief(result));
    } else {
        on_main(move || crate::reader::apply_brief(generation, result));
    }
}

fn sync_error(err: &str) -> String {
    let line = err.lines().next().unwrap_or("Sync failed.");
    if line.len() > 160 { "Sync failed.".to_string() } else { line.to_string() }
}

fn retain(db: &mut nixe_news_core::Db) -> anyhow::Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    db.retain(prefs::get().retention_days, now)?;
    Ok(())
}
