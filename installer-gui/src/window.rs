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
    account::Account, config::StoreEnv, disk, hardware, install, keyboard,
    make_conf::{OptLevel, PackageMode},
    network, partition, store, timezone as tz,
    wm::WmChoice,
};
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;

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
            timezone: RefCell::new(tz::detect_current().unwrap_or_else(|| "UTC".to_string())),
            wm: Cell::new(WmChoice::default()),
            gpu: Cell::new(None),
            packages: RefCell::new(installer_core::packages::default_ids()),
            opt_level: Cell::new(OptLevel::default()),
            package_mode: Cell::new(PackageMode::default()),
            manual_root_fs: Cell::new(partition::RootFs::Btrfs),
            manual_swap_gib: RefCell::new(None),
            username: RefCell::new(String::new()),
            password: RefCell::new(String::new()),
        })
    }

    fn swap_gib(&self) -> u64 {
        let ram_bytes = self.profile.borrow().as_ref().map(|p| p.ram_bytes).unwrap_or(0);
        self.manual_swap_gib.borrow().unwrap_or_else(|| partition::swap_size_gib(ram_bytes))
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

    let menu_button = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text("More").build();
    let page_menus: PageMenus = Rc::new(RefCell::new(HashMap::new()));

    let header = adw::HeaderBar::new();
    header.pack_start(&back_button);
    header.pack_end(&menu_button);

    page_menus.borrow_mut().insert("welcome".to_string(), advanced_setup_popover(state.clone()));

    let disk_page = disk_select_page(nav.clone(), state.clone(), page_menus.clone());
    let network_page = network_page_build(nav.clone(), disk_page.clone());
    // Screen script §4: skipped silently if already connected — checked once at
    // startup, not re-checked live, since link state genuinely won't change mid-wizard
    // on installer hardware.
    let target_after_prelude = if network::IwdClient::ethernet_link_up() { disk_page.clone() } else { network_page.clone() };
    let opt_level_page = opt_level_select_page(nav.clone(), state.clone(), target_after_prelude.clone());
    let packages_page = packages_select_page(nav.clone(), state.clone(), opt_level_page.clone());
    let gpu_page = gpu_select_page(nav.clone(), state.clone(), packages_page.clone());
    let wm_page = wm_select_page(nav.clone(), state.clone(), gpu_page.clone());
    let timezone_page = timezone_select_page(nav.clone(), state.clone(), wm_page.clone());
    let keyboard_page = keyboard_select_page(nav.clone(), state.clone(), timezone_page.clone());
    nav.add(&welcome_page(&nav, state.clone(), keyboard_page.clone(), target_after_prelude));
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
            let tag = nav.visible_page().and_then(|p| p.tag()).map(|t| t.to_string());
            let hidden = matches!(
                tag.as_deref(),
                Some("welcome") | Some("confirm") | Some("installing") | Some("done") | None
            );
            back_button.set_visible(!hidden);
            match tag.as_deref().and_then(|t| page_menus.borrow().get(t).cloned()) {
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
        .hero-title { font-size: 1.8rem; font-weight: 800; }\n\
        .section-heading { font-size: 1.05rem; font-weight: 800; }\n\
        .info-tile { padding: 16px 10px; border-radius: 12px; }\n\
        .info-tile-icon { border-radius: 999px; min-width: 44px; min-height: 44px; \
            background-color: alpha(currentColor, 0.10); }\n\
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

fn advanced_setup_popover(state: Rc<WizardState>) -> gtk::Popover {
    let label = gtk::Label::builder().label("Advanced setup").halign(gtk::Align::Start).hexpand(true).build();
    let switch = gtk::Switch::builder().active(state.advanced.get()).valign(gtk::Align::Center).build();
    switch.connect_state_set(move |_, active| {
        state.advanced.set(active);
        glib::Propagation::Proceed
    });

    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).margin_top(6).margin_bottom(6).margin_start(6).margin_end(6).build();
    row.append(&label);
    row.append(&switch);

    gtk::Popover::builder().child(&row).build()
}

/// `next_page` (Disk directly if there's already an ethernet link, else Network) is
/// only used when Advanced setup is off — with it on, Start goes to Keyboard first
/// regardless.
fn welcome_page(
    nav: &adw::NavigationView,
    state: Rc<WizardState>,
    keyboard_page: adw::NavigationPage,
    next_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let description = if simulate_mode() {
        "SIMULATION MODE — GENTOO_INSTALLER_SIMULATE is set, no real changes will be made. \
         Automated btrfs partitioning, kernel profile detection, Limine boot setup."
    } else {
        "Automated btrfs partitioning, kernel profile detection, Limine boot setup."
    };
    let title = gtk::Label::builder().label("Gentoo Installer").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Center).build();
    let subtitle = gtk::Label::builder().label(description).css_classes(vec!["dim-label".to_string()]).wrap(true).justify(gtk::Justification::Center).build();

    // Three at-a-glance facts as tiles, the way the store's detail page shows download
    // size / build time — what the installer will do, before asking anything.
    let tiles = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).homogeneous(true).margin_top(12).build();
    for (icon, head, text) in [
        ("drive-harddisk-symbolic", "Disk", "btrfs, ESP and swap laid out for you"),
        ("cpu-symbolic", "Kernel", "prebuilt for your CPU and GPU"),
        ("system-run-symbolic", "Boot", "Limine, UEFI or BIOS"),
    ] {
        let image = gtk::Image::builder().icon_name(icon).pixel_size(22).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let badge = gtk::Box::builder().css_classes(vec!["info-tile-icon".to_string()]).halign(gtk::Align::Center).build();
        badge.append(&image);
        let head = gtk::Label::builder().label(head).css_classes(vec!["section-heading".to_string()]).build();
        let text = gtk::Label::builder().label(text).css_classes(vec!["dim-label".to_string(), "caption".to_string()]).wrap(true).justify(gtk::Justification::Center).build();
        let tile = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(vec!["card".to_string(), "info-tile".to_string()]).build();
        tile.append(&badge);
        tile.append(&head);
        tile.append(&text);
        tiles.append(&tile);
    }

    let button = gtk::Button::builder()
        .label("Start")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .margin_top(24)
        .build();

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).valign(gtk::Align::Center).margin_start(24).margin_end(24).build();
    content.append(&title);
    content.append(&subtitle);
    content.append(&tiles);
    content.append(&button);
    let clamp = adw::Clamp::builder().child(&content).maximum_size(680).valign(gtk::Align::Center).build();

    let page = adw::NavigationPage::builder()
        .title("Welcome")
        .child(&clamp)
        .build();
    page.set_tag(Some("welcome"));

    let nav = nav.clone();
    button.connect_clicked(move |_| {
        if state.advanced.get() {
            nav.push(&keyboard_page);
        } else {
            nav.push(&next_page);
        }
    });

    page
}

