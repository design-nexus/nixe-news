# News — notes for working on this app

- GTK4 (`gtk4` 0.11, `v4_18`) + Rust. No libadwaita and no WebKit. The window follows
  Music: a flat sidebar (`hexpand` false, 250 px, icons under 980 px), the page, and a
  status bar. Settings is `nixe_ui::settings_card`. Colors are `@theme_*` tokens.
- `crates/nixe-news-core` owns SQLite, feed parsing, fetch, OPML, and search. The app
  crate is the GTK shell and the worker. `crates/nixe-sync` moves folders, feeds, read
  state, and a few preferences to Nextcloud WebDAV. Article bodies stay on this computer.
- The worker is a Tokio thread. HTTP is blocking `reqwest` inside `spawn_blocking`.
  Results return with `glib::idle_add_once`. Do not fetch or parse on the GTK thread.
- `http://` is allowed only when the user typed that URL. Private and loopback targets
  stay blocked unless `NIXE_NEWS_ALLOW_PRIVATE` is set (tests and local fixtures).
- Reader text is sanitized feed HTML turned into plain text. Open original uses
  `xdg-open` for `http` and `https` only.
- Control tools on this app's bus name: `news.search`, `news.headlines`,
  `news.mark_read`, and `news.add_feed`. Adding a feed from Assistant stays off unless
  `assistant_can_subscribe` is set.
- Schema `user_version` 2 adds categories, interests, smart filters, and duplicate clusters.
  Discover browses `directory.opml` with no network call. For You is off until
  Settings → Intelligence → Recommendations. It scores stars, tags, and folder names.
- Summaries are off until Settings → Intelligence → Summaries. They call Assistant and do not
  load a model. If Assistant fails, the article text stays on screen.
- Export writes one article as Markdown to a path the user picks. Assistant uses
  `news.export_markdown`, which stays inside the home folder and does not write Notes.
- `apps/news-glimpse` searches `news.sqlite` read-only for Glimpse (`news`, `news unread`,
  `news saved`, or free text). Notifications for new articles and feed failures stay off
  until Settings → Notifications.
- Checks: `cargo test -p nixe-sync -p nixe-news-core -p nixe-news` and
  `cargo clippy -p nixe-news -p nixe-news-core -p nixe-sync --all-targets -- -D warnings`.
  Layout: `NEWS_SNAPSHOT=/tmp/news.png NEWS_SNAPSHOT_W=800 NEWS_SNAPSHOT_H=700 nixe-news`
  under scratch `XDG_*` dirs. It prints the sidebar width and whether the nav is compact.
