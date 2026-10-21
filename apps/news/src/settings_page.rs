//! The settings card's groups.

use crate::prefs;
use crate::theme;
use crate::widgets::{self, Page};
use crate::window;
use crate::worker;
use gtk::prelude::*;

pub fn build(page: &Page) {
    window_group(page);
    reading(page);
    categories(page);
    refresh(page);
    notifications(page);
    sync(page);
    intelligence(page);
    keyboard(page);
}

fn window_group(page: &Page) {
    let group = page.group("News window");
    group.add(&nixe_ui::widgets::start_row());
    let prefs = prefs::get();
    let options: Vec<(String, String)> = theme::all().iter().map(|t| (t.id.clone(), t.name.clone())).collect();
    let (theme_row, theme_dd) =
        widgets::choice_row("Theme", "A bundled theme, or one of your own.", options, &prefs.theme, |id| {
            prefs::update(|p| {
                p.theme = id;
                p.mode = prefs::ThemeMode::Theme;
            });
            theme::apply();
        });
    theme_dd.set_sensitive(prefs.mode == prefs::ThemeMode::Theme || !theme::omarchy_available());
    if theme::omarchy_available() {
        let dd = theme_dd.clone();
        let (row, _) = widgets::switch_row(
            "Follow Omarchy theme",
            "Match the desktop's colors and update when the Omarchy theme changes.",
            prefs.mode == prefs::ThemeMode::Omarchy,
            move |on| {
                prefs::update(|p| p.mode = if on { prefs::ThemeMode::Omarchy } else { prefs::ThemeMode::Theme });
                dd.set_sensitive(!on);
                theme::apply();
            },
        );
        group.add(&row);
    }
    group.add(&theme_row);
    let (row, _) = widgets::switch_row("Glow", "Soft accent glow around focused and selected elements.", prefs.glow, |on| {
        prefs::update(|p| p.glow = on);
        theme::apply();
    });
    group.add(&row);
    let (row, _) = widgets::switch_row("Reduce motion", "Turn off transitions in this window.", prefs.reduce_motion, |on| {
        prefs::update(|p| p.reduce_motion = on);
        theme::apply();
    });
    group.add(&row);
}

fn reading(page: &Page) {
    let group = page.group("Reading");
    let prefs = prefs::get();
    group.add(&widgets::segmented_row(
        "Layout",
        "Show cover cards and excerpts, or a compact headline list.",
        widgets::opts(&[("list", "Covers"), ("compact", "Compact")]),
        &prefs.layout,
        |layout| {
            prefs::update(|p| p.layout = layout);
            window::reload();
        },
    ));
    group.add(&widgets::segmented_row(
        "Reader text size",
        "Adjust reading font size in the article pane.",
        widgets::opts(&[("small", "Small"), ("medium", "Standard"), ("large", "Large")]),
        &prefs.reader_font_scale,
        |size| {
            prefs::update(|p| p.reader_font_scale = size);
            crate::reader::update_font_size();
        },
    ));
    group.add(&widgets::segmented_row(
        "Reader typography",
        "Choose font style for article reading.",
        widgets::opts(&[("serif", "Serif"), ("sans", "Sans"), ("mono", "Monospace")]),
        &prefs.reader_font,
        |font| {
            prefs::update(|p| p.reader_font = font);
            crate::reader::update_font_size();
            window::reload();
        },
    ));
    let (row, _) = widgets::switch_row("Mark read on open", "Opening an article marks it read.", prefs.mark_read_on_open, |on| {
        prefs::update(|p| p.mark_read_on_open = on);
    });
    group.add(&row);
    let (row, _) = widgets::switch_row(
        "Keep article text",
        "Store article text on this computer for offline reading.",
        prefs.keep_content,
        |on| {
            prefs::update(|p| p.keep_content = on);
        },
    );
    group.add(&row);
    let spin = gtk::SpinButton::with_range(7.0, 3650.0, 1.0);
    spin.set_value(prefs.retention_days as f64);
    spin.connect_value_changed(|spin| {
        prefs::update(|p| p.retention_days = spin.value() as i64);
    });
    group.add(&widgets::row("Keep articles", "Days to keep articles that are not starred or saved.", Some(spin.upcast_ref())));
    let (row, _) = widgets::switch_row(
        "Assistant can follow topics",
        "Let Nixe Assistant follow topics and channels on your behalf.",
        prefs.assistant_can_subscribe,
        |on| prefs::update(|p| p.assistant_can_subscribe = on),
    );
    group.add(&row);
}