/// Advanced-setup only: keyboard layout, auto-detected default via `keyboard::detect_current`.
fn keyboard_select_page(nav: adw::NavigationView, state: Rc<WizardState>, timezone_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Keyboard layout").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder().label("Type below to check it's right.").css_classes(vec!["dim-label".to_string()]).halign(gtk::Align::Start).build();

    let layouts = keyboard::list_layouts();
    let current = state.keyboard_layout.borrow().clone();
    let display_strings: Vec<String> = layouts.iter().map(|l| format!("{} — {}", l.code, l.description)).collect();
    let selected_index = layouts.iter().position(|l| l.code == current).unwrap_or(0) as u32;

    let model = gtk::StringList::new(&display_strings.iter().map(String::as_str).collect::<Vec<_>>());
    let dropdown = gtk::DropDown::builder().model(&model).selected(selected_index).build();
    dropdown.set_enable_search(true);

    {
        let state = state.clone();
        let layouts = layouts.clone();
        dropdown.connect_selected_notify(move |dd| {
            if let Some(l) = layouts.get(dd.selected() as usize) {
                *state.keyboard_layout.borrow_mut() = l.code.clone();
            }
        });
    }

    let test_field = gtk::Entry::builder().placeholder_text("Type here to test").build();

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&dropdown);
    content.append(&test_field);

    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&timezone_page));
    }

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Keyboard").child(&clamp).build();
    page.set_tag(Some("keyboard"));
    page
}

/// Advanced-setup only: time zone, auto-detected default via `timezone::detect_current`.
/// `next_page` is Disk directly if there's already an ethernet link, else Network.
fn timezone_select_page(nav: adw::NavigationView, state: Rc<WizardState>, next_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Where are you?").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder().label("This sets your time zone and clock.").css_classes(vec!["dim-label".to_string()]).halign(gtk::Align::Start).build();

    let zones = tz::list_zones();
    let current = state.timezone.borrow().clone();
    let selected_index = zones.iter().position(|z| *z == current).unwrap_or(0) as u32;

    let model = gtk::StringList::new(&zones.iter().map(String::as_str).collect::<Vec<_>>());
    let dropdown = gtk::DropDown::builder().model(&model).selected(selected_index).build();
    dropdown.set_enable_search(true);

    {
        let state = state.clone();
        let zones = zones.clone();
        dropdown.connect_selected_notify(move |dd| {
            if let Some(z) = zones.get(dd.selected() as usize) {
                *state.timezone.borrow_mut() = z.clone();
            }
        });
    }

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&dropdown);

    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Time zone").child(&clamp).build();
    page.set_tag(Some("timezone"));
    page
}

