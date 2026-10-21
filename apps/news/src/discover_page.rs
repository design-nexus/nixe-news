//! Channels & Topics: curated directory of topics and publications, plus custom sources.

use crate::source;
use crate::timeline;
use crate::widgets;
use crate::window;
use crate::worker;
use gtk::prelude::*;
use nixe_ui::clamp::Clamp;
use std::cell::RefCell;

thread_local! {
    static FOUND: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

pub fn populate(host: &gtk::Box) {
    while let Some(child) = host.first_child() {
        host.remove(&child);
    }

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.add_css_class("toolbar");

    let back = crate::widgets::bar_button("go-previous-symbolic", "Back to stories");
    back.connect_clicked(|_| {
        timeline::show_list();
        window::navigate(&nixe_news_core::View::All);
    });
    toolbar.append(&back);

    let title = widgets::label("Channels & Topics", "toolbar-title");
    toolbar.append(&title);

    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    toolbar.append(&gap);

    host.append(&toolbar);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.add_css_class("settings-page");
    body.set_margin_top(20);
    body.set_margin_bottom(36);
    body.set_margin_start(24);
    body.set_margin_end(24);

    let intro = widgets::label(
        "Follow topics and leading publications to curate your daily news feed. Content updates automatically in the background.",
        "empty-hint",
    );
    intro.set_xalign(0.0);
    intro.set_wrap(true);
    body.append(&intro);

    let followed = window::with_db(|db| db.smart_filters()).unwrap_or_default();
    let subscribed = window::with_db(|db| db.feeds()).unwrap_or_default();
    let topics = nixe_news_core::directory().unwrap_or_default();

    for topic in topics {
        let topic_group = widgets::vbox(0);
        topic_group.add_css_class("group-list");

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        head.add_css_class("settings-option");
        let topic_title = widgets::label(&topic.title, "settings-option-title");
        topic_title.set_hexpand(true);
        topic_title.set_xalign(0.0);
        head.append(&topic_title);

        let name = topic.title.clone();
        let already = followed.iter().any(|filter| filter.name == topic.title);
        let follow = gtk::Button::with_label(if already { "✓ Following" } else { "+ Follow" });
        if already {
            follow.add_css_class("following");
        }
        let btn = follow.clone();
        follow.connect_clicked(move |_| {
            let is_followed = window::with_db(|db| db.smart_filters())
                .map(|filters| filters.iter().any(|f| f.name == name))
                .unwrap_or(false);
            if is_followed {
                let _ = window::with_db(|db| db.unfollow_topic(&name));
                btn.set_label("+ Follow");
                btn.remove_css_class("following");
                window::toast(&format!("Unfollowed {name}"));
            } else {
                let _ = window::with_db(|db| db.follow_topic(&name));
                btn.set_label("✓ Following");
                btn.add_css_class("following");
                window::toast(&format!("Following {name}"));
            }
            window::refresh_lists();
        });
        head.append(&follow);
        topic_group.append(&head);

        for feed in topic.children {
            let Some(url) = feed.feed_url.clone() else { continue };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("settings-option");

            let pub_name = source::name(&url, &feed.title);
            let label = widgets::label(&pub_name, "article-title");
            label.set_hexpand(true);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&label);

            let have = subscribed.iter().any(|item| item.url == url);
            let button = gtk::Button::with_label(if have { "Following" } else { "Follow" });
            if have {
                button.add_css_class("following");
            }
            let sub_btn = button.clone();
            let category = topic.title.clone();
            let curl = url.clone();
            let cname = pub_name.clone();
            button.connect_clicked(move |_| {
                let feeds = window::with_db(|db| db.feeds()).unwrap_or_default();
                if let Some(existing) = feeds.iter().find(|item| item.url == curl) {
                    let _ = window::with_db(|db| db.delete_feed(&existing.id));
                    sub_btn.set_label("Follow");
                    sub_btn.remove_css_class("following");
                    window::toast(&format!("Unfollowed {cname}"));
                    window::refresh_lists();
                } else {
                    worker::add_in_category(curl.clone(), category.clone());
                    sub_btn.set_label("Following");
                    sub_btn.add_css_class("following");
                    window::toast(&format!("Following {cname}"));
                    window::refresh_lists();
                }
            });
            row.append(&button);
            topic_group.append(&row);
        }

        body.append(&topic_group);
    }

    // Custom publication search card at the bottom
    let custom_label = widgets::label("FOLLOW CUSTOM PUBLICATION", "section-name");
    custom_label.set_xalign(0.0);
    custom_label.set_margin_top(8);
    body.append(&custom_label);

    let find_card = widgets::vbox(0);
    find_card.add_css_class("group-list");

    let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    find_row.add_css_class("settings-option");
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Enter website or publication address (e.g. arstechnica.com)"));
    entry.set_hexpand(true);

    let find_btn = gtk::Button::with_label("Find");
    let entry_for_find = entry.clone();
    find_btn.connect_clicked(move |_| worker::find_feeds(entry_for_find.text().to_string()));
    entry.connect_activate(|entry| worker::find_feeds(entry.text().to_string()));

    find_row.append(&entry);
    find_row.append(&find_btn);
    find_card.append(&find_row);
    body.append(&find_card);

    let found = gtk::Box::new(gtk::Orientation::Vertical, 0);
    found.add_css_class("group-list");
    found.set_visible(false);
    body.append(&found);
    FOUND.with(|slot| *slot.borrow_mut() = Some(found));

    let clamped = Clamp::new(&body, 760);
    let scroll = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&clamped)
        .build();

    host.append(&scroll);
}

pub fn show_found(feeds: Vec<nixe_news_core::DiscoveredFeed>) {
    let Some(box_) = FOUND.with(|slot| slot.borrow().clone()) else { return };
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
    box_.set_visible(true);
    if feeds.is_empty() {
        let note = widgets::label("No publications found at that address.", "empty-hint");
        note.set_xalign(0.0);
        note.set_margin_top(12);
        note.set_margin_bottom(12);
        note.set_margin_start(16);
        box_.append(&note);
        return;
    }
    for feed in feeds {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("settings-option");

        let pub_name = source::name(&feed.url, &feed.title);
        let label = widgets::label(&pub_name, "article-title");
        label.set_hexpand(true);
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&label);

        let url = feed.url.clone();
        let button = gtk::Button::with_label("Follow");
        let btn = button.clone();
        let cname = pub_name.clone();
        button.connect_clicked(move |_| {
            worker::add_url(url.clone());
            btn.set_label("Following");
            btn.add_css_class("following");
            window::toast(&format!("Following {cname}"));
            window::refresh_lists();
        });
        row.append(&button);
        box_.append(&row);
    }
}
