//! The article list and front page. List rows are created by a factory and
//! rebound as the user scrolls; Headlines and section views render the paper.

use crate::discover_page;
use crate::paths;
use crate::prefs;
use crate::reader;
use crate::source;
use crate::widgets;
use crate::window;
use crate::worker;
use gtk::gio;
use gtk::prelude::*;
use nixe_news_core::{AiFailure, Edition, Excerpt, View, lay_paper};
use std::cell::{Cell, RefCell};

thread_local! {
    static STACK: RefCell<Option<gtk::Stack>> = const { RefCell::new(None) };
    static STORE: RefCell<Option<gio::ListStore>> = const { RefCell::new(None) };
    static LIST_PAGE: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
    static PAPER: RefCell<Option<gtk::ScrolledWindow>> = const { RefCell::new(None) };
    static PAPER_BODY: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static SECTION: RefCell<String> = const { RefCell::new(String::new()) };
    static SELECTION: RefCell<Option<gtk::SingleSelection>> = const { RefCell::new(None) };
    static LIST: RefCell<Option<gtk::ListView>> = const { RefCell::new(None) };
    static EMPTY: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static EMPTY_TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static EMPTY_DESC: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static TOOLBAR_TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static TOOLBAR_META: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static LAYOUT_BTN: RefCell<Option<gtk::Button>> = const { RefCell::new(None) };
    static FOLLOW_BTN: RefCell<Option<gtk::Button>> = const { RefCell::new(None) };
    static DAILY_BRIEF: RefCell<Option<String>> = const { RefCell::new(None) };
    static DAILY_BRIEF_RUNNING: Cell<bool> = const { Cell::new(false) };
    static BRIEF_EXPANDED: Cell<bool> = const { Cell::new(false) };
    static DISCOVER: Cell<bool> = const { Cell::new(false) };
    static DISCOVER_HOST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

pub fn build() -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&toolbar());
    let store = gio::ListStore::new::<gtk::StringObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    selection.set_autoselect(false);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let outer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        outer.add_css_class("article-row");
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("unread-dot");
        dot.set_valign(gtk::Align::Start);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        let title = gtk::Label::new(None);
        title.add_css_class("article-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let excerpt = gtk::Label::new(None);
        excerpt.add_css_class("article-excerpt");
        excerpt.set_xalign(0.0);
        excerpt.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let meta = gtk::Label::new(None);
        meta.add_css_class("article-meta");
        meta.set_xalign(0.0);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&title);
        text.append(&excerpt);
        text.append(&meta);
        outer.append(&dot);
        outer.append(&text);
        item.set_child(Some(&outer));
    });
    factory.connect_bind(|_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(id) = item.item().and_downcast::<gtk::StringObject>() else { return };
        let Some(article) = window::find_article(&id.string()) else { return };
        let Some(outer) = item.child().and_downcast::<gtk::Box>() else { return };
        let compact = prefs::get().layout == "compact";
        if compact {
            outer.add_css_class("compact");
        } else {
            outer.remove_css_class("compact");
        }
        let Some(dot) = outer.first_child().and_downcast::<gtk::Box>() else { return };
        let Some(text) = dot.next_sibling().and_downcast::<gtk::Box>() else { return };
        let Some(title) = text.first_child().and_downcast::<gtk::Label>() else { return };
        let Some(excerpt) = title.next_sibling().and_downcast::<gtk::Label>() else { return };
        let Some(meta) = excerpt.next_sibling().and_downcast::<gtk::Label>() else { return };

        dot.set_valign(if compact { gtk::Align::Center } else { gtk::Align::Start });
        if article.read {
            dot.remove_css_class("active");
            title.remove_css_class("unread");
        } else {
            dot.add_css_class("active");
            title.add_css_class("unread");
        }
        title.set_text(headline(&article.title));
        excerpt.set_text(&article.summary);
        excerpt.set_visible(!compact && !article.summary.is_empty());

        let mut line = source::byline(&article.original_url, &article.feed_title, article.published_at);
        if article.starred {
            line.push_str(" · ★ Starred");
        }
        if article.saved {
            line.push_str(" · Saved");
        }
        if !article.why.is_empty() {
            line = format!("{line} · {}", article.why);
        }
        meta.set_text(&line);
    });
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list.set_single_click_activate(true);
    list.connect_activate(move |list, pos| {
        let Some(model) = list.model() else { return };
        let Some(id) = model.item(pos).and_downcast::<gtk::StringObject>() else { return };
        window::select_id(Some(id.string().to_string()));
        if let Some(article) = window::find_article(&id.string()) {
            reader::show(&article);
        }
    });
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&list)
        .hexpand(true)
        .vexpand(true)
        .build();

    let paper_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    paper_body.set_margin_top(16);
    paper_body.set_margin_bottom(36);
    paper_body.set_margin_start(14);
    paper_body.set_margin_end(14);
    paper_body.set_hexpand(true);
    let paper = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&paper_body)
        .hexpand(true)
        .vexpand(true)
        .build();
    paper.set_visible(false);

    let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
    empty.add_css_class("empty-state");
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_can_target(false);
    let empty_icon = gtk::Image::from_icon_name("newspaper-symbolic");
    empty_icon.set_pixel_size(36);
    empty_icon.add_css_class("dim");
    let empty_title = widgets::label("No stories", "toolbar-title");
    empty_title.set_halign(gtk::Align::Center);
    let empty_desc = widgets::label("Follow topics or channels to start reading.", "empty-hint");
    empty_desc.set_halign(gtk::Align::Center);
    empty_desc.set_justify(gtk::Justification::Center);
    empty_desc.set_wrap(true);
    empty_desc.set_max_width_chars(48);
    empty.append(&empty_icon);
    empty.append(&empty_title);
    empty.append(&empty_desc);

    let page = gtk::Overlay::new();
    page.set_child(Some(&scroll));
    page.add_overlay(&empty);
    root.append(&page);
    root.append(&paper);
    stack.add_named(&root, Some("list"));
    stack.add_named(&reader::build(), Some("story"));

    let discover = gtk::Box::new(gtk::Orientation::Vertical, 0);
    discover.set_hexpand(true);
    discover.set_vexpand(true);
    DISCOVER_HOST.with(|host| *host.borrow_mut() = Some(discover.clone()));
    stack.add_named(&discover, Some("discover"));

    STACK.with(|s| *s.borrow_mut() = Some(stack.clone()));
    STORE.with(|s| *s.borrow_mut() = Some(store));
    SELECTION.with(|s| *s.borrow_mut() = Some(selection));
    LIST.with(|s| *s.borrow_mut() = Some(list));
    LIST_PAGE.with(|s| *s.borrow_mut() = Some(page));
    PAPER.with(|s| *s.borrow_mut() = Some(paper));
    PAPER_BODY.with(|s| *s.borrow_mut() = Some(paper_body));
    EMPTY.with(|s| *s.borrow_mut() = Some(empty));
    EMPTY_TITLE.with(|s| *s.borrow_mut() = Some(empty_title));
    EMPTY_DESC.with(|s| *s.borrow_mut() = Some(empty_desc));
    stack
}

