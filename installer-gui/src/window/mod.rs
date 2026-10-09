//! Main window: an Adwaita ToolbarView wrapping a NavigationView, one page per wizard step:
//! Welcome -> [Keyboard -> Timezone -> Desktop -> Optimization, if Advanced] -> [Network,
//! if no ethernet link] -> Disk -> Account -> Confirm -> Installing -> Done.
//!
//! Visual patterns (card-style disk picker, bar-with-separate-legend partitioning preview,
//! progress bar + collapsible log) are ported from elementary's GTK installer
//! (github.com/elementary/installer — `Widgets/DiskGrid.vala`, `Widgets/DiskBar.vala`,
//! `Views/ProgressView.vala`), adapted to libadwaita idioms — this is the standard
//! libadwaita look (HeaderBar, NavigationView, boxed cards), matching this system's own
//! apps (e.g. `../portage-store`), not the stripped-down non-adwaita design from the
//! formal build spec's §9 — that direction didn't match what was actually wanted here.
//!
//! The screen script's "⋯" menu (§0) is one `gtk::MenuButton` in the shared header whose
//! popover content is swapped per page: Welcome gets the "Advanced setup" switch, Disk
//! gets "Manual partitioning…" (enabled only once Advanced is on). Other pages hide the
//! button entirely rather than showing an empty menu.

use adw::prelude::*;
use gtk::glib;
use installer_core::{
    account::Account,
    config::StoreEnv,
    disk, hardware, install, keyboard,
    make_conf::{OptLevel, PackageMode},
    network, partition, store, timezone as tz,
    wm::WmChoice,
};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;

mod account;
mod confirm;
mod desktop;
mod locale;
mod progress;
mod storage;
mod welcome;
mod wifi;
use account::*;
use confirm::*;
use desktop::*;
use locale::*;
pub use progress::build_debug_progress;
use progress::*;
use storage::*;
use welcome::*;
use wifi::*;

/// Per-page "⋯" popover content, keyed by `NavigationPage` tag. Built once, up front,
/// by whichever page function owns the context (`nav`, `state`) it needs to wire real
/// clicks — not rebuilt per visit, and never reached-into from outside via widget-name
/// search (a `GtkPopover` lives on its own surface; walking the main window's widget
/// tree to find something inside one doesn't work).
type PageMenus = Rc<RefCell<HashMap<String, gtk::Popover>>>;

/// Set to click through the wizard without touching a real disk — see
/// `installer_core::install::InstallOptions::simulate`.
fn simulate_mode() -> bool {
    std::env::var("GENTOO_INSTALLER_SIMULATE").is_ok()
}

/// Everything pages need to hand off to each other. One shared struct instead of a
/// growing pile of individually-threaded `Rc<RefCell<T>>` args — this used to be three
/// separate ones before Keyboard/Timezone/Account/manual-partitioning added five more.
struct WizardState {
    selected_disk: RefCell<Option<disk::Disk>>,
    profile: RefCell<Option<hardware::Profile>>,
    existing_os: RefCell<Option<String>>,
    advanced: Cell<bool>,
    keyboard_layout: RefCell<String>,
    timezone: RefCell<String>,
    wm: Cell<WmChoice>,
    gpu: Cell<Option<hardware::Gpu>>,
    packages: RefCell<Vec<String>>,
    opt_level: Cell<OptLevel>,
    package_mode: Cell<PackageMode>,
    secure_boot: Cell<bool>,
    manual_root_fs: Cell<partition::RootFs>,
    /// `None` means "use the automatic RAM-based size" — set only if the user actually
    /// changes it on the manual-partitioning page.
    manual_swap_gib: RefCell<Option<u64>>,
    username: RefCell<String>,
    password: RefCell<String>,
}

