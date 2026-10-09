use super::*;

pub(super) fn advanced_setup_popover(state: Rc<WizardState>) -> gtk::Popover {
    let label = gtk::Label::builder()
        .label("Advanced setup")
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    let switch = gtk::Switch::builder()
        .active(state.advanced.get())
        .valign(gtk::Align::Center)
        .build();
    switch.connect_state_set(move |_, active| {
        state.advanced.set(active);
        glib::Propagation::Proceed
    });

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    row.append(&label);
    row.append(&switch);

    gtk::Popover::builder().child(&row).build()
}

/// `next_page` (Software selection) is only used when Advanced setup is off — with it on, Start goes to Keyboard first
/// regardless.
pub(super) fn welcome_page(
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
    let title = gtk::Label::builder()
        .label("Gentoo Installer")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Center)
        .build();
    let subtitle = gtk::Label::builder()
        .label(description)
        .css_classes(vec!["dim-label".to_string()])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();

    // Three at-a-glance facts as tiles, the way the store's detail page shows download
    // size / build time — what the installer will do, before asking anything.
    let tiles = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .homogeneous(true)
        .margin_top(12)
        .build();
    for (icon, head, text) in [
        (
            "drive-harddisk-symbolic",
            "Disk",
            "btrfs, ESP and swap laid out for you",
        ),
        (
            "computer-symbolic",
            "Kernel",
            "prebuilt for your CPU and GPU",
        ),
        ("system-run-symbolic", "Boot", "Limine, UEFI or BIOS"),
    ] {
        let badge = icon_badge(icon, 22);
        let head = gtk::Label::builder()
            .label(head)
            .css_classes(vec!["section-heading".to_string()])
            .build();
        let text = gtk::Label::builder()
            .label(text)
            .css_classes(vec!["dim-label".to_string(), "caption".to_string()])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let tile = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(vec!["card".to_string(), "info-tile".to_string()])
            .build();
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

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .valign(gtk::Align::Center)
        .margin_start(24)
        .margin_end(24)
        .build();
    content.append(&title);
    content.append(&subtitle);
    content.append(&tiles);
    content.append(&button);
    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(680)
        .valign(gtk::Align::Center)
        .build();

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