fn toolbar() -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("toolbar");
    let title = widgets::label("Top Stories", "toolbar-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let meta = widgets::label("", "toolbar-meta");
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    bar.append(&title);
    bar.append(&meta);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);

    let follow = gtk::Button::with_label("+ Follow");
    follow.add_css_class("follow-button");
    follow.set_visible(false);
    follow.connect_clicked(|_| {
        let sec = SECTION.with(|s| s.borrow().clone());
        if !sec.is_empty() {
            let is_followed = window::with_db(|db| db.smart_filters())
                .map(|filters| filters.iter().any(|f| f.name.eq_ignore_ascii_case(&sec)))
                .unwrap_or(false);
            if is_followed {
                let _ = window::with_db(|db| db.unfollow_topic(&sec));
                window::toast(&format!("Unfollowed {sec}"));
            } else {
                let _ = window::with_db(|db| db.follow_topic(&sec));
                window::toast(&format!("Following {sec}"));
            }
            window::refresh_lists();
        } else if let View::Topic(_) = window::view() {
            let name = window::current_page_name();
            let _ = window::with_db(|db| db.unfollow_topic(&name));
            window::toast(&format!("Unfollowed {name}"));
            show_section("");
            window::navigate(&View::All);
        } else if let View::Feed(id) = window::view() {
            let name = window::current_page_name();
            if window::with_db(|db| db.delete_feed(&id)).is_ok() {
                window::toast(&format!("Unfollowed {name}"));
                show_section("");
                window::navigate(&View::All);
            }
        }
    });
    bar.append(&follow);

    let refresh = widgets::bar_button("view-refresh-symbolic", "Refresh news (R)");
    refresh.connect_clicked(|_| {
        window::toast("Refreshing news…");
        worker::refresh_current();
    });
    bar.append(&refresh);

    let compact = prefs::get().layout == "compact";
    let toggle = widgets::bar_button(
        if compact { "view-grid-symbolic" } else { "view-list-symbolic" },
        if compact { "Switch to cover cards" } else { "Switch to compact list" },
    );
    toggle.connect_clicked(|_| {
        prefs::update(|p| p.layout = if p.layout == "compact" { "list".into() } else { "compact".into() });
        window::reload();
    });
    bar.append(&toggle);

    TOOLBAR_TITLE.with(|s| *s.borrow_mut() = Some(title));
    TOOLBAR_META.with(|s| *s.borrow_mut() = Some(meta));
    LAYOUT_BTN.with(|s| *s.borrow_mut() = Some(toggle));
    FOLLOW_BTN.with(|s| *s.borrow_mut() = Some(follow));
    bar
}

