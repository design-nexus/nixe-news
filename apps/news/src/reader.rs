//! The story. Opening one replaces the paper and loads the article text.

use crate::assistant_client;
use crate::cmd;
use crate::dialogs;
use crate::html_text;
use crate::note;
use crate::paths;
use crate::prefs;
use crate::source;
use crate::timeline;
use crate::widgets;
use crate::window;
use crate::worker;
use gtk::prelude::*;
use nixe_news_core::{AiFailure, ArticleListItem, Excerpt, finish};
use std::cell::{Cell, RefCell};

thread_local! {
    static TITLE: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static KICKER: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static COVER: RefCell<Option<gtk::Picture>> = const { RefCell::new(None) };
    static COVER_WRAP: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
    static META: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static BODY: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static BRIEF_BODY: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static BRIEF_CARD: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static AI_ROW: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static PROGRESS: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static PROGRESS_TEXT: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static OPEN_ID: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPEN_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    static SAVE: RefCell<Option<gtk::Button>> = const { RefCell::new(None) };
    static PUBLISHER_BOX: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

const UNAVAILABLE: &str = "This story did not open. It is still at the publisher.";

pub fn build() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("reader-view");
    root.set_hexpand(true);
    root.set_vexpand(true);

    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.add_css_class("toolbar");

    let back = widgets_button("go-previous-symbolic", "Back to stories");
    back.connect_clicked(|_| hide());
    toolbar.append(&back);

    let back_label = widgets::label("Back", "toolbar-title");
    toolbar.append(&back_label);

    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    toolbar.append(&gap);

    let ai = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    ai.set_visible(prefs::get().summaries);
    for (icon, tip, task) in [
        ("document-edit-symbolic", "Summarize", "summary"),
        ("view-paged-symbolic", "Brief", "brief"),
        ("view-dual-symbolic", "Compare", "compare"),
        ("audio-volume-high-symbolic", "Speak", "spoken"),
    ] {
        let button = widgets_button(icon, tip);
        button.connect_clicked(move |_| start_brief(task));
        ai.append(&button);
    }
    toolbar.append(&ai);

    let save = widgets_button("bookmark-new-symbolic", "Save story (B)");
    save.connect_clicked(|_| toggle_save());

    let copy = widgets_button("edit-copy-symbolic", "Copy story link");
    copy.connect_clicked(|_| {
        if let Some(url) = OPEN_URL.with(|p| p.borrow().clone())
            && let Some(display) = gtk::gdk::Display::default()
        {
            display.clipboard().set_text(&url);
            window::toast("Story link copied to clipboard");
        }
    });

    let font_toggle = widgets_button("format-text-larger-symbolic", "Change text size");
    font_toggle.connect_clicked(|_| {
        let next = match prefs::get().reader_font_scale.as_str() {
            "small" => "medium",
            "medium" => "large",
            _ => "small",
        };
        prefs::update(|p| p.reader_font_scale = next.into());
        update_font_size();
        window::toast(&format!("Text size: {next}"));
    });

    let open = widgets_button("external-link-symbolic", "Read at publisher / Open in browser (O)");
    open.connect_clicked(|_| open_original());
    let export = widgets_button("document-save-symbolic", "Export note");
    export.connect_clicked(|_| export_note());

    toolbar.append(&save);
    toolbar.append(&copy);
    toolbar.append(&font_toggle);
    toolbar.append(&open);
    toolbar.append(&export);

    root.append(&toolbar);

    let pane = gtk::Box::new(gtk::Orientation::Vertical, 16);
    pane.add_css_class("reader");
    pane.set_hexpand(true);
    pane.set_margin_start(28);
    pane.set_margin_end(28);
    pane.set_margin_top(20);
    pane.set_margin_bottom(64);

    let cover = gtk::Picture::new();
    cover.set_content_fit(gtk::ContentFit::Cover);
    cover.set_can_shrink(true);
    cover.set_height_request(280);
    cover.add_css_class("reader-cover");

    let cover_wrap = gtk::Overlay::new();
    cover_wrap.set_child(Some(&cover));
    cover_wrap.set_measure_overlay(&cover, false);
    cover_wrap.set_visible(false);

    let kicker = gtk::Label::new(None);
    kicker.add_css_class("reader-kicker");
    kicker.set_xalign(0.0);
    kicker.set_visible(false);

    let title = gtk::Label::new(None);
    title.add_css_class("reader-title");
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::Word);

    let meta = gtk::Label::new(None);
    meta.add_css_class("reader-meta");
    meta.set_xalign(0.0);
    meta.set_wrap(true);

    let progress = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    progress.add_css_class("reader-brief-card");
    progress.set_visible(false);
    let progress_text = gtk::Label::new(Some("Writing a summary…"));
    progress_text.set_xalign(0.0);
    progress_text.set_hexpand(true);
    let cancel = widgets_button("window-close-symbolic", "Cancel");
    cancel.connect_clicked(|_| cancel_brief());
    progress.append(&progress_text);
    progress.append(&cancel);

    let brief_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    brief_card.add_css_class("reader-brief-card");
    let brief_title = widgets::label("AI INTELLIGENCE", "reader-brief-title");
    let brief_body = gtk::Label::new(None);
    brief_body.add_css_class("reader-brief-body");
    brief_body.set_xalign(0.0);
    brief_body.set_wrap(true);
    brief_card.append(&brief_title);
    brief_card.append(&brief_body);
    brief_card.set_visible(false);

    let body = gtk::Label::new(None);
    body.add_css_class("reader-body");
    body.set_hexpand(true);
    body.set_xalign(0.0);
    body.set_yalign(0.0);
    body.set_wrap(true);
    body.set_wrap_mode(gtk::pango::WrapMode::Word);

    pane.append(&cover_wrap);
    pane.append(&kicker);
    pane.append(&title);
    pane.append(&meta);
    pane.append(&progress);
    pane.append(&brief_card);
    pane.append(&body);

    let pub_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    pub_box.add_css_class("publisher-box");
    pub_box.set_visible(false);

    let pub_notice = gtk::Label::new(Some("Full article could not be formatted for the reader. Read the original article at the publisher in your browser."));
    pub_notice.add_css_class("publisher-notice");
    pub_notice.set_xalign(0.0);
    pub_notice.set_wrap(true);
    pub_box.append(&pub_notice);

    let pub_btn = gtk::Button::with_label("Read full story at publisher ↗");
    pub_btn.add_css_class("publisher-btn");
    pub_btn.set_halign(gtk::Align::Start);
    pub_btn.connect_clicked(|_| open_original());
    pub_box.append(&pub_btn);

    pane.append(&pub_box);

    let scroll = gtk::ScrolledWindow::builder()
        .child(&pane)
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .build();

    let swipe = gtk::GestureSwipe::new();
    swipe.connect_swipe(|_, vx, _vy| {
        if vx > 150.0 {
            hide();
        }
    });
    scroll.add_controller(swipe);

    let scroll_ctrl = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::HORIZONTAL
            | gtk::EventControllerScrollFlags::VERTICAL
            | gtk::EventControllerScrollFlags::KINETIC,
    );
    let accum_dx = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
    let last_trigger = std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now()));
    let accum = accum_dx.clone();
    let last = last_trigger.clone();
    scroll_ctrl.connect_scroll(move |_, dx, dy| {
        if dx.abs() > dy.abs() * 1.5 {
            let current = accum.get() + dx;
            accum.set(current);
            if current.abs() > 30.0 && last.get().elapsed().as_millis() > 500 {
                last.set(std::time::Instant::now());
                accum.set(0.0);
                hide();
                return gtk::glib::Propagation::Stop;
            }
        } else {
            accum.set(0.0);
        }
        gtk::glib::Propagation::Proceed
    });
    let accum_dec = accum_dx.clone();
    scroll_ctrl.connect_decelerate(move |_, _, _| {
        accum_dec.set(0.0);
    });
    scroll.add_controller(scroll_ctrl);

    root.append(&scroll);

    TITLE.with(|p| *p.borrow_mut() = Some(title));
    KICKER.with(|p| *p.borrow_mut() = Some(kicker));
    COVER.with(|p| *p.borrow_mut() = Some(cover));
    COVER_WRAP.with(|p| *p.borrow_mut() = Some(cover_wrap));
    META.with(|p| *p.borrow_mut() = Some(meta));
    BODY.with(|p| *p.borrow_mut() = Some(body));
    BRIEF_BODY.with(|p| *p.borrow_mut() = Some(brief_body));
    BRIEF_CARD.with(|p| *p.borrow_mut() = Some(brief_card));
    AI_ROW.with(|p| *p.borrow_mut() = Some(ai));
    PROGRESS.with(|p| *p.borrow_mut() = Some(progress));
    PROGRESS_TEXT.with(|p| *p.borrow_mut() = Some(progress_text));
    SAVE.with(|p| *p.borrow_mut() = Some(save));
    PUBLISHER_BOX.with(|p| *p.borrow_mut() = Some(pub_box));
    root
}

