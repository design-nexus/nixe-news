//! The News window: the header (the app and the page, the sidebar toggle; the search in
//! the middle; the story count, Refresh and Settings at the end) over the sidebar and the
//! pages. The sidebar does not grow, and it collapses to icons under 980 px or when the
//! user collapses it.

use crate::dialogs;
use crate::paths;
use crate::prefs;
use crate::reader;
use crate::settings_dialog;
use crate::timeline;
use crate::widgets;
use crate::worker;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use nyx_news_core::{ArticleListItem, Db, View, open};
use nyx_ui::chrome;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "F"], "Search news"),
    (&["/"], "Quick search filter"),
    (&["J"], "Next story"),
    (&["K"], "Previous story"),
    (&["Enter"], "Read story"),
    (&["B"], "Save story (Bookmark)"),
    (&["O"], "Open in browser"),
    (&["R"], "Refresh news"),
    (&["Esc"], "Back to Top Stories or clear search"),
    (&["Ctrl", "B"], "Collapse or expand sidebar"),
    (&["Ctrl", ","], "Settings"),
    (&["F1"], "Show keyboard shortcuts"),
    (&["Ctrl", "Q"], "Close"),
];

struct Ui {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    nav: gtk::Box,
    nav_list: gtk::Box,
    search: gtk::SearchEntry,
    header: chrome::AppHeader,
    view: View,
    articles: Vec<ArticleListItem>,
    selected: Option<String>,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
    static NARROW: Cell<bool> = const { Cell::new(false) };
    static TOAST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

pub fn is_narrow() -> bool {
    NARROW.with(|n| n.get())
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

pub fn overlay() -> Option<gtk::Overlay> {
    ui().map(|u| u.borrow().overlay.clone())
}

pub fn with_db<T>(f: impl FnOnce(&mut Db) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut db = open(&paths::library_db())?;
    f(&mut db)
}

pub fn articles() -> Vec<ArticleListItem> {
    ui().map(|u| u.borrow().articles.clone()).unwrap_or_default()
}

pub fn find_article(id: &str) -> Option<ArticleListItem> {
    ui()?.borrow().articles.iter().find(|a| a.id == id).cloned()
}

pub fn view() -> View {
    ui().map(|u| u.borrow().view.clone()).unwrap_or(View::All)
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if ui().is_none() {
        build(app);
        worker::start();
        if prefs::get().refresh_on_startup {
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), worker::refresh_due);
        }
        if prefs::get().sync_enabled {
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), worker::sync_background);
        }
    }
    let Some(ui) = ui() else { return };
    let window = ui.borrow().window.clone();
    window.present();
    match section {
        Some("settings") => settings_dialog::open(),
        Some("discover") => show_discover(),
        Some("all") | Some("following") | None => {
            if !prefs::get().saw_headlines {
                prefs::update(|prefs| prefs.saw_headlines = true);
                navigate(&View::All);
            } else {
                let start = nyx_theme::start_page("all", &prefs::get().last_section);
                navigate(&view_from_section(&start));
            }
        }
        Some(other) => navigate(&view_from_section(other)),
    }
}

fn view_from_section(section: &str) -> View {
    match section {
        "unread" => View::Unread,
        "starred" => View::Starred,
        "saved" => View::Saved,
        "foryou" if prefs::get().recommendations => View::ForYou,
        "discover" => View::All,
        id if id.starts_with("feed:") => View::Feed(id.trim_start_matches("feed:").into()),
        id if id.starts_with("folder:") => View::Folder(id.trim_start_matches("folder:").into()),
        id if id.starts_with("topic:") => View::Topic(id.trim_start_matches("topic:").into()),
        _ => View::All,
    }
}

fn section_id(view: &View) -> String {
    match view {
        View::All => "all".into(),
        View::Unread => "unread".into(),
        View::Starred => "starred".into(),
        View::Saved => "saved".into(),
        View::Feed(id) => format!("feed:{id}"),
        View::Folder(id) => format!("folder:{id}"),
        View::ForYou => "foryou".into(),
        View::Topic(id) => format!("topic:{id}"),
    }
}

