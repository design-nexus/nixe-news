//! Add a feed, and import or export OPML.

use crate::widgets;
use crate::window;
use crate::worker;
use gtk::prelude::*;
use nixe_news_core::canonical_url;

pub fn add_feed() {
    let (dialog, card) = widgets::dialog("Add a feed", 480);
    let hint = widgets::label("RSS, Atom, or JSON Feed address", "dim");
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    card.append(&hint);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("https://example.com/feed.xml"));
    entry.set_hexpand(true);
    entry.set_width_chars(48);
    card.append(&entry);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let dialog_cancel = dialog.clone();
    cancel.connect_clicked(move |_| dialog_cancel.close());
    let add = gtk::Button::with_label("Add");
    add.add_css_class("suggested-action");
    let dialog_add = dialog.clone();
    let entry_add = entry.clone();
    add.connect_clicked(move |_| submit(&entry_add.text(), &dialog_add));
    let entry_enter = entry.clone();
    let dialog_enter = dialog.clone();
    entry.connect_activate(move |_| submit(&entry_enter.text(), &dialog_enter));
    buttons.append(&cancel);
    buttons.append(&add);
    card.append(&buttons);
    dialog.present();
}

fn submit(text: &str, dialog: &gtk::Window) {
    let text = text.trim();
    if text.is_empty() {
        window::toast("Enter a feed address.");
        return;
    }
    if canonical_url(text).is_err() && !text.starts_with("http://") && !text.starts_with("https://") {
        window::toast("That address isn't a feed URL.");
        return;
    }
    worker::add_url(text.to_string());
    dialog.close();
    window::toast("Checking the feed…");
}

pub fn import_opml(window: &gtk::ApplicationWindow) {
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Import OPML");
    let parent = window.clone();
    dialog.open(Some(&parent), None::<&gtk::gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else {
            window::toast("That file couldn't be read.");
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match window::with_db(|db| db.import_opml(&text)) {
                Ok((added, existing)) => {
                    window::reload();
                    window::toast(&format!("Imported {added} feeds, {existing} already there."));
                }
                Err(err) => window::toast(&err.to_string()),
            },
            Err(err) => window::toast(&err.to_string()),
        }
    });
}

pub fn export_markdown(window: &gtk::ApplicationWindow, markdown: String, name: String) {
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Export Markdown");
    dialog.set_initial_name(Some(&name));
    let parent = window.clone();
    dialog.save(Some(&parent), None::<&gtk::gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        match std::fs::write(&path, &markdown) {
            Ok(()) => window::toast("Exported the article."),
            Err(err) => window::toast(&err.to_string()),
        }
    });
}

pub fn export_opml(window: &gtk::ApplicationWindow) {
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Export OPML");
    dialog.set_initial_name(Some("nixe-news.opml"));
    let parent = window.clone();
    dialog.save(Some(&parent), None::<&gtk::gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        match window::with_db(|db| db.export_opml()) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&path, text) {
                    window::toast(&err.to_string());
                } else {
                    window::toast("Exported subscriptions.");
                }
            }
            Err(err) => window::toast(&err.to_string()),
        }
    });
}
