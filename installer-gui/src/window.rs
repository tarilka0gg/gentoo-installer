//! Main window: an Adwaita ToolbarView wrapping a NavigationView, one page per wizard step:
//! Welcome -> disk-select -> confirm -> installing. Network step for the GUI ISO (main niri
//! image) still needs its iwd-backed page.
//!
//! Visual patterns (card-style disk picker, bar-with-separate-legend partitioning preview,
//! progress bar + collapsible log) are ported from elementary's GTK installer
//! (github.com/elementary/installer — `Widgets/DiskGrid.vala`, `Widgets/DiskBar.vala`,
//! `Views/ProgressView.vala`), adapted to libadwaita idioms — this is the standard
//! libadwaita look (HeaderBar, NavigationView, boxed cards), matching this system's own
//! apps (e.g. `../portage-store`), not the stripped-down non-adwaita design from the
//! formal build spec's §9 — that direction didn't match what was actually wanted here.

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
    install_css();

    let nav = adw::NavigationView::new();
    let selected_disk: Rc<RefCell<Option<disk::Disk>>> = Rc::new(RefCell::new(None));
    let profile: Rc<RefCell<Option<hardware::Profile>>> = Rc::new(RefCell::new(None));
    let existing_os: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Back"));
    back_button.set_visible(false);
    {
        let nav = nav.clone();
        back_button.connect_clicked(move |_| {
            nav.pop();
        });
    }

    let header = adw::HeaderBar::new();
    header.pack_start(&back_button);

    let disk_page = disk_select_page(nav.clone(), selected_disk.clone(), profile.clone(), existing_os.clone());
    nav.add(&welcome_page(&nav, &disk_page));
    nav.add(&disk_page);

    // Screen script §0: back arrow hidden on Welcome and from Confirm onward (destructive
    // point of no return — Confirm's own footer `Back` button is the only way backward
    // from there). Driven off the navigation stack itself rather than toggled at each
    // transition site, so it also does the right thing when the user pops back manually.
    {
        let back_button = back_button.clone();
        nav.connect_visible_page_notify(move |nav| {
            let hidden = matches!(
                nav.visible_page().and_then(|p| p.tag()).as_deref(),
                Some("welcome") | Some("confirm") | Some("installing") | Some("done") | None
            );
            back_button.set_visible(!hidden);
        });
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
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

/// One shared stylesheet for the whole app — disk-card selection state and the
/// partition-bar/legend swatch colors, which libadwaita has no ready-made classes for.
fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        "\
        .gentoo-disk-card:checked { \
            border: 2px solid @accent_bg_color; \
            background-color: alpha(@accent_bg_color, 0.08); \
        }\n\
        .gentoo-disk-card { border: 2px solid transparent; border-radius: 12px; }\n\
        .gentoo-part-esp { background-color: #3584e4; border-radius: 6px; }\n\
        .gentoo-part-swap { background-color: #e5a50a; border-radius: 6px; }\n\
        .gentoo-part-root { background-color: #26a269; border-radius: 6px; }\n\
        ",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
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

/// Card-style disk picker (ported from elementary installer's `DiskGrid`/`DiskButton`):
/// a radio-grouped `ToggleButton` per disk with an icon, bold name, and path/size
/// subtitle, instead of a plain row list — the selected card gets an accent border via
/// the `:checked` CSS above.
fn disk_card(disk: &disk::Disk, group_with: Option<&gtk::ToggleButton>) -> gtk::ToggleButton {
    let icon = gtk::Image::builder().icon_name("drive-harddisk-symbolic").pixel_size(32).build();

    let name_label = gtk::Label::builder()
        .label(&disk.path)
        .halign(gtk::Align::Start)
        .css_classes(vec!["heading".to_string()])
        .build();
    let subtitle_label = gtk::Label::builder()
        .label(format!("{}  {}", disk::format_size(disk.size_bytes), disk.model))
        .halign(gtk::Align::Start)
        .css_classes(vec!["dim-label".to_string()])
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();

    let text_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
    text_box.append(&name_label);
    text_box.append(&subtitle_label);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(10)
        .margin_bottom(10)
        .build();
    content.append(&icon);
    content.append(&text_box);

    let button = gtk::ToggleButton::builder()
        .child(&content)
        .css_classes(vec!["card".to_string(), "gentoo-disk-card".to_string()])
        .build();
    if let Some(g) = group_with {
        button.set_group(Some(g));
    }
    button
}

/// A disk needs room for ESP + swap + a minimally usable root — same floor
/// `phase::preflight` computes — below that it's not a real install target.
fn min_disk_bytes() -> u64 {
    use installer_core::partition::{SWAP_MIN_GIB, ESP_SIZE_MIB};
    (ESP_SIZE_MIB + SWAP_MIN_GIB * 1024 + 8 * 1024) * 1024 * 1024
}

fn disk_select_page(
    nav: adw::NavigationView,
    selected_disk: Rc<RefCell<Option<disk::Disk>>>,
    profile: Rc<RefCell<Option<hardware::Profile>>>,
    existing_os: Rc<RefCell<Option<String>>>,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Where should Gentoo go?")
        .css_classes(vec!["title-1".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    let body = gtk::Label::builder()
        .label("Everything on the disk you choose will be erased.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let cards_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(12).build();

    let profile_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .margin_bottom(4)
        .wrap(true)
        .css_classes(vec!["dim-label".to_string()])
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

    // disk::list + detect::gather are async in installer-core; spin up a throwaway
    // runtime for this synchronous GTK callback context rather than threading a shared
    // one through yet.
    let (disks, existing_os_found) = std::thread::spawn(|| {
        tokio::runtime::Runtime::new().expect("tokio runtime").block_on(async {
            let runner = installer_core::command::RealCommandRunner;
            let disks = disk::list(&runner).await.unwrap_or_default();
            let detected = installer_core::detect::gather(&runner).await;
            (disks, detected.existing_os)
        })
    })
    .join()
    .unwrap_or_default();

    *existing_os.borrow_mut() = existing_os_found.clone();
    if let Some(os) = &existing_os_found {
        let note = gtk::Label::builder()
            .label(format!("{os} was found on this machine."))
            .css_classes(vec!["dim-label".to_string()])
            .halign(gtk::Align::Start)
            .wrap(true)
            .build();
        cards_box.append(&note);
    }

    let min_bytes = min_disk_bytes();
    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .sensitive(false)
        .build();

    if disks.is_empty() {
        cards_box.append(&adw::ActionRow::builder().title("No disks found").build());
    } else {
        let mut first_button: Option<gtk::ToggleButton> = None;
        for d in &disks {
            let eligible = d.size_bytes >= min_bytes;
            let button = disk_card(d, first_button.as_ref());
            button.set_sensitive(eligible);
            if !eligible {
                button.set_active(false);
            }

            if eligible {
                if first_button.is_none() {
                    button.set_active(true);
                    *selected_disk.borrow_mut() = Some(d.clone());
                    next_button.set_sensitive(true);
                    first_button = Some(button.clone());
                }

                let selected_disk = selected_disk.clone();
                let d = d.clone();
                button.connect_toggled(move |b| {
                    if b.is_active() {
                        *selected_disk.borrow_mut() = Some(d.clone());
                    }
                });
            }

            cards_box.append(&button);
            if !eligible {
                let reason = gtk::Label::builder()
                    .label(format!("Too small — needs at least {}", disk::format_size(min_bytes)))
                    .css_classes(vec!["dim-label".to_string(), "caption".to_string()])
                    .halign(gtk::Align::Start)
                    .margin_start(12)
                    .build();
                cards_box.append(&reason);
            }
        }
    }

    let confirm_page = Rc::new(RefCell::new(None::<adw::NavigationPage>));
    {
        let nav = nav.clone();
        let selected_disk = selected_disk.clone();
        let profile = profile.clone();
        let existing_os = existing_os.clone();
        let confirm_page = confirm_page.clone();
        next_button.connect_clicked(move |_| {
            if confirm_page.borrow().is_none() {
                let page = confirm_page_build(nav.clone(), selected_disk.clone(), profile.clone(), existing_os.clone());
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
    content.append(&heading);
    content.append(&body);
    content.append(&profile_label);
    content.append(&cards_box);
    content.append(&next_button);

    let clamp = adw::Clamp::builder().child(&content).maximum_size(700).build();

    let page = adw::NavigationPage::builder()
        .title("Select disk")
        .child(&clamp)
        .build();
    page.set_tag(Some("disk-select"));
    page
}

/// Screen script §10: "styled deliberately plainer and denser than every other page —
/// no tiles, no large heading, tighter leading. Reading it is the point." Plain sentence
/// lines instead of a StatusPage; only lines backed by something real are shown (no
/// account/timezone/encryption lines yet — those pages don't exist).
fn confirm_page_build(
    nav: adw::NavigationView,
    selected_disk: Rc<RefCell<Option<disk::Disk>>>,
    profile: Rc<RefCell<Option<hardware::Profile>>>,
    existing_os: Rc<RefCell<Option<String>>>,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Review before installing")
        .css_classes(vec!["title-1".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let simulate = simulate_mode();
    let disk = selected_disk.borrow().clone();
    let sentences = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(14).margin_top(20).build();

    let sentence = |text: &str, destructive: bool| {
        let label = gtk::Label::builder().label(text).halign(gtk::Align::Start).wrap(true).build();
        if destructive {
            label.add_css_class("error");
        }
        label
    };

    if simulate {
        if let Some(d) = &disk {
            sentences.append(&sentence(
                &format!("SIMULATION MODE — no real changes will be made to {} ({}).", d.path, disk::format_size(d.size_bytes)),
                false,
            ));
        }
    } else if let Some(d) = &disk {
        sentences.append(&sentence(&format!("Install Gentoo on {} ({}).", d.path, disk::format_size(d.size_bytes)), false));

        let erase_line = match existing_os.borrow().as_ref() {
            Some(os) => format!("Erase everything on this disk, including {os}."),
            None => "Erase everything on this disk.".to_string(),
        };
        sentences.append(&sentence(&erase_line, true));
    }

    sentences.append(&sentence("This takes about 15\u{2013}20 minutes.", false));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&sentences);

    let ram_bytes = profile.borrow().as_ref().map(|p| p.ram_bytes).unwrap_or(0);
    if let Some(d) = &disk {
        content.append(&partition_bar(d.size_bytes, ram_bytes));
    }

    let error_label = gtk::Label::builder().halign(gtk::Align::Start).wrap(true).visible(false).css_classes(vec!["error".to_string()]).build();
    content.append(&error_label);

    let back_secondary = gtk::Button::builder().label("Back").css_classes(vec!["flat".to_string()]).build();
    let install_button = gtk::Button::builder()
        .label(if simulate { "Simulate install" } else { "Erase and install" })
        .css_classes(vec![
            (if simulate { "suggested-action" } else { "destructive-action" }).to_string(),
            "pill".to_string(),
        ])
        .build();
    let footer = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).halign(gtk::Align::End).margin_top(24).build();
    footer.append(&back_secondary);
    footer.append(&install_button);
    content.append(&footer);

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Confirm").child(&clamp).build();
    page.set_tag(Some("confirm"));

    {
        let nav = nav.clone();
        back_secondary.connect_clicked(move |_| {
            nav.pop();
        });
    }

    let installing: Rc<RefCell<Option<InstallingWidgets>>> = Rc::new(RefCell::new(None));
    {
        let nav = nav.clone();
        let selected_disk = selected_disk.clone();
        let profile = profile.clone();
        let installing = installing.clone();
        let error_label = error_label.clone();
        install_button.connect_clicked(move |btn| {
            let simulate = simulate_mode();
            let store_env = if simulate {
                None
            } else {
                match StoreEnv::from_env() {
                    Ok(env) => Some(env),
                    Err(e) => {
                        error_label.set_label(&format!("store not configured: {e}"));
                        error_label.set_visible(true);
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

            if installing.borrow().is_none() {
                let widgets = installing_page_build(nav.clone());
                nav.add(&widgets.page);
                *installing.borrow_mut() = Some(widgets);
            }
            let widgets = installing.borrow().as_ref().unwrap().clone();
            nav.push(&widgets.page);
            spawn_install(opts, widgets);
        });
    }

    page
}

/// GParted/elementary-style layout preview: ESP/swap/root as proportioned colored
/// segments, with size/label text moved into a legend list below the bar rather than
/// crammed inside the (often very narrow) segments themselves — real disks make ESP
/// (512MiB) and swap tiny relative to root, so text has to live outside the bar to stay
/// legible. Each segment still gets a minimum pixel width so it stays visible at all.
fn partition_bar(disk_size_bytes: u64, ram_bytes: u64) -> gtk::Box {
    use installer_core::partition::{swap_size_gib, ESP_SIZE_MIB};

    let total_mib = (disk_size_bytes / 1024 / 1024).max(1);
    let esp_mib = ESP_SIZE_MIB;
    let swap_mib = swap_size_gib(ram_bytes) * 1024;
    let root_mib = total_mib.saturating_sub(esp_mib + swap_mib);

    const BAR_WIDTH: i32 = 480;
    const MIN_SEG: i32 = 48;
    let esp_px = MIN_SEG;
    let swap_frac = swap_mib as f64 / total_mib as f64;
    let swap_px = ((BAR_WIDTH as f64 * swap_frac) as i32).clamp(MIN_SEG, 160);
    let root_px = (BAR_WIDTH - esp_px - swap_px).max(MIN_SEG);

    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(3)
        .halign(gtk::Align::Center)
        .build();
    let segment = |css_class: &str, width: i32| -> gtk::Box {
        gtk::Box::builder()
            .width_request(width)
            .height_request(28)
            .css_classes(vec![css_class.to_string()])
            .build()
    };
    bar.append(&segment("gentoo-part-esp", esp_px));
    bar.append(&segment("gentoo-part-swap", swap_px));
    bar.append(&segment("gentoo-part-root", root_px));

    let legend = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).halign(gtk::Align::Center).build();
    let legend_row = |css_class: &str, title: &str, size_bytes: u64, fs: &str| -> gtk::Box {
        let swatch = gtk::Box::builder()
            .width_request(14)
            .height_request(14)
            .valign(gtk::Align::Center)
            .css_classes(vec![css_class.to_string()])
            .build();
        let label = gtk::Label::builder()
            .label(format!("{title} — {} ({fs})", disk::format_size(size_bytes)))
            .halign(gtk::Align::Start)
            .build();
        let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
        row.append(&swatch);
        row.append(&label);
        row
    };
    legend.append(&legend_row("gentoo-part-esp", "ESP", esp_mib * 1024 * 1024, "vfat"));
    legend.append(&legend_row("gentoo-part-swap", "swap", swap_mib * 1024 * 1024, "swap"));
    legend.append(&legend_row("gentoo-part-root", "root", root_mib * 1024 * 1024, "btrfs"));

    let wrapper = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
    wrapper.append(&bar);
    wrapper.append(&legend);
    wrapper
}

#[derive(Clone)]
struct InstallingWidgets {
    nav: adw::NavigationView,
    page: adw::NavigationPage,
    progress: gtk::ProgressBar,
    status_label: gtk::Label,
    log_label: gtk::Label,
}

/// Ported from elementary installer's `ProgressView`: a determinate progress bar with a
/// short status line by default, and the full step-by-step log tucked behind a
/// collapsed `Expander` — most users never need to open it, but it's there for anyone
/// debugging a failed install.
fn installing_page_build(nav: adw::NavigationView) -> InstallingWidgets {
    let status_label = gtk::Label::builder()
        .label("Getting ready")
        .css_classes(vec!["title-2".to_string()])
        .wrap(true)
        .build();

    let progress = gtk::ProgressBar::builder().hexpand(true).show_text(false).build();

    let log_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .wrap(true)
        .selectable(true)
        .build();
    let log_scroller = gtk::ScrolledWindow::builder()
        .child(&log_label)
        .min_content_height(160)
        .build();
    let details = gtk::Expander::builder().label("Show details").child(&log_scroller).build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&status_label);
    content.append(&progress);
    content.append(&details);

    let clamp = adw::Clamp::builder().child(&content).maximum_size(600).build();

    let page = adw::NavigationPage::builder().title("Installing").child(&clamp).build();
    page.set_tag(Some("installing"));

    InstallingWidgets { nav, page, progress, status_label, log_label }
}

/// Bridges `install::run`'s tokio-channel progress into the GTK main loop: the install
/// itself runs on a background thread with its own tokio runtime (GTK's own loop isn't
/// async), and progress is relayed through a `std::sync::mpsc` channel that a
/// `glib::timeout_add_local` on the main thread drains every 200ms — the same
/// non-blocking-poll approach the CLI uses per ratatui frame.
fn spawn_install(opts: install::InstallOptions, widgets: InstallingWidgets) {
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

    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let mut log = widgets.log_label.label().to_string();
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Progress(p) => {
                    widgets.progress.set_fraction(progress_fraction(&p));
                    let line = describe(&p);
                    widgets.status_label.set_label(&line);
                    if !log.is_empty() {
                        log.push('\n');
                    }
                    log.push_str(&line);
                }
                Event::Finished(Ok(())) => {
                    widgets.progress.set_fraction(1.0);
                    let done_page = done_page_build();
                    widgets.nav.add(&done_page);
                    widgets.nav.push(&done_page);
                }
                Event::Finished(Err(e)) => {
                    widgets.status_label.set_label(&format!("Install failed: {e}"));
                    if !log.is_empty() {
                        log.push('\n');
                    }
                    log.push_str(&format!("ERROR: {e}"));
                }
            }
        }
        widgets.log_label.set_label(&log);
        glib::ControlFlow::Continue
    });
}

/// Screen script §12. `Restart` is a real `systemctl reboot` outside simulate mode —
/// guarded so clicking it during UI iteration never reboots the development machine.
fn done_page_build() -> adw::NavigationPage {
    let status = adw::StatusPage::builder()
        .title("Gentoo is ready")
        .description("Restart to start using it.")
        .icon_name("emblem-ok-symbolic")
        .build();

    let note = gtk::Label::builder()
        .label(
            "Your Portage configuration is version-controlled from this moment. \
             Open Portage Store after restarting to see what the installer set, and to change it.",
        )
        .wrap(true)
        .halign(gtk::Align::Center)
        .justify(gtk::Justification::Center)
        .css_classes(vec!["dim-label".to_string()])
        .build();

    let restart_button = gtk::Button::builder()
        .label("Restart")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .build();
    restart_button.connect_clicked(|btn| {
        if simulate_mode() {
            btn.set_label("Simulated — would restart now");
            btn.set_sensitive(false);
            return;
        }
        let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
    });

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).halign(gtk::Align::Center).build();
    content.append(&note);
    content.append(&restart_button);
    status.set_child(Some(&content));

    let page = adw::NavigationPage::builder().title("Done").child(&status).build();
    page.set_tag(Some("done"));
    page
}

const TOTAL_STEPS: f64 = 7.0;

fn progress_fraction(p: &install::Progress) -> f64 {
    let step = match p {
        install::Progress::Partitioning => 0.0,
        install::Progress::DownloadingStage3 => 1.0,
        install::Progress::UnpackingStage3 => 2.0,
        install::Progress::ConfiguringStore => 3.0,
        install::Progress::InstallingKernel { .. } => 4.0,
        install::Progress::WritingFstab => 5.0,
        install::Progress::InstallingBootloader => 6.0,
        install::Progress::Done => 7.0,
    };
    step / TOTAL_STEPS
}

/// Screen script §11 phase labels, applied to the phases this codebase actually
/// performs — the doc's own list (Setting your language and time, Creating your
/// account, Building the startup image, ...) includes steps nothing here does yet, so
/// those aren't claimed. Where our mechanism differs from the doc's assumed squashfs
/// image copy (we download+unpack an official stage3 instead), the label says what's
/// actually happening rather than borrowing the doc's phrase for a different mechanism.
fn describe(p: &install::Progress) -> String {
    match p {
        install::Progress::Partitioning => "Preparing the disk".into(),
        install::Progress::DownloadingStage3 => "Downloading the base system".into(),
        install::Progress::UnpackingStage3 => "Setting up the base system".into(),
        install::Progress::ConfiguringStore => "Setting up package sources".into(),
        install::Progress::InstallingKernel { atom, degraded_by } => {
            if *degraded_by == 0 {
                format!("Installing your kernel ({atom}, exact hardware match)")
            } else {
                format!("Installing your kernel ({atom}, closest available match)")
            }
        }
        install::Progress::WritingFstab => "Setting up the file system".into(),
        install::Progress::InstallingBootloader => "Installing the bootloader".into(),
        install::Progress::Done => "Finishing up".into(),
    }
}
