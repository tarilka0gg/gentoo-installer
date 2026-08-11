//! Main window shell: an Adwaita ToolbarView wrapping a NavigationView, one page per
//! wizard step. Pages get their own modules as each installer-core step gets wired up.

use adw::prelude::*;
use libadwaita as adw;

pub fn build(app: &adw::Application) {
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());

    let status = adw::StatusPage::builder()
        .title("Gentoo Installer")
        .description("Automated btrfs partitioning, kernel profile detection, Limine boot setup.")
        .icon_name("drive-harddisk-symbolic")
        .build();
    toolbar_view.set_content(Some(&status));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(900)
        .default_height(640)
        .content(&toolbar_view)
        .title("Gentoo Installer")
        .build();

    window.present();
}