/// Advanced-setup only: compositor choice, niri preselected (the silent default when
/// Advanced setup is off entirely). `next_page` is Disk directly if there's already an
/// ethernet link, else Network — same choice `timezone_select_page` makes.
fn wm_select_page(nav: adw::NavigationView, state: Rc<WizardState>, next_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Desktop").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder()
        .label("Pick a compositor. Noctalia (the shell/bar) is installed alongside any of these.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let current = state.wm.get();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(2)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(12)
        .homogeneous(true)
        .build();
    let mut group: Option<gtk::ToggleButton> = None;
    for (i, choice) in WmChoice::ALL.iter().enumerate() {
        let tile = gtk::ToggleButton::builder()
            .label(choice.display_name())
            .css_classes(vec!["category-tile".to_string(), format!("category-tile-{}", i % 6)])
            .active(*choice == current)
            .build();
        match &group {
            Some(first) => tile.set_group(Some(first)),
            None => group = Some(tile.clone()),
        }
        let state = state.clone();
        let choice = *choice;
        tile.connect_toggled(move |t| {
            if t.is_active() {
                state.wm.set(choice);
            }
        });
        grid.insert(&tile, -1);
    }

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&grid);

    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Desktop").child(&clamp).build();
    page.set_tag(Some("wm"));
    page
}

/// Advanced-setup only: which GPU driver/kernel build to use. Detection picks the default
/// (listed first and marked); the rest are for machines where the hardware can't say what
/// the user wants (nouveau instead of the proprietary driver, for one).
fn gpu_select_page(nav: adw::NavigationView, state: Rc<WizardState>, next_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Graphics driver").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let detected = hardware::Profile::detect().ok().map(|p| p.gpu);
    let body = gtk::Label::builder()
        .label("This picks the kernel build and, for NVIDIA, whether the proprietary driver is compiled for it.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let mut choices: Vec<hardware::Gpu> = Vec::new();
    if let Some(d) = detected {
        choices.push(d);
    }
    choices.extend(hardware::Gpu::ALL.iter().copied().filter(|g| Some(*g) != detected));
    let labels: Vec<String> = choices
        .iter()
        .map(|g| if Some(*g) == detected { format!("{} — detected", g.display_name()) } else { g.display_name().to_string() })
        .collect();
    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let dropdown = gtk::DropDown::builder().model(&model).selected(0).build();
    {
        let state = state.clone();
        let choices = choices.clone();
        dropdown.connect_selected_notify(move |dd| {
            // Index 0 is the detected GPU: leave the override unset so a failed detection
            // at install time still falls through to the normal path.
            let picked = choices.get(dd.selected() as usize).copied();
            state.gpu.set(if dd.selected() == 0 && detected.is_some() { None } else { picked });
        });
    }

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&dropdown);
    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }
    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Graphics").child(&clamp).build();
    page.set_tag(Some("gpu"));
    page
}

/// Advanced-setup only: optional software, one switch per group in `installer_core::packages`.
/// The defaults (terminal, browser, CLI tools) are ticked; with Advanced off they are
/// installed anyway.
fn packages_select_page(nav: adw::NavigationView, state: Rc<WizardState>, next_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Software").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder()
        .label("Installed after the desktop. Prebuilt where Gentoo offers a binary; everything else compiles.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(vec!["boxed-list".to_string()]).build();
    for group in installer_core::packages::GROUPS {
        let row = adw::ActionRow::builder().title(group.name).subtitle(group.description).build();
        let switch = gtk::Switch::builder().valign(gtk::Align::Center).active(state.packages.borrow().iter().any(|id| id == group.id)).build();
        let state = state.clone();
        let id = group.id.to_string();
        switch.connect_state_set(move |_, on| {
            let mut ids = state.packages.borrow_mut();
            ids.retain(|i| *i != id);
            if on {
                ids.push(id.clone());
            }
            glib::Propagation::Proceed
        });
        row.add_suffix(&switch);
        row.set_activatable_widget(Some(&switch));
        list.append(&row);
    }

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&list);
    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }
    let scroll = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).build();
    let clamp = adw::Clamp::builder().child(&scroll).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Software").child(&clamp).build();
    page.set_tag(Some("packages"));
    page
}