#[allow(dead_code)]
pub fn mark_current_read() {
    let ids: Vec<String> = window::articles().into_iter().filter(|a| !a.read).map(|a| a.id).collect();
    if ids.is_empty() {
        return;
    }
    let count = ids.len();
    let _ = window::with_db(|db| {
        for id in &ids {
            db.set_read(id, true)?;
        }
        Ok(())
    });
    window::refresh_lists();
    window::toast(&if count == 1 { "1 story marked read.".into() } else { format!("{count} stories marked read.") });
}

fn update_toolbar() {
    let searching = window::search_active();
    let page = if searching {
        format!("Search: “{}”", window::search_query())
    } else {
        window::current_page_name()
    };
    if let Some(title) = TOOLBAR_TITLE.with(|s| s.borrow().clone()) {
        title.set_text(&page);
    }
    let count = window::articles().len();
    let meta_text = if count == 0 {
        String::new()
    } else if count == 1 {
        "· 1 story".into()
    } else {
        format!("· {count} stories")
    };
    if let Some(meta) = TOOLBAR_META.with(|s| s.borrow().clone()) {
        meta.set_text(&meta_text);
    }
    if let Some(btn) = LAYOUT_BTN.with(|s| s.borrow().clone()) {
        let compact = prefs::get().layout == "compact";
        btn.set_icon_name(if compact { "view-grid-symbolic" } else { "view-list-symbolic" });
        btn.set_tooltip_text(Some(if compact { "Switch to cover cards" } else { "Switch to compact list" }));
    }
    if let Some(btn) = FOLLOW_BTN.with(|s| s.borrow().clone()) {
        let sec = SECTION.with(|s| s.borrow().clone());
        if !sec.is_empty() {
            let is_followed = window::with_db(|db| db.smart_filters())
                .map(|filters| filters.iter().any(|f| f.name.eq_ignore_ascii_case(&sec)))
                .unwrap_or(false);
            btn.set_visible(true);
            if is_followed {
                btn.set_label("✓ Following");
                btn.add_css_class("following");
            } else {
                btn.set_label("+ Follow");
                btn.remove_css_class("following");
            }
        } else if matches!(window::view(), View::Topic(_) | View::Feed(_)) && !searching {
            btn.set_visible(true);
            btn.set_label("Unfollow");
            btn.remove_css_class("following");
        } else {
            btn.set_visible(false);
        }
    }
}

pub fn show_story() {
    if let Some(stack) = STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("story");
    }
}

