//! Main window: an Adwaita ToolbarView wrapping a NavigationView, one page per wizard step.
//! Disk list and hardware profile are populated from installer-core on the disk-select page;
//! network step for the GUI ISO (main niri image) still needs its iwd-backed page.

use adw::prelude::*;
use installer_core::{disk, hardware};
use libadwaita as adw;

pub fn build(app: &adw::Application) {
    let nav = adw::NavigationView::new();
    let disk_page = disk_select_page();
    nav.add(&welcome_page(&nav, &disk_page));
    nav.add(&disk_page);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());
    toolbar_view.set_content(Some(&nav));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(900)
        .default_height(640)
        .content(&toolbar_view)
        .title("Gentoo Installer")
        .build();

    window.present();
}

fn welcome_page(nav: &adw::NavigationView, disk_page: &adw::NavigationPage) -> adw::NavigationPage {
    let status = adw::StatusPage::builder()
        .title("Gentoo Installer")
        .description("Automated btrfs partitioning, kernel profile detection, Limine boot setup.")
        .icon_name("drive-harddisk-symbolic")
        .build();

    let button = gtk::Button::builder()
        .label("Start")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .build();
    status.set_child(Some(&button));

    let page = adw::NavigationPage::builder()
        .title("Welcome")
        .child(&status)
        .build();
    page.set_tag(Some("welcome"));

    let nav = nav.clone();
    let disk_page = disk_page.clone();
    button.connect_clicked(move |_| nav.push(&disk_page));

    page
}

fn disk_select_page() -> adw::NavigationPage {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();

    let profile_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .wrap(true)
        .build();

    match hardware::Profile::detect() {
        Ok(p) => profile_label.set_label(&format!(
            "CPU: {:?}   GPU: {:?}   RAM: {} GiB   Laptop: {}   Kernel profile: {}",
            p.cpu,
            p.gpu,
            p.ram_bytes / 1024 / 1024 / 1024,
            p.is_laptop,
            p.kernel_suffix()
        )),
        Err(e) => profile_label.set_label(&format!("hardware detection failed: {e}")),
    }

    // lsblk is async in installer-core; spin up a throwaway runtime for this synchronous
    // GTK callback context rather than threading a shared one through yet.
    let disks = std::thread::spawn(|| {
        tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(disk::list())
    })
    .join()
    .unwrap_or_else(|_| Ok(Vec::new()));

    match disks {
        Ok(disks) if !disks.is_empty() => {
            for d in disks {
                let row = adw::ActionRow::builder()
                    .title(d.path.clone())
                    .subtitle(format!("{}  {}", disk::format_size(d.size_bytes), d.model))
                    .build();
                list.append(&row);
            }
        }
        Ok(_) => {
            list.append(&adw::ActionRow::builder().title("No disks found").build());
        }
        Err(e) => {
            list.append(
                &adw::ActionRow::builder()
                    .title("Disk listing failed")
                    .subtitle(e.to_string())
                    .build(),
            );
        }
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(24)
        .margin_bottom(24)
        .build();
    content.append(&profile_label);
    content.append(&list);

    let clamp = adw::Clamp::builder().child(&content).maximum_size(700).build();

    let page = adw::NavigationPage::builder()
        .title("Select disk")
        .child(&clamp)
        .build();
    page.set_tag(Some("disk-select"));
    page
}