fn widgets_button(icon: &str, tooltip: &str) -> gtk::Button {
    crate::widgets::bar_button(icon, tooltip)
}

pub fn show(article: &ArticleListItem) {
    if let Some(pub_box) = PUBLISHER_BOX.with(|p| p.borrow().clone()) {
        pub_box.set_visible(false);
    }
    if let Some(cover_wrap) = COVER_WRAP.with(|p| p.borrow().clone()) {
        if let Some(url) = article.image_url.as_ref().filter(|url| paths::cover_file(url).exists()) {
            if let Some(cover) = COVER.with(|p| p.borrow().clone()) {
                cover.set_filename(Some(paths::cover_file(url)));
            }
            cover_wrap.set_visible(true);
        } else {
            cover_wrap.set_visible(false);
        }
    }
    if let Some(kicker) = KICKER.with(|p| p.borrow().clone()) {
        let pub_name = source::name(&article.original_url, &article.feed_title);
        if !pub_name.is_empty() {
            kicker.set_text(&pub_name.to_uppercase());
            kicker.set_visible(true);
        } else {
            kicker.set_visible(false);
        }
    }
    if let Some(title) = TITLE.with(|p| p.borrow().clone()) {
        title.set_text(if article.title.is_empty() { "Untitled" } else { &article.title });
    }
    let cached = article
        .content_hash
        .as_ref()
        .and_then(|hash| window::with_db(|db| Ok(db.body(&paths::content_cache(), hash))).ok().flatten());
    let saved = std::fs::read_to_string(paths::story_file(&article.id)).ok().filter(|text| text.len() >= 400);
    let ready = saved.or_else(|| cached.map(|html| html_text::plain(&html)).filter(|t| t.len() >= 400));
    let feed_fallback = if !article.summary.trim().is_empty() {
        Some(html_text::plain(&article.summary))
    } else {
        None
    };
    let initial_text = ready.clone().or_else(|| feed_fallback.clone()).unwrap_or_else(|| "Opening the story…".into());
    if let Some(meta) = META.with(|p| p.borrow().clone()) {
        let byline = source::byline(&article.original_url, &article.feed_title, article.published_at);
        let read_time = source::read_time(&initial_text);
        let mut meta_line = format!("{byline} · {read_time}");
        if ready.is_some() {
            meta_line.push_str(" · Cached offline");
        }
        meta.set_text(&meta_line);
    }
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        body.set_text(&initial_text);
    }
    update_font_size();
    OPEN_ID.with(|p| *p.borrow_mut() = Some(article.id.clone()));
    OPEN_URL.with(|p| *p.borrow_mut() = Some(article.original_url.clone()));
    if ready.is_none() && (article.original_url.starts_with("https://") || article.original_url.starts_with("http://")) {
        worker::story(article.id.clone(), article.original_url.clone());
    }
    clear_brief();
    set_ai_visible(prefs::get().summaries);
    window::select_id(Some(article.id.clone()));
    if prefs::get().mark_read_on_open && !article.read {
        let _ = window::with_db(|db| db.set_read(&article.id, true));
        window::reload();
    }
    paint_actions();
    timeline::show_story();
}

