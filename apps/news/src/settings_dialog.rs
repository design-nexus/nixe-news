//! Settings as a card over the window.

use crate::settings_page;
use crate::widgets;
use crate::window;
use gtk::prelude::*;
use nixe_ui::settings_card::{self, Spec};

pub use nixe_ui::settings_card::fit;

const GROUPS: &[(&str, &str, &str)] = &[
    ("News window", "applications-graphics-symbolic", "Theme, glow, motion"),
    ("Reading", "view-list-symbolic", "Layout, retention, read state"),
    ("News Topics", "tag-symbolic", "Enable or disable front page categories"),
    ("Refresh", "view-refresh-symbolic", "How often feeds update"),
    ("Notifications", "preferences-system-notifications-symbolic", "New articles and failures"),
    ("Sync", "folder-remote-symbolic", "Nextcloud WebDAV"),
    ("Intelligence", "system-run-symbolic", "Recommendations and summaries"),
    ("Keyboard", "input-keyboard-symbolic", "Shortcuts"),
];

const SPEC: Spec = Spec { groups: GROUPS, search_text: None };

pub fn open() {
    let (Some(overlay), Some(window)) = (window::overlay(), window::window()) else { return };
    settings_card::open(&overlay, &window, SPEC, || {
        let page = widgets::page("settings");
        settings_page::build(&page);
        (page.root.upcast(), page.body)
    });
}