pub fn close_story() {
    let Some(stack) = STACK.with(|s| s.borrow().clone()) else { return };
    if stack.visible_child_name().as_deref() == Some("story") {
        stack.set_visible_child_name("list");
    }
}

pub fn bind_articles() {
    update_toolbar();
    let headlines = window::view() == View::All && !discover_visible() && !window::search_active();
    if let Some(paper) = PAPER.with(|s| s.borrow().clone()) {
        paper.set_visible(headlines);
    }
    if let Some(page) = LIST_PAGE.with(|s| s.borrow().clone()) {
        page.set_visible(!headlines);
    }
    if headlines {
        fill_paper();
        return;
    }
    let Some(store) = STORE.with(|s| s.borrow().clone()) else { return };
    store.remove_all();
    let mut seen_urls = std::collections::HashSet::new();
    let mut seen_titles: Vec<(String, Option<i64>)> = Vec::new();
    for article in window::articles() {
        let clean = if !article.url.is_empty() {
            nixe_news_core::canonical_url(&article.url).unwrap_or_else(|_| article.url.clone())
        } else {
            nixe_news_core::canonical_url(&article.original_url).unwrap_or_else(|_| article.original_url.clone())
        };
        if !clean.is_empty() && seen_urls.contains(&clean) {
            continue;
        }
        if !article.title.trim().is_empty()
            && seen_titles.iter().any(|(t, when)| nixe_news_core::same_story(&article.title, article.published_at, t, *when))
        {
            continue;
        }
        if !clean.is_empty() {
            seen_urls.insert(clean);
        }
        seen_titles.push((article.title.clone(), article.published_at));
        store.append(&gtk::StringObject::new(&article.id));
    }
    if let Some(empty) = EMPTY.with(|s| s.borrow().clone()) {
        let feeds = window::with_db(|db| db.feeds()).map(|f| f.len()).unwrap_or(0);
        empty.set_visible(store.n_items() == 0);
        let (title, desc) = if window::search_active() {
            ("No matching articles", "Try a different search term or press Esc to clear.")
        } else if window::view() == View::ForYou {
            ("Nothing in For You yet", "Bookmark stories or follow topics. For You curates stories based on your reading interests.")
        } else if feeds == 0 {
            ("No channels followed", "Explore Channels & Topics in the sidebar to get started.")
        } else if window::view() == View::Unread {
            ("You're caught up", "No unread stories right now.")
        } else if window::view() == View::Starred {
            ("No starred stories", "Bookmark stories to read them here.")
        } else if window::view() == View::Saved {
            ("No saved stories", "Save stories with B or the bookmark button to read them offline later.")
        } else {
            ("No stories right now", "Refresh news with R to check for the latest updates.")
        };
        if let Some(t) = EMPTY_TITLE.with(|s| s.borrow().clone()) {
            t.set_text(title);
        }
        if let Some(d) = EMPTY_DESC.with(|s| s.borrow().clone()) {
            d.set_text(desc);
        }
    }
}

pub fn show_section(name: &str) {
    SECTION.with(|section| *section.borrow_mut() = name.to_string());
}

pub fn section() -> String {
    SECTION.with(|section| section.borrow().clone())
}

pub fn apply_daily_brief(result: Result<String, AiFailure>) {
    DAILY_BRIEF_RUNNING.with(|r| r.set(false));
    let text = match result {
        Ok(brief) if !brief.trim().is_empty() => brief,
        _ => {
            let articles = window::articles();
            let mut lines = Vec::new();
            lines.push("Key developments across today's stories:".to_string());
            for a in articles.iter().take(4) {
                let name = source::name(&a.original_url, &a.feed_title);
                let summary = if a.summary.is_empty() {
                    a.title.clone()
                } else {
                    format!("{}: {}", a.title, a.summary)
                };
                lines.push(format!("• [{name}] {summary}"));
            }
            lines.join("\n\n")
        }
    };
    DAILY_BRIEF.with(|b| *b.borrow_mut() = Some(text));
    bind_articles();
}