/// Advanced-setup only: `-O2`/`-O3` for `make.conf`, O2 preselected (the silent default
/// when Advanced setup is off entirely — see `make_conf::OptLevel`'s doc comment for why
/// O2 is the recommended choice, not just an arbitrary default).
fn opt_level_select_page(nav: adw::NavigationView, state: Rc<WizardState>, next_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Build optimization").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder()
        .label("-O3 rarely measurably outperforms -O2 outside numeric-heavy code, and makes builds slower and binaries larger. Leave this off unless you have a specific reason.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).build();
    let switch_label = gtk::Label::builder().label("Use -O3").halign(gtk::Align::Start).hexpand(true).build();
    let switch = gtk::Switch::builder().active(state.opt_level.get() == OptLevel::O3).valign(gtk::Align::Center).build();
    {
        let state = state.clone();
        switch.connect_state_set(move |_, active| {
            state.opt_level.set(if active { OptLevel::O3 } else { OptLevel::O2 });
            glib::Propagation::Proceed
        });
    }
    row.append(&switch_label);
    row.append(&switch);

    let pkg_body = gtk::Label::builder()
        .label(
            "By default, large packages (compilers, graphics libraries, desktop toolkits — \
             exactly what pulls in the longest build times) come down as prebuilt binaries from \
             Gentoo's own official package host instead of compiling from source. Overlay-only \
             packages (niri, Hyprland, Noctalia...) always compile from source regardless — only \
             the official Gentoo tree has prebuilt binaries.",
        )
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let pkg_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).margin_top(12).build();
    let pkg_switch_label = gtk::Label::builder().label("Build everything from source").halign(gtk::Align::Start).hexpand(true).build();
    let pkg_switch = gtk::Switch::builder().active(state.package_mode.get() == PackageMode::Source).valign(gtk::Align::Center).build();
    {
        let state = state.clone();
        pkg_switch.connect_state_set(move |_, active| {
            state.package_mode.set(if active { PackageMode::Source } else { PackageMode::default() });
            glib::Propagation::Proceed
        });
    }
    pkg_row.append(&pkg_switch_label);
    pkg_row.append(&pkg_switch);

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&row);
    content.append(&pkg_body);
    content.append(&pkg_row);

    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Optimization").child(&clamp).build();
    page.set_tag(Some("opt-level"));
    page
}

enum NetCmd {
    Scan,
    Connect { path: String, passphrase: Option<String> },
}

enum NetEvt {
    Scanned(Vec<network::Network>),
    ScanFailed(String),
    Connected,
    ConnectFailed(String),
}

/// Runs the `IwdClient` on a dedicated thread with its own tokio runtime for the
/// lifetime of the app: the client's D-Bus connection has to stay open for our
/// registered iwd `Agent` to keep receiving `RequestPassphrase` callbacks, so this can't
/// be the usual "spin up a runtime, do one thing, tear it down" pattern used elsewhere
/// in this file (e.g. `disk::list`). Commands go in over a tokio channel (send is
/// non-blocking from the GTK thread); events come back over a `std::sync::mpsc` polled
/// by `glib::timeout_add_local`, same bridge shape as `spawn_install`.
fn spawn_network_worker() -> (tokio::sync::mpsc::UnboundedSender<NetCmd>, std_mpsc::Receiver<NetEvt>) {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<NetCmd>();
    let (evt_tx, evt_rx) = std_mpsc::channel::<NetEvt>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            let client = match network::IwdClient::connect().await {
                Ok(c) => c,
                Err(e) => {
                    let _ = evt_tx.send(NetEvt::ScanFailed(e.to_string()));
                    return;
                }
            };

            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    NetCmd::Scan => {
                        if let Err(e) = client.request_scan().await {
                            let _ = evt_tx.send(NetEvt::ScanFailed(e.to_string()));
                            continue;
                        }
                        // iwd's Scan() returns once scanning *starts*, not when results
                        // are ready — there's no blocking "wait for scan" call.
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        match client.scan().await {
                            Ok(networks) => {
                                let _ = evt_tx.send(NetEvt::Scanned(networks));
                            }
                            Err(e) => {
                                let _ = evt_tx.send(NetEvt::ScanFailed(e.to_string()));
                            }
                        }
                    }
                    NetCmd::Connect { path, passphrase } => match client.connect_to(&path, passphrase.as_deref()).await {
                        Ok(()) => {
                            let _ = evt_tx.send(NetEvt::Connected);
                        }
                        Err(e) => {
                            let _ = evt_tx.send(NetEvt::ConnectFailed(e.to_string()));
                        }
                    },
                }
            }
        });
    });

    (cmd_tx, evt_rx)
}