impl WizardState {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            selected_disk: RefCell::new(None),
            profile: RefCell::new(None),
            existing_os: RefCell::new(None),
            advanced: Cell::new(false),
            keyboard_layout: RefCell::new(keyboard::detect_current()),
            timezone: RefCell::new(
                installer_core::autodetect::guess_here()
                    .timezone
                    .unwrap_or_else(|| "UTC".to_string()),
            ),
            wm: Cell::new(WmChoice::default()),
            gpu: Cell::new(None),
            packages: RefCell::new(installer_core::packages::default_ids()),
            opt_level: Cell::new(OptLevel::default()),
            package_mode: Cell::new(PackageMode::default()),
            secure_boot: Cell::new(false),
            manual_root_fs: Cell::new(partition::RootFs::Btrfs),
            manual_swap_gib: RefCell::new(None),
            username: RefCell::new(String::new()),
            password: RefCell::new(String::new()),
        })
    }

    fn swap_gib(&self) -> u64 {
        let ram_bytes = self
            .profile
            .borrow()
            .as_ref()
            .map(|p| p.ram_bytes)
            .unwrap_or(0);
        self.manual_swap_gib
            .borrow()
            .unwrap_or_else(|| partition::swap_size_gib(ram_bytes))
    }
}

pub fn build(app: &adw::Application) {
    install_css();

    let nav = adw::NavigationView::new();
    let state = WizardState::new();

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Back"));
    back_button.set_visible(false);
    {
        let nav = nav.clone();
        back_button.connect_clicked(move |_| {
            nav.pop();
        });
    }

    let menu_button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .build();
    let page_menus: PageMenus = Rc::new(RefCell::new(HashMap::new()));

    let header = adw::HeaderBar::new();
    header.pack_start(&back_button);
    header.pack_end(&menu_button);

    page_menus
        .borrow_mut()
        .insert("welcome".to_string(), advanced_setup_popover(state.clone()));

    let disk_page = disk_select_page(nav.clone(), state.clone(), page_menus.clone());
    let network_page = network_page_build(nav.clone(), disk_page.clone());
    // Screen script §4: skipped silently if already connected — checked once at
    // startup, not re-checked live, since link state genuinely won't change mid-wizard
    // on installer hardware.
    let target_after_prelude = if network::IwdClient::ethernet_link_up() {
        disk_page.clone()
    } else {
        network_page.clone()
    };
    let opt_level_page =
        opt_level_select_page(nav.clone(), state.clone(), target_after_prelude.clone());
    let packages_page = packages_select_page(
        nav.clone(),
        state.clone(),
        opt_level_page.clone(),
        target_after_prelude.clone(),
    );
    let gpu_page = gpu_select_page(nav.clone(), state.clone(), packages_page.clone());
    let wm_page = wm_select_page(nav.clone(), state.clone(), gpu_page.clone());
    let timezone_page = timezone_select_page(nav.clone(), state.clone(), wm_page.clone());
    let keyboard_page = keyboard_select_page(nav.clone(), state.clone(), timezone_page.clone());
    nav.add(&welcome_page(
        &nav,
        state.clone(),
        keyboard_page.clone(),
        packages_page.clone(),
    ));
    nav.add(&keyboard_page);
    nav.add(&timezone_page);
    nav.add(&wm_page);
    nav.add(&gpu_page);
    nav.add(&packages_page);
    nav.add(&opt_level_page);
    nav.add(&network_page);
    nav.add(&disk_page);

    // Screen script §0: back arrow hidden on Welcome and from Confirm onward (destructive
    // point of no return — Confirm's own footer `Back` button is the only way backward
    // from there). Driven off the navigation stack itself rather than toggled at each
    // transition site, so it also does the right thing when the user pops back manually.
    // The "⋯" menu follows the same visible-page signal: content swapped per page (looked
    // up from `page_menus`, built once above), hidden entirely on pages with nothing
    // real to put in it.
    {
        let back_button = back_button.clone();
        let menu_button = menu_button.clone();
        let page_menus = page_menus.clone();
        nav.connect_visible_page_notify(move |nav| {
            let tag = nav
                .visible_page()
                .and_then(|p| p.tag())
                .map(|t| t.to_string());
            let hidden = matches!(
                tag.as_deref(),
                Some("welcome") | Some("confirm") | Some("installing") | Some("done") | None
            );
            back_button.set_visible(!hidden);
            match tag
                .as_deref()
                .and_then(|t| page_menus.borrow().get(t).cloned())
            {
                Some(popover) => {
                    menu_button.set_visible(true);
                    menu_button.set_popover(Some(&popover));
                }
                None => menu_button.set_visible(false),
            }
        });
    }
    if let Some(popover) = page_menus.borrow().get("welcome") {
        menu_button.set_visible(true);
        menu_button.set_popover(Some(popover));
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&nav));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(800)
        .default_height(640)
        .content(&toolbar_view)
        .title("Gentoo Installer")
        .build();

    window.present();
}