pub fn ensure_daily_brief() {
    if !prefs::get().summaries {
        return;
    }
    let has_brief = DAILY_BRIEF.with(|b| b.borrow().is_some());
    let is_running = DAILY_BRIEF_RUNNING.with(|r| r.get());
    if has_brief || is_running {
        return;
    }
    let articles = window::articles();
    if articles.is_empty() {
        return;
    }
    DAILY_BRIEF_RUNNING.with(|r| r.set(true));
    let excerpts: Vec<Excerpt> = articles
        .iter()
        .take(6)
        .map(|a| {
            let text = if a.summary.is_empty() {
                a.title.clone()
            } else {
                format!("{}: {}", a.title, a.summary)
            };
            Excerpt {
                title: a.title.clone(),
                feed: a.feed_title.clone(),
                url: a.original_url.clone(),
                text,
            }
        })
        .collect();
    worker::brief("brief".to_string(), excerpts, 9999);
}

fn daily_briefing_card() -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("daily-briefing-card");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = gtk::Image::from_icon_name("starred-symbolic");
    icon.add_css_class("daily-briefing-icon");
    let title = widgets::label("TODAY'S AI BRIEFING", "daily-briefing-title");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    header.append(&icon);
    header.append(&title);

    let expanded = BRIEF_EXPANDED.with(|e| e.get());
    let chevron_icon = if expanded { "pan-up-symbolic" } else { "pan-down-symbolic" };
    let toggle_btn = gtk::Button::builder()
        .icon_name(chevron_icon)
        .tooltip_text(if expanded { "Collapse briefing" } else { "Read briefing" })
        .css_classes(["flat", "daily-briefing-toggle"])
        .build();
    toggle_btn.connect_clicked(|_| {
        BRIEF_EXPANDED.with(|e| e.set(!e.get()));
        bind_articles();
    });
    header.append(&toggle_btn);
    card.append(&header);

    let running = DAILY_BRIEF_RUNNING.with(|r| r.get());
    let brief = DAILY_BRIEF.with(|b| b.borrow().clone());

    if expanded {
        if running {
            let status = widgets::label("Generating your briefing from today's top stories…", "daily-briefing-status");
            status.set_xalign(0.0);
            card.append(&status);
        } else if let Some(text) = brief {
            let body = widgets::label(&text, "daily-briefing-text");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            card.append(&body);
        } else {
            let desc = widgets::label(
                "Updating briefing with today's key headlines across World, Tech, and Science…",
                "daily-briefing-status",
            );
            desc.set_xalign(0.0);
            card.append(&desc);
        }
    } else {
        let teaser_text = if running {
            "Generating your briefing from today's top stories…".to_string()
        } else if let Some(text) = brief {
            let first = text
                .lines()
                .find(|l| !l.trim().is_empty() && !l.starts_with('#') && !l.starts_with("Key developments"))
                .unwrap_or_else(|| text.lines().next().unwrap_or(&text));
            first.to_string()
        } else {
            let articles = window::articles();
            articles
                .first()
                .map(|a| {
                    if a.summary.is_empty() {
                        a.title.clone()
                    } else {
                        format!("{}: {}", a.title, a.summary)
                    }
                })
                .unwrap_or_else(|| "Catch up in 60 seconds with an AI summary of today's key headlines.".into())
        };

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let teaser = widgets::label(&teaser_text, "daily-briefing-teaser");
        teaser.set_xalign(0.0);
        teaser.set_hexpand(true);
        teaser.set_ellipsize(gtk::pango::EllipsizeMode::End);
        teaser.set_lines(1);
        row.append(&teaser);

        let read_more = gtk::Button::with_label("Read briefing →");
        read_more.add_css_class("daily-briefing-link");
        read_more.connect_clicked(|_| {
            BRIEF_EXPANDED.with(|e| e.set(true));
            bind_articles();
        });
        row.append(&read_more);
        card.append(&row);
    }

    card.upcast()
}

