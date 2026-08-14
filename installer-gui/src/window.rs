//! Main window: an Adwaita ToolbarView wrapping a NavigationView, one page per wizard step:
//! Welcome -> disk-select -> confirm -> installing. Network step for the GUI ISO (main niri
//! image) still needs its iwd-backed page.

use adw::prelude::*;
use gtk::glib;
use installer_core::{config::StoreEnv, disk, hardware, install, partition, store};
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;

/// Set to click through the wizard without touching a real disk — see
/// `installer_core::install::InstallOptions::simulate`.
fn simulate_mode() -> bool {
    std::env::var("GENTOO_INSTALLER_SIMULATE").is_ok()
}

pub fn build(app: &adw::Application) {
    let nav = adw::NavigationView::new();
    let selected_disk: Rc<RefCell<Option<disk::Disk>>> = Rc::new(RefCell::new(None));
    let profile: Rc<RefCell<Option<hardware::Profile>>> = Rc::new(RefCell::new(None));

    let disk_page = disk_select_page(nav.clone(), selected_disk.clone(), profile.clone());
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
    let description = if simulate_mode() {
        "SIMULATION MODE — GENTOO_INSTALLER_SIMULATE is set, no real changes will be made. \
         Automated btrfs partitioning, kernel profile detection, Limine boot setup."
    } else {
        "Automated btrfs partitioning, kernel profile detection, Limine boot setup."
    };
    let status = adw::StatusPage::builder()
        .title("Gentoo Installer")
        .description(description)
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

fn disk_select_page(
    nav: adw::NavigationView,
    selected_disk: Rc<RefCell<Option<disk::Disk>>>,
    profile: Rc<RefCell<Option<hardware::Profile>>>,
) -> adw::NavigationPage {
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
        Ok(p) => {
            profile_label.set_label(&format!(
                "RAM: {} GiB   Kernel profile: {}",
                p.ram_bytes / 1024 / 1024 / 1024,
                p.combo()
            ));
            *profile.borrow_mut() = Some(p);
        }
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

    let disks = disks.unwrap_or_default();
    let disks_rc = Rc::new(disks.clone());
    if disks.is_empty() {
        list.append(&adw::ActionRow::builder().title("No disks found").build());
    } else {
        for d in &disks {
            let row = adw::ActionRow::builder()
                .title(d.path.clone())
                .subtitle(format!("{}  {}", disk::format_size(d.size_bytes), d.model))
                .activatable(true)
                .build();
            list.append(&row);
        }
        list.select_row(list.row_at_index(0).as_ref());
    }

    let next_button = gtk::Button::builder()
        .label("Next")
        .css_classes(vec!["suggested-action".to_string()])
        .halign(gtk::Align::End)
        .sensitive(!disks.is_empty())
        .build();

    let confirm_page = Rc::new(RefCell::new(None::<adw::NavigationPage>));
    {
        let nav = nav.clone();
        let list = list.clone();
        let disks_rc = disks_rc.clone();
        let selected_disk = selected_disk.clone();
        let profile = profile.clone();
        let confirm_page = confirm_page.clone();
        next_button.connect_clicked(move |_| {
            let Some(row) = list.selected_row() else { return };
            let idx = row.index();
            if idx < 0 {
                return;
            }
            let Some(disk) = disks_rc.get(idx as usize) else { return };
            *selected_disk.borrow_mut() = Some(disk.clone());

            if confirm_page.borrow().is_none() {
                let page = confirm_page_build(nav.clone(), selected_disk.clone(), profile.clone());
                nav.add(&page);
                *confirm_page.borrow_mut() = Some(page);
            }
            if let Some(page) = confirm_page.borrow().as_ref() {
                nav.push(page);
            }
        });
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
    content.append(&next_button);

    let clamp = adw::Clamp::builder().child(&content).maximum_size(700).build();

    let page = adw::NavigationPage::builder()
        .title("Select disk")
        .child(&clamp)
        .build();
    page.set_tag(Some("disk-select"));
    page
}

fn confirm_page_build(
    nav: adw::NavigationView,
    selected_disk: Rc<RefCell<Option<disk::Disk>>>,
    profile: Rc<RefCell<Option<hardware::Profile>>>,
) -> adw::NavigationPage {
    let warning = adw::StatusPage::builder()
        .title("Confirm install")
        .icon_name("dialog-warning-symbolic")
        .build();

    let simulate = simulate_mode();
    let text = match selected_disk.borrow().as_ref() {
        Some(d) if simulate => format!(
            "SIMULATION MODE — no real changes will be made to {} ({}).",
            d.path,
            disk::format_size(d.size_bytes)
        ),
        Some(d) => format!(
            "About to WIPE {} ({}) and install: ESP 512MiB, swap, btrfs root with \
             @/@home/@var/@log subvolumes, Limine bootloader.",
            d.path,
            disk::format_size(d.size_bytes)
        ),
        None => "No disk selected.".to_string(),
    };
    warning.set_description(Some(&text));

    let install_button = gtk::Button::builder()
        .label(if simulate { "Simulate install" } else { "Install" })
        .css_classes(vec![
            (if simulate { "suggested-action" } else { "destructive-action" }).to_string(),
            "pill".to_string(),
        ])
        .halign(gtk::Align::Center)
        .build();
    warning.set_child(Some(&install_button));

    let page = adw::NavigationPage::builder()
        .title("Confirm")
        .child(&warning)
        .build();
    page.set_tag(Some("confirm"));

    let installing_page = Rc::new(RefCell::new(None::<adw::NavigationPage>));
    {
        let nav = nav.clone();
        let selected_disk = selected_disk.clone();
        let profile = profile.clone();
        let installing_page = installing_page.clone();
        let warning = warning.clone();
        install_button.connect_clicked(move |btn| {
            let simulate = simulate_mode();
            let store_env = if simulate {
                None
            } else {
                match StoreEnv::from_env() {
                    Ok(env) => Some(env),
                    Err(e) => {
                        warning.set_description(Some(&format!("store not configured: {e}")));
                        return;
                    }
                }
            };
            let Some(disk) = selected_disk.borrow().clone() else { return };
            let Some(prof) = profile.borrow().clone() else { return };
            btn.set_sensitive(false);

            let layout = partition::plan(&disk.path, partition::RootFs::Btrfs, prof.ram_bytes);
            let opts = install::InstallOptions {
                layout,
                target: "/mnt/gentoo".into(),
                store: store::StoreConfig {
                    binhost_url: store_env.as_ref().map(|e| e.binhost_url.clone()).unwrap_or_default(),
                    overlay_git_url: store_env.as_ref().map(|e| e.overlay_git_url.clone()).unwrap_or_default(),
                    overlay_name: store_env.as_ref().map(|e| e.overlay_name.clone()).unwrap_or_default(),
                },
                kernel_base_name: store_env
                    .map(|e| e.kernel_base_name)
                    .unwrap_or_else(|| "gentoo-diy-kernel".into()),
                simulate,
            };

            if installing_page.borrow().is_none() {
                let page = installing_page_build();
                nav.add(&page);
                *installing_page.borrow_mut() = Some(page);
            }
            let page = installing_page.borrow().as_ref().unwrap().clone();
            nav.push(&page);
            spawn_install(opts, page);
        });
    }

    page
}

fn installing_page_build() -> adw::NavigationPage {
    let log_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .wrap(true)
        .selectable(true)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    log_label.set_widget_name("install-log");

    let scroller = gtk::ScrolledWindow::builder().child(&log_label).vexpand(true).build();

    let page = adw::NavigationPage::builder().title("Installing").child(&scroller).build();
    page.set_tag(Some("installing"));
    page
}

/// Bridges `install::run`'s tokio-channel progress into the GTK main loop: the install
/// itself runs on a background thread with its own tokio runtime (GTK's own loop isn't
/// async), and progress is relayed through a `std::sync::mpsc` channel that a
/// `glib::timeout_add_local` on the main thread drains every 200ms — the same
/// non-blocking-poll approach the CLI uses per ratatui frame.
fn spawn_install(opts: install::InstallOptions, page: adw::NavigationPage) {
    #[derive(Debug)]
    enum Event {
        Progress(install::Progress),
        Finished(Result<(), String>),
    }

    let (tx, rx) = std_mpsc::channel::<Event>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
            let handle = tokio::spawn(install::run(opts, progress_tx));
            while let Some(p) = progress_rx.recv().await {
                if tx.send(Event::Progress(p)).is_err() {
                    return;
                }
            }
            let result = match handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(format!("install task panicked: {e}")),
            };
            let _ = tx.send(Event::Finished(result));
        });
    });

    let log_label = page
        .child()
        .and_downcast::<gtk::ScrolledWindow>()
        .and_then(|s| s.child())
        .and_downcast::<gtk::Label>()
        .expect("installing page always has a Label child");

    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let mut current = log_label.label().to_string();
        while let Ok(event) = rx.try_recv() {
            let line = match event {
                Event::Progress(p) => describe(&p),
                Event::Finished(Ok(())) => "Install complete.".to_string(),
                Event::Finished(Err(e)) => format!("ERROR: {e}"),
            };
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&line);
        }
        log_label.set_label(&current);
        glib::ControlFlow::Continue
    });
}

fn describe(p: &install::Progress) -> String {
    match p {
        install::Progress::Partitioning => "Partitioning disk...".into(),
        install::Progress::DownloadingStage3 => "Downloading stage3...".into(),
        install::Progress::UnpackingStage3 => "Unpacking stage3...".into(),
        install::Progress::ConfiguringStore => "Configuring portage store...".into(),
        install::Progress::InstallingKernel { atom, degraded_by } => {
            if *degraded_by == 0 {
                format!("Installing kernel: {atom} (exact hardware match)")
            } else {
                format!("Installing kernel: {atom} (generalized, degraded {degraded_by} step(s))")
            }
        }
        install::Progress::WritingFstab => "Writing fstab...".into(),
        install::Progress::InstallingBootloader => "Installing Limine...".into(),
        install::Progress::Done => "Install complete.".into(),
    }
}