/// Screen script §4: skipped silently if already connected (handled by the caller —
/// see `target_after_prelude` in `build`), so this only ever renders when there's no
/// ethernet link. Network is optional — `Continue`/`Continue without a network` both
/// advance regardless of connection state, since everything the installer needs is
/// already on this device.
fn network_page_build(nav: adw::NavigationView, disk_page: adw::NavigationPage) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Connect to a network").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder()
        .label("Gentoo installs everything it needs from this device, so a connection is optional. It's used to check for updates on first start.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let status_label = gtk::Label::builder().halign(gtk::Align::Start).css_classes(vec!["dim-label".to_string()]).visible(false).build();
    let networks_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(12).build();

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(16).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&status_label);
    content.append(&networks_box);

    let continue_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    let skip_button = gtk::Button::builder().label("Continue without a network").css_classes(vec!["flat".to_string()]).halign(gtk::Align::End).build();
    let footer = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).halign(gtk::Align::End).margin_top(12).build();
    footer.append(&skip_button);
    footer.append(&continue_button);
    content.append(&footer);
    {
        let nav = nav.clone();
        let disk_page = disk_page.clone();
        continue_button.connect_clicked(move |_| nav.push(&disk_page));
    }
    {
        let nav = nav.clone();
        skip_button.connect_clicked(move |_| nav.push(&disk_page));
    }

    let (cmd_tx, evt_rx): (Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<NetCmd>>>>, _) = {
        let (tx, rx) = spawn_network_worker();
        (Rc::new(RefCell::new(Some(tx))), rx)
    };
    let networks_cache: Rc<RefCell<Vec<network::Network>>> = Rc::new(RefCell::new(Vec::new()));

    // Kick off an initial scan as soon as the page is first shown.
    {
        let cmd_tx = cmd_tx.clone();
        let status_label = status_label.clone();
        content.connect_map(move |_| {
            status_label.set_label("Scanning for networks…");
            status_label.set_visible(true);
            if let Some(tx) = cmd_tx.borrow().as_ref() {
                let _ = tx.send(NetCmd::Scan);
            }
        });
    }

    glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
        while let Ok(evt) = evt_rx.try_recv() {
            match evt {
                NetEvt::Scanned(networks) => {
                    status_label.set_visible(false);
                    *networks_cache.borrow_mut() = networks.clone();
                    while let Some(child) = networks_box.first_child() {
                        networks_box.remove(&child);
                    }
                    for net in &networks {
                        let row = network_row(net.clone(), cmd_tx.clone(), status_label.clone());
                        networks_box.append(&row);
                    }
                }
                NetEvt::ScanFailed(e) => {
                    status_label.set_label(&format!("Couldn't scan for networks: {e}"));
                    status_label.set_visible(true);
                }
                NetEvt::Connected => {
                    status_label.set_label("Connected.");
                    status_label.set_visible(true);
                }
                NetEvt::ConnectFailed(e) => {
                    status_label.set_label(&format!("Couldn't connect: {e}"));
                    status_label.set_visible(true);
                }
            }
        }
        glib::ControlFlow::Continue
    });

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Network").child(&clamp).build();
    page.set_tag(Some("network"));
    page
}

