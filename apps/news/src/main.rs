//! News — a local news reader for Omarchy.

mod assistant_client;
mod cmd;
mod control;
mod dialogs;
mod discover_page;
mod html_text;
mod note;
mod paths;
mod prefs;
mod reader;
mod settings_dialog;
mod settings_page;
mod source;
mod theme;
mod timeline;
mod widgets;
mod window;
mod worker;

use gtk::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.design_nexus.nixe.News";

const USAGE: &str = "Usage: nixe-news [OPTIONS]\n\
\n\
  --section ID    open Following, Discover, or settings\n\
  --toggle        close the window if it's open, otherwise open it\n";

fn main() -> glib::ExitCode {
    nixe_core::app::init(nixe_core::app::App { slug: "nixe-news", name: "News" });
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("GSK_RENDERER", "ngl") };
    }

    let app = gtk::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    app.connect_startup(|app| {
        theme::install();
        nixe_core::control::serve_async(app, control::tools(), control::handle);
    });
    app.connect_command_line(|app, cl| {
        let argv: Vec<String> = cl.arguments().iter().map(|a| a.to_string_lossy().to_string()).collect();
        let has = |flag: &str| argv.iter().any(|a| a == flag);
        let value = |flag: &str| argv.iter().position(|a| a == flag).and_then(|i| argv.get(i + 1)).cloned();
        if has("--toggle")
            && let Some(w) = window::window()
            && w.is_visible()
        {
            w.close();
            return glib::ExitCode::SUCCESS;
        }
        window::present(app, value("--section").as_deref());
        glib::ExitCode::SUCCESS
    });
    app.connect_shutdown(|_| {
        worker::shutdown();
        prefs::flush();
    });
    app.run()
}
