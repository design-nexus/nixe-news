//! Settings as a card over the window.

use crate::settings_page;
use crate::widgets;
use crate::window;
use gtk::prelude::*;
use nyx_ui::settings_card::{self, About, Spec, Standard};

pub use nyx_ui::settings_card::fit;

const GROUPS: &[(&str, &str, &str)] = &[
    ("Reading", "view-list-symbolic", "Layout, retention, read state"),
    ("News Topics", "tag-symbolic", "Enable or disable front page categories"),
    ("Refresh", "view-refresh-symbolic", "How often feeds update"),
    ("Notifications", "preferences-system-notifications-symbolic", "New articles and failures"),
    ("Sync", "folder-remote-symbolic", "Nextcloud WebDAV"),
    ("Intelligence", "system-run-symbolic", "Recommendations and summaries"),
];

const SPEC: Spec = Spec {
    groups: GROUPS,
    search_text: None,
    standard: Some(Standard {
        app: "news",
        about: About {
            name: "News",
            icon: "io.github.design_nexus.nyx.News",
            version: env!("CARGO_PKG_VERSION"),
            description: "Your RSS, Atom and JSON feeds, read on this computer.",
            licence: "MIT",
            website: "https://github.com/design-nexus/nyx-news",
            issues: "https://github.com/design-nexus/nyx-news/issues",
            credits: &[],
        },
        shortcuts: window::SHORTCUTS,
        reduce_motion: || crate::prefs::get().reduce_motion,
        set_reduce_motion: |on| crate::prefs::update(|p| p.reduce_motion = on),
    }),
};

pub fn open() {
    let (Some(overlay), Some(window)) = (window::overlay(), window::window()) else { return };
    settings_card::open(&overlay, &window, SPEC, || {
        let page = widgets::page("settings");
        settings_page::build(&page);
        (page.root.upcast(), page.body)
    });
}
