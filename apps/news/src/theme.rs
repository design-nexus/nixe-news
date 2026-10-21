//! Theming: the shared Nixe theme engine, given this app's stylesheet.

pub use nixe_theme::*;

use crate::prefs;

pub fn install() {
    nixe_theme::install(Style {
        window_class: "news-window",
        base_skip: &[],
        css: concat!(include_str!("style.css"), "\n", include_str!("paper.css")),
        no_glow_css: "",
        extra_colors: None,
        prefs: || {
            let p = prefs::get();
            ThemePrefs { mode: p.mode, theme: p.theme, glow: p.glow, reduce_motion: p.reduce_motion }
        },
        set_theme: Some(|mode, theme| {
            prefs::update(|p| {
                p.mode = mode;
                p.theme = theme;
            });
        }),
    });
}