pub fn replace_body(id: &str, text: &str) {
    let open = OPEN_ID.with(|p| p.borrow().clone());
    if open.as_deref() != Some(id) {
        return;
    }
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        body.set_text(if text.trim().is_empty() { UNAVAILABLE } else { text });
    }
    if let Some(pub_box) = PUBLISHER_BOX.with(|p| p.borrow().clone()) {
        pub_box.set_visible(false);
    }
}

pub fn unavailable(id: &str) {
    let open = OPEN_ID.with(|p| p.borrow().clone());
    if open.as_deref() != Some(id) {
        return;
    }
    let summary = window::find_article(id).map(|a| a.summary).unwrap_or_default();
    let text = html_text::plain(&summary);
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        if !text.trim().is_empty() {
            body.set_text(&text);
        } else {
            body.set_text(UNAVAILABLE);
        }
    }
    if let Some(pub_box) = PUBLISHER_BOX.with(|p| p.borrow().clone()) {
        pub_box.set_visible(true);
    }
}

pub fn update_font_size() {
    let scale = prefs::get().reader_font_scale;
    let font = prefs::get().reader_font;
    if let Some(body) = BODY.with(|p| p.borrow().clone()) {
        body.remove_css_class("font-small");
        body.remove_css_class("font-medium");
        body.remove_css_class("font-large");
        match scale.as_str() {
            "small" => body.add_css_class("font-small"),
            "large" => body.add_css_class("font-large"),
            _ => body.add_css_class("font-medium"),
        }
        body.remove_css_class("font-serif");
        body.remove_css_class("font-mono");
        body.remove_css_class("font-sans");
        match font.as_str() {
            "mono" => body.add_css_class("font-mono"),
            "sans" => body.add_css_class("font-sans"),
            _ => body.add_css_class("font-serif"),
        }
    }
}