fn page_name(view: &View) -> String {
    match view {
        View::All => "Top Stories".into(),
        View::Unread => "Recent Stories".into(),
        View::Starred => "Starred".into(),
        View::Saved => "Saved".into(),
        View::Feed(id) => feed_title(id),
        View::Folder(id) => folder_name(id),
        View::ForYou => "For You".into(),
        View::Topic(id) => topic_name(id),
    }
}

fn feed_title(id: &str) -> String {
    with_db(|db| Ok(db.feed(id)?.map(|f| f.title).unwrap_or_else(|| "Feed".into()))).unwrap_or_else(|_| "Feed".into())
}

fn topic_name(id: &str) -> String {
    with_db(|db| {
        Ok(db
            .smart_filters()?
            .into_iter()
            .find(|filter| filter.id == id)
            .map(|filter| filter.name)
            .unwrap_or_else(|| "Topic".into()))
    })
    .unwrap_or_else(|_| "Topic".into())
}

fn folder_name(id: &str) -> String {
    with_db(|db| Ok(db.folders()?.into_iter().find(|f| f.id == id).map(|f| f.name).unwrap_or_else(|| "Folder".into())))
        .unwrap_or_else(|_| "Folder".into())
}

pub fn navigate(view: &View) {
    let Some(ui) = ui() else { return };
    ui.borrow_mut().view = view.clone();
    prefs::update(|p| p.last_section = section_id(view));
    let name = page_name(view);
    chrome::set_title(&ui.borrow().window, "News", &name);
    ui.borrow().header.set_subtitle(&name);
    timeline::show_list();
    reader::hide();
    reload();
    rebuild_nav();
}

pub fn show_discover() {
    timeline::show_discover();
    reader::hide();
    let ui = ui().unwrap();
    chrome::set_title(&ui.borrow().window, "News", "Discover");
    ui.borrow().header.set_subtitle("Channels & Topics");
    rebuild_nav();
}

pub fn refresh_lists() {
    reload();
    rebuild_nav();
}

pub fn apply_synced_prefs(fields: std::collections::BTreeMap<String, String>) {
    prefs::update(|prefs| {
        if let Some(days) = fields.get("retention_days").and_then(|value| value.parse().ok())
            && prefs.retention_days != days
        {
            prefs.retention_days = days;
        }
        if let Some(layout) = fields.get("layout")
            && (layout == "list" || layout == "compact")
            && prefs.layout != *layout
        {
            prefs.layout = layout.clone();
        }
        if let Some(minutes) = fields.get("poll_minutes").and_then(|value| value.parse::<i64>().ok()) {
            let minutes = minutes.clamp(15, 24 * 60);
            if prefs.poll_minutes != minutes {
                prefs.poll_minutes = minutes;
            }
        }
        if let Some(mark) = fields.get("mark_read_on_open") {
            let mark = mark == "true" || mark == "1";
            if prefs.mark_read_on_open != mark {
                prefs.mark_read_on_open = mark;
            }
        }
        if let Some(on) = fields.get("recommendations") {
            let on = on == "true" || on == "1";
            if prefs.recommendations != on {
                prefs.recommendations = on;
            }
        }
        if let Some(on) = fields.get("summaries") {
            let on = on == "true" || on == "1";
            if prefs.summaries != on {
                prefs.summaries = on;
            }
        }
    });
    reader::set_ai_visible(prefs::get().summaries);
    let _ = with_db(|db| db.set_poll_minutes(prefs::get().poll_minutes));
    if !prefs::get().recommendations && view() == View::ForYou {
        navigate(&View::All);
    } else {
        refresh_lists();
    }
}

pub fn search_active() -> bool {
    ui().map(|u| !u.borrow().search.text().trim().is_empty()).unwrap_or(false)
}

pub fn search_query() -> String {
    ui().map(|u| u.borrow().search.text().trim().to_string()).unwrap_or_default()
}

pub fn current_page_name() -> String {
    let Some(ui) = ui() else { return "Top Stories".into() };
    let v = ui.borrow().view.clone();
    if v == View::All {
        let s = timeline::section();
        if !s.is_empty() {
            return s;
        }
    }
    page_name(&v)
}

