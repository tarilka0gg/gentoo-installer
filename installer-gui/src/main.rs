//! GTK4 + libadwaita wizard for the main ISO (niri + shell package preinstalled on target).
//! Steps mirror the CLI's, plus network config done inline in-app instead of TUI.

mod window;

use adw::prelude::*;
use gtk::glib;
use libadwaita as adw;

const APP_ID: &str = "org.gentoo_diy.Installer";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    if std::env::args().any(|a| a == "--debug-icons") {
        app.connect_activate(window::build_debug_icons);
    } else if std::env::args().any(|a| a == "--debug-progress") {
        app.connect_activate(window::build_debug_progress);
    } else {
        app.connect_activate(window::build);
    }
    // Our own flags must not reach GApplication, which rejects unknown options.
    app.run_with_args::<&str>(&[])
}
