# Nixe News

Native RSS reader and news browser for the Nixe suite. Repository is the existing workspace `design-nexus/nixe` at `/home/ken/Projects/nixe`. News is a new app in that workspace, not a new repository and not a separate service.

Phase 1 is the only implementation this plan authorizes. Later phases are specified so the schema and crate boundaries do not have to be rewritten, and they stay unbuilt until the previous phase is accepted.

## Decisions from the existing tree

`STYLE.md` is the visual contract. Music is the sidebar reference. Settings is the settings-card reference. Shared behavior comes from `nixe-core`, `nixe-theme`, `nixe-ui`, and `nixe-icons`. Every current app is GTK 4 (`gtk4` 0.11, feature `v4_18`) with no libadwaita and no WebKit. News follows that. Adwaita widgets and a web shell would replace the flat monospace surface the suite already shares.

There is no shared cloud-account service. Settings → Accounts is the operating-system user. Git and Theater keep provider secrets in the desktop Secret Service through `secret-tool`, with attributes and a redacted `Secret` type (`apps/git/src/providers/credentials.rs`). News uses that same shape, moved into `nixe-sync` so later apps can call it. Passwords, URLs of tokens, and token material stay out of `settings.toml`, SQLite, logs, and the command line.

Assistant integration today is per-app D-Bus, not a shared model loader. `nixe_core::control` registers `io.github.design_nexus.nixe.Control` with `Describe` and `Call` on the app’s own bus name. Handlers run on the GTK thread, so News answers them by scheduling work and replying later (`serve_async`), the way Music does. Assistant’s own IPC `Say` speaks through TTS. It is not a silent completion API. Notes and Reminders are Assistant abilities with private SQLite files, not desktop apps.

Glimpse plugins are sandboxed JSON-lines processes (`apps/glimpse/PLUGIN_API.md`, `crates/glimpse-plugin-sdk`). They search and return plain text. They do not embed GTK.

SQLite in this repo is `rusqlite` 0.40 with `bundled`. Weather’s HTTP client is blocking `reqwest` 0.12 with `rustls-tls`. Music’s podcast client is blocking `ureq` plus a small `roxmltree` RSS parser aimed at audio enclosures. That parser is not a general news reader and stays in Music.

Theme, density, text size, and “open on” already live in `~/.config/nixe/theme.toml` and follow every open app. News does not invent a second theme store.

## What the app is

| | |
| --- | --- |
| Package | `nixe-news` |
| Binary | `nixe-news` |
| App id | `io.github.design_nexus.nixe.News` |
| Slug | `nixe-news` via `nixe_core::app::init` |
| Config | `~/.config/nixe-news/settings.toml` |
| Library | `~/.local/share/nixe-news/news.sqlite` |
| Cache | `~/.cache/nixe-news/` (bodies, images, HTTP validators) |
| Window | Music’s frame: sidebar head, 250 px navigation, toolbar, virtualized list, status bar, settings card. Default 1120×800. Tiled, not floating. |

Sidebar groups, in order:

- **Discover** (Phase 3; the page exists as an empty state in Phase 1)
- **For You** (Phase 3; hidden until recommendations are enabled)
- **Following**: All, Unread, Starred, Saved, then folders, then feeds
- A flat `+` on Following adds a feed URL

Settings is the gear card, never a sidebar row. Pages have no titles. The window title is `News — All` (or the folder or feed name). The status bar holds `F1 Shortcuts`, the unread count, sync state once it exists, and the gear.

Layouts in Phase 1: list and compact. Magazine waits until images and excerpts are trustworthy.

Core reading works with recommendations off, AI off, sync off, and no network after the last successful refresh.

## Crate layout

```
crates/nixe-sync/            # no GTK, no news types
crates/nixe-news-core/       # no GTK
apps/news/                   # GTK shell
```

`nixe-sync` is a workspace crate, same as `nixe-weather-core` and `nixe-media-cache`. A second repository would duplicate edition, license, and CI for a library only Nixe apps link. Its public surface is records, a journal, merge policies, credentials, and a provider trait. News maps its rows onto that surface in `nixe-news-core`. Notes and Reminders can do the same later without linking the news crate.