pub fn hide() {
    timeline::close_story();
}

#[allow(dead_code)]
pub fn toggle_read() {
    let Some(id) = window::selected_id() else { return };
    let read = window::find_article(&id).is_some_and(|a| a.read);
    let _ = window::with_db(|db| db.set_read(&id, !read));
    window::reload();
    paint_actions();
}

#[allow(dead_code)]
pub fn toggle_star() {
    let Some(id) = window::selected_id() else { return };
    let starred = window::find_article(&id).is_some_and(|a| a.starred);
    let _ = window::with_db(|db| db.set_starred(&id, !starred));
    window::reload();
    paint_actions();
}

pub fn toggle_save() {
    let Some(id) = window::selected_id() else { return };
    let saved = window::find_article(&id).is_some_and(|a| a.saved);
    let _ = window::with_db(|db| db.set_saved(&id, !saved));
    window::toast(if saved { "Story removed from Saved" } else { "Story saved" });
    window::reload();
    paint_actions();
}

fn paint_actions() {
    let Some(id) = window::selected_id() else { return };
    let Some(article) = window::find_article(&id) else { return };
    if let Some(button) = SAVE.with(|p| p.borrow().clone()) {
        button.set_icon_name(if article.saved { "user-bookmarks-symbolic" } else { "bookmark-new-symbolic" });
        button.set_tooltip_text(Some(if article.saved { "Remove from Saved (B)" } else { "Save story (B)" }));
        if article.saved {
            button.add_css_class("active");
        } else {
            button.remove_css_class("active");
        }
    }
}

pub fn set_ai_visible(on: bool) {
    if let Some(row) = AI_ROW.with(|p| p.borrow().clone()) {
        row.set_visible(on);
    }
    if !on {
        cancel_brief();
    }
}

pub fn apply_brief(generation: u64, result: Result<String, AiFailure>) {
    if generation != GENERATION.get() {
        return;
    }
    hide_progress();
    let article = BODY.with(|p| p.borrow().as_ref().map(|label| label.text().to_string())).unwrap_or_default();
    let previous = current_brief();
    let pane = finish(&article, previous.as_deref(), result);
    if let Some(brief) = pane.brief.as_deref() {
        if let Some(label) = BRIEF_BODY.with(|p| p.borrow().clone()) {
            label.set_text(brief);
        }
        if let Some(card) = BRIEF_CARD.with(|p| p.borrow().clone()) {
            card.set_visible(true);
        }
    }
    if let Some(message) = pane.toast {
        window::toast(&message);
    }
}