fn fill_paper() {
    let Some(body) = PAPER_BODY.with(|s| s.borrow().clone()) else { return };
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    let wanted = SECTION.with(|section| section.borrow().clone());
    let articles = window::articles();
    let edition = if !wanted.is_empty() {
        if articles.is_empty() {
            Vec::new()
        } else {
            let mut seen_urls = std::collections::HashSet::new();
            let mut seen_titles: Vec<(String, Option<i64>)> = Vec::new();
            let mut unique = Vec::new();
            for a in articles {
                let clean = if !a.url.is_empty() {
                    nixe_news_core::canonical_url(&a.url).unwrap_or_else(|_| a.url.clone())
                } else {
                    nixe_news_core::canonical_url(&a.original_url).unwrap_or_else(|_| a.original_url.clone())
                };
                if !clean.is_empty() && seen_urls.contains(&clean) {
                    continue;
                }
                if !a.title.trim().is_empty()
                    && seen_titles.iter().any(|(t, when)| nixe_news_core::same_story(&a.title, a.published_at, t, *when))
                {
                    continue;
                }
                if !clean.is_empty() {
                    seen_urls.insert(clean);
                }
                seen_titles.push((a.title.clone(), a.published_at));
                unique.push(a);
            }
            vec![Edition { name: wanted.clone(), stories: unique }]
        }
    } else {
        let sections = window::with_db(|db| db.feed_sections()).unwrap_or_default();
        lay_paper(&articles, &sections)
    };
    let disabled = prefs::get().disabled_categories;
    let edition: Vec<_> = edition
        .into_iter()
        .filter(|s| !disabled.iter().any(|d| d.eq_ignore_ascii_case(&s.name)))
        .collect();

    if edition.is_empty() {
        let feeds = window::with_db(|db| db.feeds()).map(|f| f.len()).unwrap_or(0);
        let name = if wanted.is_empty() { "Top Stories".into() } else { wanted };
        let empty = widgets::empty_state(
            "newspaper-symbolic",
            &if feeds == 0 { "Welcome to News".into() } else { format!("No stories in {name}") },
            if feeds == 0 {
                "Follow topics and publications in Channels & Topics to build your feed."
            } else {
                "Fetching the latest stories for this section."
            },
            if feeds == 0 {
                Some(("Explore Channels & Topics", Box::new(window::show_discover)))
            } else {
                None
            },
        );
        body.append(&empty);
        return;
    }
    let mut covers = Vec::new();
    let mut missing_images: Vec<(String, String)> = Vec::new();
    for section in &edition {
        for story in &section.stories {
            if story.image_url.is_none()
                && (story.original_url.starts_with("https://") || story.original_url.starts_with("http://"))
                && !paths::no_image_file(&story.id).exists()
            {
                missing_images.push((story.id.clone(), story.original_url.clone()));
            }
        }
    }
    if prefs::get().layout == "compact" {
        if wanted.is_empty() {
            let masthead = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            masthead.add_css_class("newspaper-masthead");
            let date_lbl = widgets::label(&source::masthead_date(), "masthead-date");
            date_lbl.set_xalign(0.0);
            masthead.append(&date_lbl);
            body.append(&masthead);
            if prefs::get().summaries {
                ensure_daily_brief();
                body.append(&daily_briefing_card());
            }
        }
        for (idx, section) in edition.iter().enumerate() {
            if wanted.is_empty() {
                body.append(&section_header(&section.name, idx == 0));
            }
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            list.add_css_class("group-list");
            for story in &section.stories {
                list.append(&compact_row(story));
            }
            body.append(&list);
        }
        return;
    }
    let hero = edition[0].stories[0].clone();
    if wanted.is_empty() {
        let masthead = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        masthead.add_css_class("newspaper-masthead");
        let date_lbl = widgets::label(&source::masthead_date(), "masthead-date");
        date_lbl.set_xalign(0.0);
        masthead.append(&date_lbl);
        body.append(&masthead);
        if prefs::get().summaries {
            ensure_daily_brief();
            body.append(&daily_briefing_card());
        }
    }
    body.append(&hero_card(&hero, &mut covers));
    for (idx, section) in edition.iter().enumerate() {
        let rest: Vec<_> = section.stories.iter().filter(|s| s.id != hero.id).collect();
        if rest.is_empty() {
            continue;
        }
        if wanted.is_empty() {
            body.append(&section_header(&section.name, idx == 0));
        }

        let top_cards: Vec<_> = rest.iter().take(3).cloned().collect();
        let collapsed_rows: Vec<_> = rest.iter().skip(3).take(4).cloned().collect();

        let card_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        card_row.set_homogeneous(true);
        card_row.set_hexpand(true);
        for story in top_cards {
            card_row.append(&story_card(&story, &mut covers));
        }
        body.append(&card_row);

        if !collapsed_rows.is_empty() {
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            list.add_css_class("group-list");
            list.set_margin_top(6);
            for story in collapsed_rows {
                list.append(&compact_row(&story));
            }
            body.append(&list);
        }
    }
    worker::covers(covers);
    if !missing_images.is_empty() {
        worker::discover_images(missing_images);
    }
}