fn categories(page: &Page) {
    let group = page.group("News Topics");
    group.note("Choose which topic shelves appear on your front page and sidebar.");
    let all_topics = nixe_news_core::directory()
        .map(|topics| topics.into_iter().map(|topic| topic.title).collect::<Vec<_>>())
        .unwrap_or_default();
    for topic_name in all_topics {
        let name = topic_name.clone();
        let disabled = prefs::get().disabled_categories.contains(&name);
        let (row, _) = widgets::switch_row(
            &topic_name,
            &format!("Show {topic_name} stories on the front page and sidebar."),
            !disabled,
            move |on| {
                prefs::update(|p| {
                    if on {
                        p.disabled_categories.retain(|c| c != &name);
                    } else if !p.disabled_categories.contains(&name) {
                        p.disabled_categories.push(name.clone());
                    }
                });
                window::refresh_lists();
            },
        );
        group.add(&row);
    }
}

fn refresh(page: &Page) {
    let group = page.group("Refresh");
    let prefs = prefs::get();
    let spin = gtk::SpinButton::with_range(15.0, 1440.0, 15.0);
    spin.set_value(prefs.poll_minutes as f64);
    spin.connect_value_changed(|spin| {
        let minutes = spin.value() as i64;
        prefs::update(|p| p.poll_minutes = minutes);
        let _ = window::with_db(|db| db.set_poll_minutes(minutes));
    });
    group.add(&widgets::row("Check every", "Minutes between news updates. 15 is the shortest.", Some(spin.upcast_ref())));
    let (row, _) =
        widgets::switch_row("Refresh on startup", "Check for new stories when News opens.", prefs.refresh_on_startup, |on| {
            prefs::update(|p| p.refresh_on_startup = on);
        });
    group.add(&row);
}

fn notifications(page: &Page) {
    let group = page.group("Notifications");
    let prefs = prefs::get();
    let (row, _) = widgets::switch_row(
        "New articles",
        "Tell you when a refresh finds articles that were not here before.",
        prefs.notify_articles,
        |on| prefs::update(|prefs| prefs.notify_articles = on),
    );
    group.add(&row);
    let (row, _) =
        widgets::switch_row("Update failures", "Tell you when stories could not be refreshed.", prefs.notify_failures, |on| {
            prefs::update(|prefs| prefs.notify_failures = on)
        });
    group.add(&row);
}