pub fn reload() {
    let Some(ui) = ui() else { return };
    let view = ui.borrow().view.clone();
    let query = ui.borrow().search.text().to_string();
    let loaded = with_db(|db| {
        db.set_poll_minutes(prefs::get().poll_minutes)?;
        match &view {
            View::ForYou => db.for_you(200),
            View::Topic(id) => db.topic_articles(id, 200),
            View::All if query.trim().is_empty() => {
                let section = timeline::section();
                if section.is_empty() { db.edition_articles(8) } else { db.section_articles(&section, "", 40) }
            }
            View::All => {
                let section = timeline::section();
                if section.is_empty() {
                    db.articles(&View::All, query.trim(), 200, 0)
                } else {
                    db.section_articles(&section, query.trim(), 200)
                }
            }
            other => db.articles(other, query.trim(), 200, 0),
        }
    });
    match loaded {
        Ok(items) => {
            ui.borrow_mut().articles = items;
            timeline::bind_articles();
        }
        Err(err) => toast(&err.to_string()),
    }
}

pub fn selected_id() -> Option<String> {
    ui()?.borrow().selected.clone()
}

pub fn select_id(id: Option<String>) {
    if let Some(ui) = ui() {
        ui.borrow_mut().selected = id;
    }
}

pub fn move_selection(delta: i32) {
    let Some(ui) = ui() else { return };
    let articles = ui.borrow().articles.clone();
    if articles.is_empty() {
        return;
    }
    let current = ui.borrow().selected.clone();
    let index = current.and_then(|id| articles.iter().position(|a| a.id == id)).unwrap_or(0);
    let next = (index as i32 + delta).clamp(0, articles.len() as i32 - 1) as usize;
    let id = articles[next].id.clone();
    ui.borrow_mut().selected = Some(id.clone());
    timeline::select_position(next as u32);
    reader::show(&articles[next]);
}

fn section_icon(name: &str) -> &'static str {
    match name {
        "World" => "web-browser-symbolic",
        "National" => "emoji-flags-symbolic",
        "Politics" => "system-users-symbolic",
        "Business" => "x-office-spreadsheet-symbolic",
        "Technology" => "computer-symbolic",
        "Science" => "applications-science-symbolic",
        "Health" => "face-smile-symbolic",
        "Sports" => "applications-games-symbolic",
        "Culture" => "applications-graphics-symbolic",
        _ => "view-list-symbolic",
    }
}

fn nav_button(icon: &str, title: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    nav_button_with_badge(icon, title, tooltip, 0)
}

fn nav_button_with_badge(icon: &str, title: &str, tooltip: &str, count: i64) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let label = widgets::label(title, "nav-label");
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.add_css_class("compact-hide");
    content.append(&label);
    if count > 0 {
        let badge = widgets::label(&count.to_string(), "nav-readout");
        badge.add_css_class("compact-hide");
        content.append(&badge);
    }
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    (button, label)
}