fn start_brief(task: &str) {
    if !prefs::get().summaries {
        return;
    }
    let Some(id) = window::selected_id() else {
        window::toast("Open an article first.");
        return;
    };
    let Some(current) = window::find_article(&id) else {
        window::toast("Open an article first.");
        return;
    };
    let list = window::articles();
    let articles = match task {
        "brief" => {
            let mut picked: Vec<_> = list.iter().take(5).cloned().collect();
            if picked.is_empty() {
                picked.push(current);
            }
            picked
        }
        "compare" => {
            let Some(other) = list.iter().find(|article| article.id != current.id).cloned() else {
                window::toast("Two articles are needed to compare.");
                return;
            };
            vec![current, other]
        }
        _ => vec![current],
    };
    let excerpts: Vec<Excerpt> = articles.iter().map(excerpt_of).collect();
    let generation = bump_generation();
    if let Some(label) = PROGRESS_TEXT.with(|p| p.borrow().clone()) {
        label.set_text(match task {
            "brief" => "Writing a brief…",
            "compare" => "Comparing sources…",
            "spoken" => "Writing a spoken brief…",
            _ => "Writing a summary…",
        });
    }
    if let Some(row) = PROGRESS.with(|p| p.borrow().clone()) {
        row.set_visible(true);
    }
    worker::brief(task.to_string(), excerpts, generation);
}

fn cancel_brief() {
    let was_running = PROGRESS.with(|p| p.borrow().as_ref().is_some_and(|row| row.is_visible()));
    bump_generation();
    hide_progress();
    if was_running {
        assistant_client::cancel();
    }
}

fn bump_generation() -> u64 {
    GENERATION.with(|generation| {
        let next = generation.get().wrapping_add(1);
        generation.set(next);
        next
    })
}

fn hide_progress() {
    if let Some(row) = PROGRESS.with(|p| p.borrow().clone()) {
        row.set_visible(false);
    }
}

fn clear_brief() {
    let running = PROGRESS.with(|row| row.borrow().as_ref().is_some_and(|row| row.is_visible()));
    bump_generation();
    hide_progress();
    if let Some(label) = BRIEF_BODY.with(|p| p.borrow().clone()) {
        label.set_text("");
    }
    if let Some(card) = BRIEF_CARD.with(|p| p.borrow().clone()) {
        card.set_visible(false);
    }
    if running {
        assistant_client::cancel();
    }
}

fn current_brief() -> Option<String> {
    BRIEF_BODY.with(|p| p.borrow().as_ref().map(|label| label.text().to_string())).filter(|text| !text.is_empty())
}

fn excerpt_of(article: &ArticleListItem) -> Excerpt {
    let cached = article
        .content_hash
        .as_ref()
        .and_then(|hash| window::with_db(|db| Ok(db.body(&paths::content_cache(), hash))).ok().flatten());
    let text =
        cached.map(|html| html_text::plain(&html)).filter(|text| !text.is_empty()).unwrap_or_else(|| article.summary.clone());
    Excerpt { title: article.title.clone(), feed: article.feed_title.clone(), url: article.original_url.clone(), text }
}

fn export_note() {
    let Some(id) = window::selected_id() else {
        window::toast("Open an article first.");
        return;
    };
    let Some(article) = window::find_article(&id).or_else(|| window::with_db(|db| db.article(&id)).ok().flatten()) else {
        window::toast("Open an article first.");
        return;
    };
    let Some(parent) = window::window() else { return };
    dialogs::export_markdown(&parent, note::markdown(&article), note::filename(&article.title));
}

pub fn open_original() {
    let Some(url) = OPEN_URL.with(|p| p.borrow().clone()) else { return };
    if url.starts_with("https://") || url.starts_with("http://") {
        cmd::spawn(&["xdg-open", &url]);
    }
}
