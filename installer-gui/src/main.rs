//! GTK4 + libadwaita wizard for the main ISO (niri + shell package preinstalled on target).
//! Steps mirror the CLI's, plus network config done inline in-app instead of TUI.

mod window;

use adw::prelude::*;
use gtk::glib;
use libadwaita as adw;

const APP_ID: &str = "org.gentoo_diy.Installer";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(window::build);
    app.run()
}