fn section_header(name: &str, first: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("section-head");
    if first {
        row.add_css_class("first");
    }
    let head = widgets::label(&name.to_uppercase(), "section-name");
    head.set_xalign(0.0);
    head.set_hexpand(true);
    row.append(&head);
    if name != "Other" {
        let more = gtk::Button::with_label(&format!("{name} →"));
        more.add_css_class("section-more");
        let section_name = name.to_string();
        more.connect_clicked(move |_| {
            show_section(&section_name);
            window::navigate(&View::All);
            worker::open_section(section_name.clone());
        });
        row.append(&more);
    }
    row
}

fn hero_card(article: &nixe_news_core::ArticleListItem, covers: &mut Vec<String>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("hero-card");
    button.set_hexpand(true);

    let is_narrow = window::is_narrow();
    if is_narrow {
        button.add_css_class("narrow");
    }

    let container = if is_narrow {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let art = cover(article, -1, 190, covers);
        art.set_hexpand(true);
        art.set_valign(gtk::Align::Start);
        col.append(&art);
        col
    } else {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        let art = cover(article, 460, 260, covers);
        art.set_valign(gtk::Align::Center);
        row.append(&art);
        row
    };

    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    column.set_hexpand(true);
    if !is_narrow {
        column.set_valign(gtk::Align::Center);
    }

    let byline = source::byline(&article.original_url, &article.feed_title, article.published_at);
    let read_time = source::read_time(&article.summary);
    let kicker_text = format!("TOP STORY · {byline} · {read_time}");
    let kicker = widgets::label(&kicker_text, "story-kicker");
    kicker.set_xalign(0.0);
    kicker.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&kicker);

    let title = widgets::label(headline(&article.title), "hero-headline");
    if article.read {
        title.add_css_class("read");
    }
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(if is_narrow { 3 } else { 4 });
    column.append(&title);

    if !article.summary.is_empty() {
        let summary = widgets::label(&article.summary, "hero-summary");
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        summary.set_lines(if is_narrow { 2 } else { 3 });
        column.append(&summary);
    }

    container.append(&column);
    button.set_child(Some(&container));
    let article = article.clone();
    button.connect_clicked(move |_| reader::show(&article));
    button
}

fn story_card(article: &nixe_news_core::ArticleListItem, covers: &mut Vec<String>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("story-card");
    button.set_size_request(200, -1);
    button.set_hexpand(true);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_hexpand(true);
    column.append(&cover(article, -1, 140, covers));

    let byline = source::byline(&article.original_url, &article.feed_title, article.published_at);
    let kicker = widgets::label(&byline, "story-kicker");
    kicker.set_xalign(0.0);
    kicker.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&kicker);

    let title = widgets::label(headline(&article.title), "story-headline");
    if article.read {
        title.add_css_class("read");
    }
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(3);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&title);

    if !article.summary.is_empty() {
        let excerpt = widgets::label(&article.summary, "story-excerpt");
        excerpt.set_xalign(0.0);
        excerpt.set_wrap(true);
        excerpt.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        excerpt.set_lines(2);
        excerpt.set_ellipsize(gtk::pango::EllipsizeMode::End);
        column.append(&excerpt);
    }
    button.set_child(Some(&column));
    let article = article.clone();
    button.connect_clicked(move |_| reader::show(&article));
    button
}