/// One shared stylesheet for the whole app — disk-card selection state and the
/// partition-bar/legend swatch colors, which libadwaita has no ready-made classes for.
/// Round icon badge. The image must expand inside the fixed-size circle: a horizontal
/// `GtkBox` hands a non-expanding child only its natural width at the start edge, so
/// `halign(Center)` alone leaves the glyph pinned to the left of the backing.
fn icon_badge(icon: &str, pixel_size: i32) -> gtk::Box {
    let image = gtk::Image::builder()
        .icon_name(icon)
        .pixel_size(pixel_size)
        .hexpand(true)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let badge = gtk::Box::builder()
        .css_classes(vec!["info-tile-icon".to_string()])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .build();
    badge.append(&image);
    badge
}

/// `--debug-icons`: a do-nothing window that lays the app's icon badges out over centre
/// guide lines, to eyeball whether glyphs sit in the middle of their backing.
pub fn build_debug_icons(app: &adw::Application) {
    install_css();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(24)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    for icon in [
        "drive-harddisk-symbolic",
        "computer-symbolic",
        "system-run-symbolic",
        "emblem-ok-symbolic",
        "network-wireless-symbolic",
        "go-previous-symbolic",
    ] {
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&icon_badge(icon, 22)));
        for (class, halign, valign) in [
            ("debug-guide-v", gtk::Align::Center, gtk::Align::Fill),
            ("debug-guide-h", gtk::Align::Fill, gtk::Align::Center),
        ] {
            let guide = gtk::Box::builder()
                .css_classes(vec![class.to_string()])
                .halign(halign)
                .valign(valign)
                .can_target(false)
                .build();
            overlay.add_overlay(&guide);
        }
        let label = gtk::Label::builder()
            .label(icon)
            .css_classes(vec!["caption".to_string(), "dim-label".to_string()])
            .build();
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        cell.append(&overlay);
        cell.append(&label);
        row.append(&cell);
    }
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Icon alignment debug")
        .default_width(900)
        .default_height(200)
        .content(&row)
        .build();
    window.present();
}

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
        .gentoo-part-home { background-color: #9141ac; border-radius: 6px; }\n\
        .hero-title { font-size: 1.8rem; font-weight: 800; }\n\
        .section-heading { font-size: 1.05rem; font-weight: 800; }\n\
        .info-tile { padding: 16px 10px; border-radius: 12px; }\n\
        .info-tile-icon { border-radius: 999px; min-width: 44px; min-height: 44px; \
            background-color: alpha(currentColor, 0.10); }\n\
        .debug-guide-v { min-width: 1px; background-color: rgba(255, 0, 0, 0.8); }\n\
        .debug-guide-h { min-height: 1px; background-color: rgba(255, 0, 0, 0.8); }\n\
        .category-tile { min-height: 88px; padding: 0 18px; border-radius: 16px; border: none; \
            color: #ffffff; font-size: 1.15rem; font-weight: 800; \
            text-shadow: 0 1px 2px rgba(0, 0, 0, 0.25); box-shadow: none; }\n\
        .category-tile:hover { filter: brightness(1.08); }\n\
        .category-tile:active { filter: brightness(0.94); }\n\
        .category-tile:checked { outline: 3px solid @accent_bg_color; outline-offset: 2px; }\n\
        .category-tile-0 { background-image: linear-gradient(160deg, #a06ee1, #4568dc); }\n\
        .category-tile-1 { background-image: linear-gradient(160deg, #fbf6d9, #f3ebc3); \
            color: #2b5ea8; text-shadow: none; }\n\
        .category-tile-2 { background-image: linear-gradient(120deg, #ffd3a5, #f78ca0 55%, #7b4dff); }\n\
        .category-tile-3 { background-image: linear-gradient(120deg, #f0416c, #ff7158); }\n\
        .category-tile-4 { background-image: linear-gradient(160deg, #2fbf71, #17a35c); }\n\
        .category-tile-5 { background-image: linear-gradient(160deg, #6b6b76, #4a4a55); }\n\
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