### `nixe-sync`

| Module | Role |
| --- | --- |
| `id` | Stable record ids, device ids (UUID v4, persisted) |
| `hlc` | Hybrid logical clock: device id, counter, wall millis. Wall time is display-only |
| `journal` | Append-only local ops: upsert fields, add/remove set members, tombstone |
| `merge` | Pure functions. Same inputs, same output |
| `checkpoint` | Last applied remote generation, pending local ops, attempt count |
| `provider` | `SyncProvider` trait |
| `webdav` | Nextcloud WebDAV adapter |
| `credentials` | `secret-tool` store/lookup/clear, redacted secret |
| `crypto` | Optional `age` encryption of batch bodies |
| `backoff` | Exponential backoff with jitter, honors `Retry-After` |

`SyncProvider` is async. The GTK app owns one Tokio runtime on a background thread and never awaits on the main loop. The trait does not mention RSS, articles, or Nextcloud.

```rust
trait SyncProvider {
    async fn load_manifest(&self) -> Result<Option<Manifest>>;
    async fn store_manifest(&self, manifest: &Manifest, if_match: Option<&Etag>) -> Result<Etag>;
    async fn get_batch(&self, id: &BatchId) -> Result<Batch>;
    async fn put_batch(&self, batch: &Batch) -> Result<()>;
}
```

A failed call returns `Err`. It does not write a partial manifest. Callers apply remote batches inside one SQLite transaction and advance the checkpoint in that same transaction.

### `nixe-news-core`

| Module | Phase | Role |
| --- | --- | --- |
| `db` | 1 | Open WAL database, migrations, transactions |
| `model` | 1 | Feeds, folders, articles, user state |
| `normalize` | 1 | Canonical article URLs, feed URL identity |
| `fetch` | 1 | HTTP with timeouts, size caps, SSRF checks, ETag / Last-Modified |
| `parse` | 1 | `feed-rs` → model |
| `html` | 1 | `ammonia` allowlist, plain-text excerpt |
| `opml` | 1 | Import and export |
| `search` | 1 | FTS5 queries |
| `refresh` | 1 | Bounded scheduler, per-host backoff |
| `syncmap` | 2 | Which rows become journal records |
| `discover` | 3 | HTML feed autodiscovery, bundled directory |
| `cluster` | 3 | Title/time grouping, no model |
| `ai` | 4 | Optional Assistant adapter |

### `apps/news`

Match the app pattern in `CLAUDE.md`: `paths.rs`, `cmd.rs`, `prefs.rs`, `theme.rs`, and `widgets.rs` re-export the shared crates. Local modules: `window.rs`, `sidebar.rs`, `timeline.rs`, `reader.rs`, `feeds.rs`, `settings_dialog.rs`, `control.rs`, `worker.rs`. `worker.rs` is the only place that starts Tokio and crosses back to GTK with `glib::idle_add_local_once` or a channel drained on the main context.

`install.sh` gains a `news` arm (`ids="News"`, dependency `libsecret`) and `news` in `ALL`. `nixe-icons/make_icons.py` gains one glyph. README’s app table gains one row. `apps/news/CLAUDE.md` records the bus name, database path, and test environment variables.

## Libraries

Pinned to versions already used in the workspace where they exist. New crates are MIT or Apache-2.0, maintained, and free of a private news API.

