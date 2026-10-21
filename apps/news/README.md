# News

A local news reader for [Omarchy](https://omarchy.org). It subscribes to RSS, Atom,
and JSON feeds, keeps articles in SQLite, and takes its colors from your Omarchy theme.

Reading works offline after a feed has been fetched once. News does not call a news
service, and it does not load an AI model to open or search articles.

## What it does

- Add a feed by its address. `http` is fetched only when you type that address.
  Private, loopback, and link-local addresses are refused.
- Follow feeds in folders. All, Unread, Starred, and Saved are always in the sidebar.
- Mark read or unread, star, and save. Opening an article marks it read unless you turn that off.
- Search titles, authors, and excerpts.
- Import and export subscriptions as OPML.
- List and compact article layouts. The article itself is plain text from the feed, with a link to the original page.

Feeds refresh in the background about once an hour (15 minutes at the shortest). A feed
that fails waits longer before the next try. Full article text from the feed is cached
under `~/.cache/nixe-news/` when “Keep feed content” is on.

## Where things live

| | |
| --- | --- |
| Settings | `~/.config/nixe-news/settings.toml` |
| Library | `~/.local/share/nixe-news/news.sqlite` |
| Cache | `~/.cache/nixe-news/` |

Sync, publication discovery, and local summaries are reserved in the schema and the
settings card. They are off in this version.

## Keys

`Ctrl+F` search, `Ctrl+N` add a feed, `Ctrl+I` import OPML, `Ctrl+E` export OPML,
`J` / `K` move, `Enter` open, `M` read, `S` star, `B` save, `R` refresh,
`Ctrl+B` sidebar, `Ctrl+,` settings, `F1` shortcuts, `Esc` close the article.