fn rebuild_nav() {
    let Some(ui) = ui() else { return };
    let list = ui.borrow().nav_list.clone();
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let current = section_id(&ui.borrow().view);
    let discover_open = timeline::discover_visible();
    let open_section = timeline::section();

    let group = widgets::label("TODAY", "nav-group");
    group.add_css_class("compact-hide");
    group.add_css_class("first");
    list.append(&group);

    let (headlines, _) = nav_button("view-list-symbolic", "Top Stories", "Today's top news");
    if !discover_open && current == "all" && open_section.is_empty() {
        headlines.add_css_class("active");
    }
    headlines.connect_clicked(|_| {
        timeline::show_section("");
        navigate(&View::All);
    });
    list.append(&headlines);

    let (for_you, _) = nav_button("starred-symbolic", "For You", "Stories recommended for you");
    if !discover_open && current == "foryou" {
        for_you.add_css_class("active");
    }
    for_you.connect_clicked(|_| navigate(&View::ForYou));
    list.append(&for_you);

    let (_, _, saved_c) = with_db(|db| db.counts()).unwrap_or((0, 0, 0));
    let (saved, _) = nav_button_with_badge("bookmark-new-symbolic", "Saved", "Stories saved to read later", saved_c);
    if !discover_open && current == "saved" {
        saved.add_css_class("active");
    }
    saved.connect_clicked(|_| navigate(&View::Saved));
    list.append(&saved);

    let group = widgets::label("TOPICS", "nav-group");
    group.add_css_class("compact-hide");
    list.append(&group);

    let disabled = prefs::get().disabled_categories;
    let sections: Vec<_> = nyx_news_core::directory()
        .map(|topics| topics.into_iter().map(|topic| topic.title).filter(|name| !disabled.contains(name)).collect::<Vec<_>>())
        .unwrap_or_default();
    for name in sections {
        let (button, _) = nav_button(section_icon(&name), &name, &format!("{name} news"));
        if !discover_open && current == "all" && open_section == name {
            button.add_css_class("active");
        }
        let section = name.clone();
        button.connect_clicked(move |_| {
            timeline::show_section(&section);
            navigate(&View::All);
            worker::open_section(section.clone());
        });
        list.append(&button);
    }

    let topics = with_db(|db| db.smart_filters()).unwrap_or_default();
    for topic in topics {
        let (button, _) = nav_button("tag-symbolic", &topic.name, &format!("Articles matching {}", topic.name));
        let id = topic.id.clone();
        if current == format!("topic:{id}") && !discover_open {
            button.add_css_class("active");
        }
        button.connect_clicked(move |_| navigate(&View::Topic(id.clone())));
        list.append(&button);
    }

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    list.append(&spacer);

    let (sources, _) = nav_button("find-location-symbolic", "Channels & Topics", "Browse and follow topics and publications");
    if discover_open {
        sources.add_css_class("active");
    }
    sources.connect_clicked(|_| show_discover());
    list.append(&sources);

    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn build(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder().application(app).title("News").default_width(1280).default_height(800).build();
    window.add_css_class("news-window");
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some("io.github.design_nexus.nyx.News"));
    nyx_ui::keys::install(
        &window,
        nyx_ui::keys::Common {
            settings: Some(settings_dialog::open),
            shortcuts: Some(show_shortcuts),
            close: true,
            ..Default::default()
        },
    );

    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_hexpand(false);
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hexpand(false)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    // The search, always in the header (Ctrl+F or /).
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search articles  (Ctrl+F)"));
    search.add_css_class("bar-search");
    search.set_width_chars(SEARCH_WIDE);
    search.update_property(&[gtk::accessible::Property::Label("Search articles")]);
    search.connect_search_changed(|_| reload());
    search.connect_stop_search(|entry| entry.set_text(""));

    let pages = timeline::build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("settings-content");
    content.set_hexpand(true);
    content.append(&pages);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.append(&nav);
    body.append(&content);

    // ----- Header -----
    // The app and the page, the sidebar toggle; the search in the middle; Settings at
    // the end. The page's own toolbar carries its count, Refresh and layout.
    let header = chrome::app_header(&window, "application-rss+xml-symbolic", "News");
    let collapse = widgets::bar_button("sidebar-show-symbolic", "Collapse the sidebar (Ctrl+B)");
    collapse.connect_clicked(|_| toggle_sidebar());
    header.start.append(&collapse);
    header.center.append(&search);
    header.end.append(&chrome::settings_button(settings_dialog::open));

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("window-frame");
    frame.append(&header.root);
    frame.append(&body);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    window.set_child(Some(&overlay));
    install_keys(&window, &search);

    let ui = Ui {
        window: window.clone(),
        overlay: overlay.clone(),
        nav: nav.clone(),
        nav_list: list,
        search,
        header,
        view: View::All,
        articles: Vec::new(),
        selected: None,
    };
    UI.with(|slot| *slot.borrow_mut() = Some(Rc::new(RefCell::new(ui))));

    let apply_width = {
        let nav = nav.clone();
        let window = window.clone();
        move || {
            let width = if window.width() > 0 { window.width() } else { window.default_width() };
            let narrow = width > 0 && width < 980;
            settings_dialog::fit(&window);
            let prev = NARROW.with(|n| n.get());
            let sized = nav.has_css_class("sized");
            if narrow == prev && sized {
                return;
            }
            nav.add_css_class("sized");
            NARROW.with(|n| n.set(narrow));
            apply_compact(&nav, narrow || prefs::get().sidebar_collapsed);
            // A tiled half-screen header gets a shorter search.
            if let Some(ui) = self::ui() {
                ui.borrow().search.set_width_chars(if narrow { SEARCH_NARROW } else { SEARCH_WIDE });
            }
            if sized && prev != narrow {
                reload();
            }
        }
    };
    let aw = apply_width.clone();
    window.connect_realize(move |_| aw());
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    probe.set_can_focus(false);
    overlay.add_overlay(&probe);
    overlay.set_measure_overlay(&probe, false);
    probe.connect_resize(move |_, _, _| {
        let aw = apply_width.clone();
        glib::idle_add_local_once(aw);
    });
    rebuild_nav();
}