| Need | Choice | Why |
| --- | --- | --- |
| UI | `gtk4` 0.11, `v4_18` | Suite standard |
| Async | `tokio` runtime owned by `worker.rs` | Cancellation, bounded concurrency, timeouts off the main thread |
| HTTP | `reqwest` 0.12, `rustls-tls`, no native-tls | Already in `nixe-weather-core`. No OpenSSL |
| Feeds | `feed-rs` 2.4 | RSS 2.0, RSS 1.0, Atom, JSON Feed. Released July 2026. 3.0 (September 2026) changes the model; stay on 2.4 until a separate upgrade |
| HTML | `ammonia` 4.2 | Allowlist sanitizer. Released October 2026. Used directly so the allowlist is ours. `feed-rs`’s `sanitize` feature stays off |
| Autodiscovery | `scraper` | `<link rel="alternate">` only, Phase 3 |
| SQL | `rusqlite` 0.40 `bundled` | FTS5 comes with the bundled SQLite this repo already ships. Migration test creates the virtual table |
| Ids | `uuid` | v4 record ids |
| URLs | `url` | Parse and normalize |
| Secrets | `secret-tool` on PATH | Same as Git and Theater. `libsecret` is the Arch dependency |
| Encryption | `age` | Optional, Phase 2. Established format, no custom cipher |
| OPML | `roxmltree` | Already in Music. OPML is small and tested with fixtures. No new XML stack |
| Readability | none in Phase 1 | Full-text extraction is user-initiated and later. Re-evaluate a maintained extractor then. Music’s RSS parser is not reused |

Rejected for the core app: Electron, Tauri, a WebKit document view, libadwaita, a Nixe-operated directory server, loading Gemma or Kokoro inside `nixe-news`, and storing Nextcloud passwords in TOML.

## Data model

One user database, `news.sqlite`, `journal_mode=WAL`, `foreign_keys=ON`. `PRAGMA user_version` starts at 1. Each migration is a function in a list, applied inside a transaction, then the version is set. A failing migration rolls back. Tests open a temp file and walk every version.

Cache files live under `~/.cache/nixe-news/` and are disposable. Deleting the cache loses images and extracted HTML. It does not lose subscriptions, read state, stars, or tags.

Article identity is `(feed_id, item_key)`. `item_key` is the feed entry id when the feed provides one, otherwise the canonical URL. Canonical URL: `http`/`https` only, lowercase host, default ports removed, fragment removed, a fixed list of tracking parameters removed (`utm_*`, `fbclid`, `mc_cid`, `mc_eid`). The original URL is kept for the source link.

User state is a different row from article content. A refresh may replace title, summary, and content hash. It must not reset `read`, `starred`, `saved`, `archived`, `progress`, or tags.