fn sync(page: &Page) {
    let group = page.group("Sync");
    group.note("The app password is stored in the keyring. News only sends folders, feeds, and read state over HTTPS.");
    let prefs = prefs::get();
    let server = text_field("https://cloud.example", &prefs.server);
    server.connect_changed(|entry| {
        let text = entry.text().to_string();
        prefs::update(|prefs| prefs.server = text);
    });
    group.add(&field_row("Server", "The Nextcloud address, including https://.", &server));
    let username = text_field("Nextcloud username", &prefs.username);
    username.connect_changed(|entry| {
        let text = entry.text().to_string();
        prefs::update(|prefs| prefs.username = text);
    });
    group.add(&field_row("Username", "", &username));
    let folder = text_field("Nixe/News", &prefs.folder);
    folder.connect_changed(|entry| {
        let text = entry.text().to_string();
        prefs::update(|prefs| prefs.folder = text);
    });
    group.add(&field_row("Folder", "Path inside the account. Nixe/News is the usual place.", &folder));
    let password = gtk::PasswordEntry::builder().show_peek_icon(true).hexpand(true).build();
    password.set_placeholder_text(Some("App password"));
    password.connect_activate(move |entry| {
        let _ = save_password(&entry.text());
    });
    group.add(&field_row("App password", "Leave this blank to keep the password already in the keyring.", &password));

    let switch_slot: std::rc::Rc<std::cell::RefCell<Option<gtk::Switch>>> = std::rc::Rc::new(std::cell::RefCell::new(None));
    let password_for_switch = password.clone();
    let hold_for_switch = std::rc::Rc::new(std::cell::Cell::new(false));
    let slot_for_switch = switch_slot.clone();
    let (row, switch) = widgets::switch_row(
        "Sync subscriptions",
        "Send folders, feeds, and read state to Nextcloud.",
        prefs.sync_enabled,
        move |on| {
            if hold_for_switch.get() {
                return;
            }
            if on && !password_for_switch.text().is_empty() && !save_password(&password_for_switch.text()) {
                hold_for_switch.set(true);
                if let Some(switch) = slot_for_switch.borrow().clone() {
                    switch.set_active(false);
                }
                hold_for_switch.set(false);
                return;
            }
            if on && password_for_switch.text().is_empty() && !keyring_has_password() {
                window::toast("Save an app password before turning sync on.");
                hold_for_switch.set(true);
                if let Some(switch) = slot_for_switch.borrow().clone() {
                    switch.set_active(false);
                }
                hold_for_switch.set(false);
                return;
            }
            prefs::update(|prefs| prefs.sync_enabled = on);
        },
    );
    *switch_slot.borrow_mut() = Some(switch);
    group.add(&row);

    let (row, _) = widgets::button_row("Sync now", "Send this computer's changes and pull the others.", "Sync now", move |_| {
        if !password.text().is_empty() && !save_password(&password.text()) {
            return;
        }
        if !prefs::get().sync_enabled {
            window::toast("Turn on sync before syncing now.");
            return;
        }
        worker::sync_now();
    });
    group.add(&row);
}

fn text_field(placeholder: &str, value: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(value);
    entry.set_hexpand(true);
    entry.set_width_chars(48);
    entry
}

fn field_row(title: &str, desc: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.add_css_class("settings-option");
    row.add_css_class("tall");
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("settings-option-title");
    heading.set_xalign(0.0);
    row.append(&heading);
    if !desc.is_empty() {
        let note = gtk::Label::new(Some(desc));
        note.add_css_class("settings-option-description");
        note.set_xalign(0.0);
        note.set_wrap(true);
        row.append(&note);
    }
    control.set_hexpand(true);
    row.append(control);
    row
}

fn keyring_has_password() -> bool {
    let Ok(device) = window::with_db(|db| db.device_id()) else { return false };
    nixe_sync::lookup_secret("nixe-news", &device, "nextcloud").is_ok()
}

fn save_password(text: &str) -> bool {
    let Ok(secret) = nixe_sync::Secret::new(text.trim()) else {
        window::toast("That password cannot be stored.");
        return false;
    };
    let Ok(device) = window::with_db(|db| db.device_id()) else {
        window::toast("News could not open its library.");
        return false;
    };
    if nixe_sync::store_secret("nixe-news", &device, "nextcloud", &secret).is_err() {
        window::toast("The keyring did not store the password.");
        return false;
    }
    true
}

fn intelligence(page: &Page) {
    let group = page.group("Intelligence");
    group.note(
        "Recommendations stay on this computer. They use stars, tags, and folder names. Summaries ask Assistant and stay off until you turn them on.",
    );
    let (row, switch) =
        widgets::switch_row("Recommendations", "Show a For You page from what you read.", prefs::get().recommendations, |on| {
            prefs::update(|prefs| prefs.recommendations = on);
            if !on && window::view() == nixe_news_core::View::ForYou {
                window::navigate(&nixe_news_core::View::All);
            } else {
                window::refresh_lists();
            }
        });
    switch.set_sensitive(true);
    group.add(&row);
    let (row, switch) = widgets::switch_row(
        "Summaries",
        "Ask Nixe Assistant for an original summary. The article stays on screen if Assistant is down.",
        prefs::get().summaries,
        |on| {
            prefs::update(|prefs| prefs.summaries = on);
            crate::reader::set_ai_visible(on);
        },
    );
    switch.set_sensitive(true);
    group.add(&row);
}

fn keyboard(page: &Page) {
    let group = page.group("Keyboard");
    for (keys, what) in window::SHORTCUTS {
        group.add(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
}