fn is_external_article(article: &nixe_news_core::ArticleListItem) -> bool {
    nixe_news_core::requires_publisher_site(&article.original_url)
        || paths::unavailable_file(&article.id).exists()
}

fn cover(article: &nixe_news_core::ArticleListItem, width: i32, height: i32, covers: &mut Vec<String>) -> gtk::Widget {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("cover-frame");
    overlay.set_size_request(width, height);
    overlay.set_hexpand(width < 0);
    overlay.set_overflow(gtk::Overflow::Hidden);

    let fallback = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fallback.add_css_class("cover-fallback");
    fallback.set_size_request(width, height);
    fallback.set_hexpand(width < 0);
    fallback.set_halign(gtk::Align::Fill);
    fallback.set_valign(gtk::Align::Fill);
    let pub_name = source::name(&article.original_url, &article.feed_title);
    let mark = pub_name.chars().next().unwrap_or('N').to_uppercase().to_string();
    let letter = widgets::label(&mark, "cover-mark");
    letter.set_halign(gtk::Align::Center);
    letter.set_valign(gtk::Align::Center);
    letter.set_vexpand(true);
    fallback.append(&letter);
    overlay.set_child(Some(&fallback));

    if let Some(url) = article.image_url.as_ref().filter(|url| url.starts_with("https://")) {
        let path = paths::cover_file(url);
        if path.exists() {
            let picture = gtk::Picture::for_filename(&path);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            picture.set_halign(gtk::Align::Fill);
            picture.set_valign(gtk::Align::Fill);
            overlay.add_overlay(&picture);
            overlay.set_measure_overlay(&picture, false);
        } else {
            covers.push(url.clone());
        }
    }

    if is_external_article(article) {
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        badge.add_css_class("publisher-badge");
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_top(6);
        badge.set_margin_end(6);
        let icon = gtk::Image::from_icon_name("external-link-symbolic");
        icon.set_pixel_size(11);
        badge.append(&icon);
        let lbl = widgets::label("Web", "publisher-badge-label");
        badge.append(&lbl);
        badge.set_tooltip_text(Some("Opens at publisher"));
        overlay.add_overlay(&badge);
        overlay.set_measure_overlay(&badge, false);
    }
    overlay.upcast()
}

fn compact_row(article: &nixe_news_core::ArticleListItem) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("compact-story-row");
    button.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("unread-dot");
    if !article.read {
        dot.add_css_class("active");
    }
    dot.set_valign(gtk::Align::Center);
    row.append(&dot);

    let title = widgets::label(headline(&article.title), "story-headline");
    if article.read {
        title.add_css_class("read");
    }
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&title);

    if is_external_article(article) {
        let ext = gtk::Image::from_icon_name("external-link-symbolic");
        ext.add_css_class("external-icon");
        ext.set_pixel_size(12);
        ext.set_tooltip_text(Some("Opens at publisher"));
        row.append(&ext);
    }

    let line = source::byline(&article.original_url, &article.feed_title, article.published_at);
    let meta = widgets::label(&line, "story-kicker");
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.set_xalign(1.0);
    meta.set_valign(gtk::Align::Center);
    row.append(&meta);

    button.set_child(Some(&row));
    let article = article.clone();
    button.connect_clicked(move |_| reader::show(&article));
    button
}

fn headline(title: &str) -> &str {
    if title.is_empty() { "Untitled" } else { title }
}

pub fn select_position(pos: u32) {
    if let Some(selection) = SELECTION.with(|s| s.borrow().clone()) {
        selection.set_selected(pos);
    }
}

pub fn show_list() {
    DISCOVER.with(|d| d.set(false));
    if let Some(stack) = STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("list");
    }
}

pub fn show_discover() {
    DISCOVER.with(|d| d.set(true));
    refresh_discover();
    if let Some(stack) = STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("discover");
    }
}

pub fn refresh_discover() {
    if let Some(host) = DISCOVER_HOST.with(|host| host.borrow().clone()) {
        discover_page::populate(&host);
    }
}

pub fn discover_visible() -> bool {
    DISCOVER.with(|d| d.get())
}