```sql
-- user_version = 1
CREATE TABLE folders (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  parent_id TEXT REFERENCES folders(id) ON DELETE CASCADE,
  position INTEGER NOT NULL,
  deleted_at INTEGER
);

CREATE TABLE feeds (
  id TEXT PRIMARY KEY,
  url TEXT NOT NULL UNIQUE,          -- canonical feed URL
  title TEXT NOT NULL DEFAULT '',
  site_url TEXT NOT NULL DEFAULT '',
  folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
  position INTEGER NOT NULL DEFAULT 0,
  poll_minutes INTEGER NOT NULL DEFAULT 60,
  etag TEXT,
  last_modified TEXT,
  last_ok_at INTEGER,
  last_attempt_at INTEGER,           -- set on every fetch attempt, including failures
  backoff_until INTEGER,             -- earliest unix time the worker may try again
  last_error TEXT,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0,
  deleted_at INTEGER
);

CREATE TABLE articles (
  id TEXT PRIMARY KEY,
  feed_id TEXT NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  item_key TEXT NOT NULL,
  url TEXT NOT NULL,                 -- canonical
  original_url TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  author TEXT NOT NULL DEFAULT '',
  summary TEXT NOT NULL DEFAULT '',  -- feed excerpt, plain text
  content_hash TEXT,                 -- hash of feed-provided body, not the body
  published_at INTEGER,
  updated_at INTEGER,
  image_url TEXT,
  UNIQUE (feed_id, item_key)
);

CREATE TABLE article_state (
  article_id TEXT PRIMARY KEY REFERENCES articles(id) ON DELETE CASCADE,
  read_at INTEGER,                   -- NULL = unread
  starred INTEGER NOT NULL DEFAULT 0,
  saved INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  progress INTEGER NOT NULL DEFAULT 0,  -- 0–100, reader scroll
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

-- Phase 3 tables are created in a later migration, empty until then:
-- categories, feed_categories, interests, smart_filters, clusters, cluster_members

CREATE VIRTUAL TABLE articles_fts USING fts5(
  title, summary, author, url UNINDEXED,
  content='articles', content_rowid='rowid', tokenize='unicode61'
);

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
  collection TEXT NOT NULL,          -- feeds, folders, article_state, tags, ...
  record_id TEXT NOT NULL,
  op TEXT NOT NULL,                  -- upsert, tombstone, set_add, set_remove
  fields TEXT NOT NULL,              -- JSON object, no article bodies
  applied INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE sync_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

Indexes: `articles(feed_id, published_at DESC)`, `articles(url)`, `article_state(read_at)`, `feeds(folder_id)`, `change_log(applied, seq)`.

FTS stays in sync with triggers on `articles`, the same pattern Glimpse uses for its segment index. Search queries `articles_fts` and joins `article_state`. The query string is passed as an FTS5 expression only after a small builder quotes each token. User text is never concatenated into SQL.

Retention is a preference: keep articles for N days (default 90) unless starred, saved, or tagged. Cleanup runs after a refresh, in a transaction, and writes tombstones only for synced user-state rows that were deleted. Feed-provided article rows that fall out of retention are local cache, not sync events, unless the user had state on them.

Content the feed includes (`content:encoded`, Atom content, JSON Feed `content_html` / `content_text`) is stored as sanitized HTML in the cache directory, keyed by `content_hash`, only when the user turns on “Keep feed content on disk” (default on, size-capped). It is not a column that sync reads. When the preference is off, the reader shows title, summary, and the source link.

### Sync payload (Phase 2)

Synchronized collections: folders, feeds (url, title, folder, position, poll, disabled, tombstone), article_state for saved/starred/read/progress, tags, article_tags, followed categories, smart filters, and an allowlist of preferences (retention days, layout, refresh interval, AI enabled, recommendations enabled).

Not synchronized: article bodies, images, embeddings, HTTP validators, last error strings, window geometry, “follow Omarchy”, or anything already in `~/.config/nixe/theme.toml`.

### Remote WebDAV layout

Under a configurable folder, default `/Nixe/News/`:

```
manifest.json
batches/<generation>.json
```

`manifest.json` holds schema version, generation, device list, and the id of the newest batch. Batch files are immutable after a successful PUT. The manifest PUT sends `If-Match` with the ETag from the last GET. A `412` means another device won. The client downloads the new manifest and missing batches, merges, and retries its own batch. File modification times are not read and not written as the version.

Batch body:

```json
{
  "schema": 1,
  "id": "batch-uuid",
  "device_id": "device-uuid",
  "base_generation": 12,
  "ops": [
    {"hlc": "…", "collection": "feeds", "id": "…", "op": "upsert", "fields": {"url": "https://example.com/feed.xml", "title": "Example"}}
  ]
}
```

Optional encryption wraps the batch bytes with `age` to a recipient whose identity key is in Secret Service. The manifest stays structural (generation, batch id, device id) and contains no feed URLs or titles. Encryption is off until the user creates a sync key.

### Merge policies

| Record | Policy |
| --- | --- |
| Folder, feed metadata, preferences | Per-field last-writer by HLC |
| Read, star, save, archive, progress | Per-field last-writer by HLC |
| Tags on an article | `set_add` / `set_remove` ops, not a replaced array |
| Delete | Tombstone with HLC. A later upsert with a greater HLC restores the record |
| Equal HLC | Device id byte order is the tie-break, so two devices converge |

Apply order is HLC, not arrival order. The local database changes only inside the transaction that also records the new checkpoint. If the process dies mid-sync, WAL recovery keeps the previous checkpoint and the client retries. Remote batches that were PUT and then failed to update the manifest are immutable and unused until a later manifest names them. Orphan batches are ignored.

Nextcloud authentication is an app password (Nextcloud → Settings → Security → Devices & sessions) plus the server URL and username. The password is stored with `secret-tool` attributes `application=nixe-news`, `kind=nextcloud`, `account=<uuid>`. Basic auth over HTTPS only. HTTP URLs are rejected. Redirects that leave the configured origin are rejected.

## Fetch and safety

Feed and article content is untrusted.

- Schemes: `https` by default. `http` is allowed only when the user adds that exact feed.
- Block non-network schemes (`file`, `data`, `javascript`, `gopher`, `ftp`).
- Before the connection, and again on every redirect, resolve the host and refuse loopback, unspecified, link-local, private, multicast, and documentation ranges. Refuse a redirect that changes host unless the user added that feed URL as the final URL.
- Connect timeout 10 s, total timeout 20 s, at most 5 redirects.
- Feed body cap 2 MiB. Image cap 1 MiB. Response `Content-Length` above the cap is aborted.
- Global in-flight requests: 4. Per host: 2.
- Conditional GET with stored ETag and Last-Modified. `304` updates `last_ok_at` and does not rewrite articles.
- `429` and `503` honor `Retry-After` up to 24 h. Other 5xx and network errors use exponential backoff capped at 6 h. The feed’s base interval stays at the user’s poll minutes (default 60, minimum 15).
- Failures set `last_error` and leave existing articles in place.
- HTML rendered in the reader has passed `ammonia` with `p, br, h1, h2, h3, ul, ol, li, blockquote, pre, code, strong, em, a[href], img[src]`. `href` and `src` must be `https`. No style attributes, no SVG, no script, no iframes. The reader is a GTK text layout with links, not WebKit, so script execution has no engine. Images load from the cache after a separate guarded fetch.
- AI prompts, when they exist, receive the user’s summary request plus a short excerpt. The adapter’s instructions say to write an original brief and to include the source URL. They are not given a goal of reproducing the article.

Copyright behavior, present from Phase 1:

- Every article row shows the feed title and opens the original URL.
- The reader shows the feed’s own summary or content. It does not fetch the article page in the background.
- “Load full article” is a later, explicit action, stored only in the local cache, and off in Phase 1.
- No shared article host, no account that uploads bodies, no paywall cookies, no bypass.

## UI behavior (Phase 1)

Following is the start page (`nixe_theme::start_page`).

- Toolbar: filter field when the page is a list, layout segment (List / Compact), Mark all read.
- List: `GtkListView` + `SignalListItemFactory` over a `gio::ListStore` filled from a paged SQL query (about 200 rows, extended as the user scrolls). Rows are 35 px in compact, taller in list (title, feed name, time, unread dot). Selection is `@nav_active`.
- Reader: a second column when the window is at least 980 px, otherwise a page pushed on the stack. Esc returns to the list. Opening an article sets `read_at` unless the user has “mark read on scroll” off (default: mark read on open). `m` toggles read, `s` stars, `b` saves.
- Keyboard: the shared set from `nixe_ui::keys`, plus `j`/`k` or Down/Up to move, Enter to open, `r` to refresh the current feed, `Ctrl+N` to add a feed, `Ctrl+I` OPML import, `Ctrl+E` OPML export.
- Add feed: dialog with a URL entry. The worker fetches and parses. Success asks for a folder and inserts the feed. Failure is a toast with the parser or HTTP error.
- OPML import merges by canonical feed URL. Existing feeds are not duplicated. Export writes the current folders and feeds.
- Search (`Ctrl+F`) filters the open list through FTS when the query is non-empty, and by title substring when FTS would reject the token.
- Empty states: no feeds (“Add a feed”), feed with no items, offline with a last-success time in the status bar.
- Refresh on startup if the last success is older than the poll interval, then a worker sleep loop. Quitting stops the runtime. No user systemd unit in Phase 1.
- Invalid feeds stay in the list with the error in `dim_text`. They do not block other feeds.

Discover in Phase 1 is a page that says discovery arrives later and offers the add-feed action. For You is absent until a preference turns it on.

Settings card groups:

- **News window** — shared theme rows, glow, reduce motion, open on, sidebar.
- **Reading** — mark read on open, retention days, keep feed content, layout.
- **Refresh** — poll minutes, refresh on startup.
- **Sync** — present but disabled, “Available in a later version”, so the group id is stable.
- **Intelligence** — recommendations off, summaries off, both disabled until those phases. Default off.

Shortcuts dialog follows Fonts’ `show_shortcuts` structure and one `SHORTCUTS` list.

## Assistant, Glimpse, Notes, Reminders

Phase 1 still registers control tools, narrow and read-mostly, so the contract exists while the window does:

- `news.search` `{query}` → titles, feeds, urls, unread flag. Limit 20.
- `news.headlines` `{feed?}` → unread titles.
- `news.add_feed` `{url}` requires the app to be open and is refused when a preference `assistant_can_subscribe` is false (default false).
- `news.mark_read` `{id}` is allowed.

No tool deletes a feed or exports the database. `Describe` documents each tool in the JSON Music and Contacts already return.

Phase 5 adds a Glimpse plugin (`capabilities = ["search", "preview"]`, `network = false`) that opens `news.sqlite` read-only and answers `news`, `news unread`, `news saved`, and free text. Preview is plain title, feed, and summary.

Phase 5 Notes export writes a Markdown file through the save path the user picks (`nixe_core::control::save_target` when Assistant asks). It does not write the Assistant notes plugin’s private SQLite. A “Create note” action can call Assistant only after a silent completion/tool IPC exists.

Phase 4 AI calls a new Assistant IPC command that returns text and does not speak, with a timeout and a hard failure that leaves the article on screen. Audio briefings send the finished script to Assistant’s existing speech path. News never links `llama.cpp` or `loqui`. If the daemon is down, the action toasts “Assistant isn’t running” and returns.

Phase 6 news watches can call the existing Reminders ability. They do not add a second scheduler in the news process.

## Performance

- Startup reads SQLite and shows the last timeline. Network starts after the first frame.
- The list factory binds only visible rows.
- Images decode on the worker and only for rows that were bound.
- Cache budget is a preference, default 200 MB, enforced by oldest-file deletion after refresh.
- Idle CPU is the theme poll `nixe-theme` already runs, plus the refresh loop sleeping on a timer. No model load, no embedding index, no animation tick.
- SQLite writes are batched per feed inside one transaction.

## Testing

Unit tests live next to the core crates and use temp directories. `nixe_core::app` already falls back to the `nixe-test` identity when `init` was not called.

Phase 1 fixtures under `crates/nixe-news-core/tests/fixtures/`:

- Valid RSS 2.0, Atom, and JSON Feed.
- Malformed XML, truncated JSON, a feed larger than the cap, a feed with no entries, duplicate guids, missing guids.
- HTML with script, `javascript:` links, and comments. Expect them stripped.
- URL normalization and private-address refusal, including a redirect fixture.
- Migration from an empty file, and FTS returning a known title.
- OPML round-trip and import that does not duplicate.
- Retention that keeps starred items.
- A refresh that replaces a title and preserves `read_at`.

Phase 2 adds merge tests: offline ops, two devices editing one feed, a tombstone against a later edit, a manifest `412`, and a crash between batch apply and checkpoint (kill the process around a test hook, reopen, assert the database matches the last checkpoint). WebDAV tests use a local stub that speaks the verbs the adapter uses. A disposable Nextcloud container is documented and gated by `NIXE_NEWS_WEBDAV_URL`. It is not required for `cargo test --workspace`.

Phase 4 tests the AI adapter with a fake client that errors, times out, and returns text. The article pane stays usable in all three.

Commands:

```sh
cargo test -p nixe-news-core -p nixe-sync
cargo test -p nixe-news
cargo clippy -p nixe-news -p nixe-news-core -p nixe-sync --all-targets -- -D warnings
```

Format only the new crates. The workspace is not rustfmt-clean as a whole.

Visual check before calling Phase 1 done: run under `dbus-run-session` with scratch `XDG_*` homes, at the default size and at 800 px. Confirm the sidebar stays 250 px (icons at 800 px), rows scroll without loading every article as a widget, add-feed and OPML work offline against fixtures, and an unreachable feed toasts without freezing the window.

## Phases and acceptance

### Phase 1 — Foundation and RSS

Build the crates, the app shell, schema v1, fetch, parse, timeline, reader, read/unread/star/save, folders, OPML, FTS, retention, and the control methods above.

Accepted when:

- `nixe-news` opens on Following with the shared theme and the Music sidebar construction.
- A user can add an RSS, Atom, and JSON feed, read items offline after one successful fetch, mark read, star, save, search, and import/export OPML.
- Malformed feeds and private URLs fail with a toast and do not change older items.
- Tests listed above pass.
- Startup does not contact Assistant or open a model.

### Phase 2 — `nixe-sync` and Nextcloud

Accepted when two scratch databases and a stub WebDAV server converge after interleaved offline edits, a deletion, and a killed sync, and a wrong app password reports an error without writing the local file. Article bodies are absent from the uploaded batches.

### Phase 3 — Discover and For You

Shipped directory is a curated OPML in the repo, reviewed as data, with no network call required to browse it. Adding a site URL finds `rel=alternate` feed links. Topic follow creates a saved filter. Recommendations are a local score from tags, stars, and folder names, with a visible “why this” line, and a switch that removes the For You page entirely. Duplicate grouping uses normalized title plus a time window. No embeddings.

### Phase 4 — Local AI

Adapter behind the Intelligence switch. Summaries, multi-article briefs, source comparison, and spoken briefs. Each action is cancellable, shows progress without blocking the list, and survives a dead Assistant. Basic clustering from Phase 3 does not call the model.

### Phase 5 — Ecosystem

Glimpse plugin, Markdown export, notification toggles in the settings card, desktop entry and icon already installed in Phase 1 get the Omarchy install path verified (`install.sh news`). Settings theme integration is the existing suite file, not a new account page.

### Phase 6 — Later

FreshRSS, Miniflux, more sync providers, richer smart folders, news watches through Reminders, podcast enclosures (Music already plays podcasts; News should not grow a player unless that decision is revisited), research timelines. None of these change Phase 1’s schema except by additive migrations.

## Risks

| Risk | Response |
| --- | --- |
| Copyright: storing and showing full feed HTML | Show attribution and the source link. Cap retention. Keep bodies out of sync. No background article scrape |
| XSS via feed HTML | `ammonia` plus a GTK text renderer. No WebKit |
| SSRF via feed URLs and redirects | Scheme, resolve, and redirect checks above. Test the private ranges |
| Sync corruption | One transaction per applied batch. Remote writes are conditional. Checkpoints sit in that transaction |
| Two devices, clock skew | HLC, not mtime |
| Secret leakage | `secret-tool`, redacted Debug, HTTPS only, tests that scan settings.toml |
| GTK stall | Worker runtime. Main thread only applies row diffs |
| Assistant model cost and VRAM | News never loads a model. AI is off by default |
| Discovery without a vendor API | Bundled OPML and autodiscovery. An empty third-party provider trait can wait |
| `feed-rs` 3.0 model break | Pin 2.4. Upgrade is its own change |
| Notes “app” does not exist | Export Markdown. Do not write the plugin’s private database |
| Shared accounts do not exist | `nixe-sync` credentials. Do not extend Settings’ OS user page into cloud tokens |

## Unresolved

- Whether Ken wants `http` feeds at all, or HTTPS only. The plan allows `http` for an explicitly added feed.
- A silent Assistant completion IPC does not exist yet. Phase 4 starts by adding that command in `apps/assistant`, reviewed on its own, before any summary button calls it.
- The curated Discover OPML has no file yet. Phase 3 should start from a short, attributed list Ken can edit, not from a scraped catalog.
- Nextcloud app-password UX needs a real server to confirm the path and the `412` behavior. Stub tests cover the client until then.

## Implementation order after approval

1. Add `crates/nixe-sync` with the trait, HLC, merge tests, and credential wrapper. No WebDAV calls yet beyond the types Phase 1 needs for `change_log` (the table exists; the worker does not upload).
2. Add `crates/nixe-news-core` with schema v1, parser, fetch guard, OPML, FTS, and fixtures.
3. Add `apps/news` using Music’s window construction, wire the worker, settings card, control interface, desktop file, icon, `install.sh`, and README row.
4. Run the Phase 1 tests and the 800 px layout check.
5. Stop. Phase 2 begins only when Phase 1 is accepted.

Copy this document to `apps/news/PLAN.md` as the first file of that work so the repo carries the decisions. Do not start Discover, sync upload, or AI in the same change.