/// The search's width in characters, in a full window and a narrow one.
const SEARCH_WIDE: i32 = 30;
const SEARCH_NARROW: i32 = 14;

fn apply_compact(nav: &gtk::Box, compact: bool) {
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }
    fn walk(widget: &gtk::Widget, compact: bool) {
        if widget.has_css_class("compact-hide") {
            widget.set_visible(!compact);
        }
        // Icon-only: centre the icon in its row. The header buttons are already centred.
        if widget.has_css_class("nav-item")
            && let Some(content) = widget.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            walk(&next, compact);
            child = next.next_sibling();
        }
    }
    walk(nav.upcast_ref(), compact);
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    let Some(ui) = ui() else { return };
    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn typing(window: &gtk::Window) -> bool {
    let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) else { return false };
    focus.is::<gtk::Text>() || focus.is::<gtk::TextView>() || focus.ancestor(gtk::Entry::static_type()).is_some()
}

fn install_keys(window: &gtk::ApplicationWindow, search: &gtk::SearchEntry) {
    let keys = gtk::EventControllerKey::new();
    let search = search.clone();
    let win = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        if ctrl && key == gdk::Key::f {
            search.grab_focus();
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::n {
            dialogs::add_feed();
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::i {
            dialogs::import_opml(&win);
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::e {
            dialogs::export_opml(&win);
            return glib::Propagation::Stop;
        }
        if ctrl && key == gdk::Key::b {
            toggle_sidebar();
            return glib::Propagation::Stop;
        }
        if typing(win.upcast_ref()) {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::j | gdk::Key::Down => move_selection(1),
            gdk::Key::k | gdk::Key::Up => move_selection(-1),
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(id) = selected_id()
                    && let Some(article) = find_article(&id)
                {
                    reader::show(&article);
                }
            }
            gdk::Key::b | gdk::Key::s => reader::toggle_save(),
            gdk::Key::o => reader::open_original(),
            gdk::Key::r => worker::refresh_current(),
            gdk::Key::slash => {
                search.grab_focus();
            }
            gdk::Key::Escape => {
                if !search.text().is_empty() {
                    search.set_text("");
                } else {
                    reader::hide();
                    timeline::show_list();
                }
            }
            gdk::Key::F1 => show_shortcuts(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

pub fn show_shortcuts() {
    let (dialog, card) = widgets::dialog("Keyboard shortcuts", 520);
    let list = widgets::vbox(0);
    list.add_css_class("group-list");
    for (keys, what) in SHORTCUTS {
        list.append(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&list)
        .build();
    card.append(&scroll);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let dialog2 = dialog.clone();
    close.connect_clicked(move |_| dialog2.close());
    card.append(&close);
    dialog.present();
}

pub fn notify(title: &str, body: &str) {
    let Some(window) = window() else {
        return;
    };
    if window.is_visible() && window.is_active() {
        toast(&format!("{title}: {body}"));
        return;
    }
    let Some(app) = window.application() else { return };
    let notification = gio::Notification::new(title);
    notification.set_body(Some(body));
    notification.set_icon(&gio::ThemedIcon::new("io.github.design_nexus.nyx.News"));
    app.send_notification(Some("news-refresh"), &notification);
}

pub fn toast(message: &str) {
    let Some(ui) = ui() else {
        eprintln!("nyx-news: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    if let Some(old) = TOAST.with(|t| t.borrow_mut().take())
        && old.parent().is_some()
    {
        overlay.remove_overlay(&old);
    }
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(70);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bx.add_css_class("toast");
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    bx.set_margin_bottom(48);
    bx.append(&label);
    overlay.add_overlay(&bx);
    TOAST.with(|t| *t.borrow_mut() = Some(bx.clone()));
    let overlay = overlay.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(3500), move || {
        if bx.parent().is_some() {
            overlay.remove_overlay(&bx);
        }
    });
}