/// One scanned network: SSID + signal + lock icon, with a `Connect` button that reveals
/// an inline password field for secured networks (per screen script §4 — "an inline
/// password field ... within the row") rather than a separate dialog.
fn network_row(
    net: network::Network,
    cmd_tx: Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<NetCmd>>>>,
    status_label: gtk::Label,
) -> gtk::Box {
    let icon = gtk::Image::from_icon_name(if net.secured { "network-wireless-encrypted-symbolic" } else { "network-wireless-symbolic" });
    let ssid_label = gtk::Label::builder().label(&net.ssid).halign(gtk::Align::Start).hexpand(true).build();
    let connect_button = gtk::Button::builder().label("Connect").css_classes(vec!["flat".to_string()]).build();

    let header_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
    header_row.append(&icon);
    header_row.append(&ssid_label);
    header_row.append(&connect_button);

    let password_entry = gtk::PasswordEntry::builder().placeholder_text("Password").show_peek_icon(true).visible(false).build();

    let wrapper = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
    wrapper.append(&header_row);
    wrapper.append(&password_entry);

    let secured = net.secured;
    let path = net.path.clone();
    connect_button.connect_clicked(move |btn| {
        if secured && !password_entry.is_visible() {
            password_entry.set_visible(true);
            password_entry.grab_focus();
            btn.set_label("Connect");
            return;
        }
        let passphrase = if secured { Some(password_entry.text().to_string()) } else { None };
        status_label.set_label("Connecting…");
        status_label.set_visible(true);
        if let Some(tx) = cmd_tx.borrow().as_ref() {
            let _ = tx.send(NetCmd::Connect { path: path.clone(), passphrase });
        }
    });

    wrapper
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

fn disk_select_page(nav: adw::NavigationView, state: Rc<WizardState>, page_menus: PageMenus) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Where should Gentoo go?")
        .css_classes(vec!["hero-title".to_string()])
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
            *state.profile.borrow_mut() = Some(p);
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

    *state.existing_os.borrow_mut() = existing_os_found.clone();
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
                    *state.selected_disk.borrow_mut() = Some(d.clone());
                    next_button.set_sensitive(true);
                    first_button = Some(button.clone());
                }

                let state = state.clone();
                let d = d.clone();
                button.connect_toggled(move |b| {
                    if b.is_active() {
                        *state.selected_disk.borrow_mut() = Some(d.clone());
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

    let account_page = Rc::new(RefCell::new(None::<adw::NavigationPage>));
    {
        let nav = nav.clone();
        let state = state.clone();
        let account_page = account_page.clone();
        next_button.connect_clicked(move |_| {
            if account_page.borrow().is_none() {
                let page = account_page_build(nav.clone(), state.clone());
                nav.add(&page);
                *account_page.borrow_mut() = Some(page);
            }
            if let Some(page) = account_page.borrow().as_ref() {
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

    // The Disk page's own "⋯" item: built here, where `nav` and `state` are directly in
    // scope, rather than in a separate free function reached into from outside — a
    // `GtkPopover` lives on its own surface, so there's no reliable way to find a widget
    // inside one via a widget-tree walk from elsewhere.
    let manual_button = gtk::Button::builder()
        .label("Manual partitioning…")
        .css_classes(vec!["flat".to_string()])
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();
    let manual_popover = gtk::Popover::builder().child(&manual_button).build();
    // Sensitivity depends on Advanced (toggled on the Welcome page, potentially after
    // this popover was built), so it's refreshed each time the popover opens rather
    // than fixed once at construction.
    {
        let state = state.clone();
        let manual_button = manual_button.clone();
        manual_popover.connect_show(move |_| {
            manual_button.set_sensitive(state.advanced.get());
        });
    }
    let manual_page: Rc<RefCell<Option<adw::NavigationPage>>> = Rc::new(RefCell::new(None));
    {
        let nav = nav.clone();
        let state = state.clone();
        let manual_popover = manual_popover.clone();
        manual_button.connect_clicked(move |_| {
            manual_popover.popdown();
            if manual_page.borrow().is_none() {
                let page = manual_partition_page_build(nav.clone(), state.clone());
                nav.add(&page);
                *manual_page.borrow_mut() = Some(page);
            }
            if let Some(page) = manual_page.borrow().as_ref() {
                nav.push(page);
            }
        });
    }
    page_menus.borrow_mut().insert("disk-select".to_string(), manual_popover);

    page
}

/// Advanced-setup only, reached from the Disk page's "⋯" menu: the only manual control
/// our partitioning backend actually supports is root filesystem + swap size — it
/// creates exactly ESP+swap+root, not an arbitrary partition table, so that's the honest
/// scope of "manual" here rather than a full GParted-style editor this codebase can't
/// back.
fn manual_partition_page_build(nav: adw::NavigationView, state: Rc<WizardState>) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Partitions").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder()
        .label("Gentoo needs a root partition, and an EFI system partition on this machine.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let btrfs_toggle = gtk::ToggleButton::builder().label("btrfs").active(state.manual_root_fs.get() == partition::RootFs::Btrfs).build();
    let ext4_toggle = gtk::ToggleButton::builder().label("ext4").group(&btrfs_toggle).active(state.manual_root_fs.get() == partition::RootFs::Ext4).build();
    let fs_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
    fs_row.append(&gtk::Label::builder().label("Root filesystem").halign(gtk::Align::Start).hexpand(true).build());
    fs_row.append(&btrfs_toggle);
    fs_row.append(&ext4_toggle);
    {
        let state = state.clone();
        btrfs_toggle.connect_toggled(move |b| {
            if b.is_active() {
                state.manual_root_fs.set(partition::RootFs::Btrfs);
            }
        });
    }
    {
        let state = state.clone();
        ext4_toggle.connect_toggled(move |b| {
            if b.is_active() {
                state.manual_root_fs.set(partition::RootFs::Ext4);
            }
        });
    }

    let swap_adjustment = gtk::Adjustment::new(state.swap_gib() as f64, 8.0, 96.0, 1.0, 4.0, 0.0);
    let swap_spin = gtk::SpinButton::new(Some(&swap_adjustment), 1.0, 0);
    let swap_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
    swap_row.append(&gtk::Label::builder().label("Swap size (GiB)").halign(gtk::Align::Start).hexpand(true).build());
    swap_row.append(&swap_spin);
    {
        let state = state.clone();
        swap_spin.connect_value_changed(move |s| {
            *state.manual_swap_gib.borrow_mut() = Some(s.value() as u64);
        });
    }

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(20).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&fs_row);
    content.append(&swap_row);

    let save_button = gtk::Button::builder().label("Use these partitions").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&save_button);
    {
        let nav = nav.clone();
        save_button.connect_clicked(move |_| {
            nav.pop();
        });
    }

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Partitions").child(&clamp).build();
    page.set_tag(Some("manual-partition"));
    page
}

/// Screen script §9: username + password, before Confirm. Root stays locked; this
/// account gets `wheel` (see `installer_core::account::create`).
fn account_page_build(nav: adw::NavigationView, state: Rc<WizardState>) -> adw::NavigationPage {
    let heading = gtk::Label::builder().label("Create your account").css_classes(vec!["hero-title".to_string()]).halign(gtk::Align::Start).build();
    let body = gtk::Label::builder()
        .label("This account can install software and change system settings.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let username_entry = gtk::Entry::builder().placeholder_text("Username").text(state.username.borrow().as_str()).build();
    let password_entry = gtk::PasswordEntry::builder().placeholder_text("Password").show_peek_icon(true).build();
    let confirm_entry = gtk::PasswordEntry::builder().placeholder_text("Confirm password").show_peek_icon(true).build();

    let error_label = gtk::Label::builder().halign(gtk::Align::Start).wrap(true).visible(false).css_classes(vec!["error".to_string()]).build();

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).margin_start(24).margin_end(24).margin_top(48).margin_bottom(24).build();
    content.append(&heading);
    content.append(&body);
    content.append(&username_entry);
    content.append(&password_entry);
    content.append(&confirm_entry);
    content.append(&error_label);

    let next_button = gtk::Button::builder().label("Continue").css_classes(vec!["suggested-action".to_string(), "pill".to_string()]).halign(gtk::Align::End).margin_top(24).build();
    content.append(&next_button);

    let confirm_page = Rc::new(RefCell::new(None::<adw::NavigationPage>));
    {
        let nav = nav.clone();
        let state = state.clone();
        let confirm_page = confirm_page.clone();
        next_button.connect_clicked(move |_| {
            let username = username_entry.text().to_string();
            let password = password_entry.text().to_string();
            let confirm = confirm_entry.text().to_string();

            if username.trim().is_empty() {
                error_label.set_label("Enter a username.");
                error_label.set_visible(true);
                return;
            }
            if password.len() < 8 {
                error_label.set_label("Password needs to be at least 8 characters.");
                error_label.set_visible(true);
                return;
            }
            if password != confirm {
                error_label.set_label("Passwords don't match.");
                error_label.set_visible(true);
                return;
            }
            error_label.set_visible(false);

            *state.username.borrow_mut() = username;
            *state.password.borrow_mut() = password;

            if confirm_page.borrow().is_none() {
                let page = confirm_page_build(nav.clone(), state.clone());
                nav.add(&page);
                *confirm_page.borrow_mut() = Some(page);
            }
            if let Some(page) = confirm_page.borrow().as_ref() {
                nav.push(page);
            }
        });
    }

    let clamp = adw::Clamp::builder().child(&content).maximum_size(560).build();
    let page = adw::NavigationPage::builder().title("Account").child(&clamp).build();
    page.set_tag(Some("account"));
    page
}

/// Screen script §10: "styled deliberately plainer and denser than every other page —
/// no tiles, no large heading, tighter leading. Reading it is the point." Plain sentence
/// lines instead of a StatusPage; only lines backed by something real are shown.
fn confirm_page_build(nav: adw::NavigationView, state: Rc<WizardState>) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Review before installing")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let simulate = simulate_mode();
    let disk = state.selected_disk.borrow().clone();
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

        let erase_line = match state.existing_os.borrow().as_ref() {
            Some(os) => format!("Erase everything on this disk, including {os}."),
            None => "Erase everything on this disk.".to_string(),
        };
        sentences.append(&sentence(&erase_line, true));

        let username = state.username.borrow().clone();
        if !username.is_empty() {
            sentences.append(&sentence(&format!("Create an account for {username}."), false));
        }
        sentences.append(&sentence(&format!("Set the time zone to {}.", state.timezone.borrow()), false));
        sentences.append(&sentence(&format!("Install {} as your desktop.", state.wm.get().display_name()), false));
        if state.opt_level.get() == OptLevel::O3 {
            sentences.append(&sentence("Build with -O3 instead of -O2.", false));
        }
        if state.package_mode.get() == PackageMode::Source {
            sentences.append(&sentence("Build everything from source instead of using prebuilt binary packages.", false));
        }
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

    if let Some(d) = &disk {
        content.append(&partition_bar(d.size_bytes, state.swap_gib(), state.manual_root_fs.get()));
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
        let state = state.clone();
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
            let Some(disk) = state.selected_disk.borrow().clone() else { return };
            btn.set_sensitive(false);

            let layout = partition::plan_with_swap(&disk.path, state.manual_root_fs.get(), state.swap_gib());
            let account = if simulate {
                Account { username: "gentoo".into(), password: String::new() }
            } else {
                Account { username: state.username.borrow().clone(), password: state.password.borrow().clone() }
            };
            let opts = install::InstallOptions {
                layout,
                target: "/mnt/gentoo".into(),
                store: store::StoreConfig {
                    binhost_url: store_env.as_ref().map(|e| e.binhost_url.clone()).unwrap_or_default(),
                    overlay_git_url: store_env.as_ref().map(|e| e.overlay_git_url.clone()).unwrap_or_default(),
                    overlay_name: store_env.as_ref().map(|e| e.overlay_name.clone()).unwrap_or_default(),
                },
                wm_configs_git_url: store_env.as_ref().map(|e| e.wm_configs_git_url.clone()).unwrap_or_default(),
                kernel_base_name: store_env
                    .map(|e| e.kernel_base_name)
                    .unwrap_or_else(|| "gentoo-diy-kernel".into()),
                keyboard_layout: state.keyboard_layout.borrow().clone(),
                timezone: state.timezone.borrow().clone(),
                hostname: "gentoo".into(),
                locales: vec![installer_core::locale::DEFAULT_LOCALE.to_string()],
                account,
                gpu_override: state.gpu.get(),
                packages: state.packages.borrow().clone(),
                stage3: std::env::var("GENTOO_INSTALLER_STAGE3_URL").ok().filter(|u| !u.trim().is_empty()).map(|u| installer_core::stage3::Stage3Source::custom(u.trim(), std::env::var("GENTOO_INSTALLER_STAGE3_SHA512").ok())),
                wm: state.wm.get(),
                opt_level: state.opt_level.get(),
                package_mode: state.package_mode.get(),
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
fn partition_bar(disk_size_bytes: u64, swap_gib: u64, root_fs: partition::RootFs) -> gtk::Box {
    use installer_core::partition::ESP_SIZE_MIB;

    let total_mib = (disk_size_bytes / 1024 / 1024).max(1);
    let esp_mib = ESP_SIZE_MIB;
    let swap_mib = swap_gib * 1024;
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

    let root_fs_name = match root_fs {
        partition::RootFs::Btrfs => "btrfs",
        partition::RootFs::Ext4 => "ext4",
    };

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
    legend.append(&legend_row("gentoo-part-root", "root", root_mib * 1024 * 1024, root_fs_name));

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

const TOTAL_STEPS: f64 = 13.0;

fn progress_fraction(p: &install::Progress) -> f64 {
    let step = match p {
        install::Progress::Partitioning => 0.0,
        install::Progress::DownloadingStage3 => 1.0,
        install::Progress::UnpackingStage3 => 2.0,
        install::Progress::WritingMakeConf => 3.0,
        install::Progress::ConfiguringStore => 4.0,
        install::Progress::InstallingKernel { .. } => 5.0,
        install::Progress::InstallingGpuDriver => 6.0,
        install::Progress::WritingFstab => 7.0,
        install::Progress::SettingKeyboard => 8.0,
        install::Progress::SettingTimezone => 9.0,
        install::Progress::CreatingAccount => 10.0,
        install::Progress::InstallingDesktop => 11.0,
        install::Progress::InstallingBootloader => 12.0,
        install::Progress::Done => 13.0,
    };
    step / TOTAL_STEPS
}

/// Screen script §11 phase labels, applied to the phases this codebase actually
/// performs — the doc's own list includes steps nothing here does yet (building a
/// startup image), so those aren't claimed. Where our mechanism differs from the doc's
/// assumed squashfs image copy (we download+unpack an official stage3 instead), the
/// label says what's actually happening rather than borrowing the doc's phrase for a
/// different mechanism.
fn describe(p: &install::Progress) -> String {
    match p {
        install::Progress::Partitioning => "Preparing the disk".into(),
        install::Progress::DownloadingStage3 => "Downloading the base system".into(),
        install::Progress::UnpackingStage3 => "Setting up the base system".into(),
        install::Progress::WritingMakeConf => "Tuning build settings for your hardware".into(),
        install::Progress::ConfiguringStore => "Setting up package sources".into(),
        install::Progress::InstallingKernel { atom, degraded_by } => {
            if *degraded_by == 0 {
                format!("Installing your kernel ({atom}, exact hardware match)")
            } else {
                format!("Installing your kernel ({atom}, closest available match)")
            }
        }
        install::Progress::InstallingGpuDriver => "Building your Nvidia driver".into(),
        install::Progress::WritingFstab => "Setting up the file system".into(),
        install::Progress::SettingKeyboard => "Setting your keyboard layout".into(),
        install::Progress::SettingTimezone => "Setting your time zone".into(),
        install::Progress::CreatingAccount => "Creating your account".into(),
        install::Progress::InstallingDesktop => "Setting up your desktop".into(),
        install::Progress::InstallingBootloader => "Installing the bootloader".into(),
        install::Progress::Done => "Finishing up".into(),
    }
}
