use super::*;

/// Advanced-setup only: compositor choice, niri preselected (the silent default when
/// Advanced setup is off entirely). `next_page` is Disk directly if there's already an
/// ethernet link, else Network — same choice `timezone_select_page` makes.
pub(super) fn wm_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    next_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Desktop")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
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
            .css_classes(vec![
                "category-tile".to_string(),
                format!("category-tile-{}", i % 6),
            ])
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

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&grid);

    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }

    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let clamp = adw::Clamp::builder()
        .child(&scroll)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Desktop")
        .child(&clamp)
        .build();
    page.set_tag(Some("wm"));
    page
}

/// Advanced-setup only: which GPU driver/kernel build to use. Detection picks the default
/// (listed first and marked); the rest are for machines where the hardware can't say what
/// the user wants (nouveau instead of the proprietary driver, for one). With several adapters (a laptop with
/// integrated and discrete graphics) the page also says which the desktop is drawn on.
pub(super) fn gpu_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    next_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Graphics driver")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let detected = hardware::Profile::detect().ok().map(|p| p.gpu);
    let body = gtk::Label::builder()
        .label("This picks the kernel build and, for NVIDIA, whether the proprietary driver is compiled for it.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let adapters = std::process::Command::new("lspci")
        .args(["-nn", "-D"])
        .output()
        .ok()
        .map(|o| installer_core::gpu::parse_lspci(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();

    let mut choices: Vec<hardware::Gpu> = Vec::new();
    if let Some(d) = detected {
        choices.push(d);
    }
    choices.extend(
        hardware::Gpu::ALL
            .iter()
            .copied()
            .filter(|g| Some(*g) != detected),
    );

    // The drivers fold into one row: a drop-down's list would cover the Continue button below it, an expander pushes it down.
    let drivers = adw::ExpanderRow::builder()
        .title("Driver")
        .subtitle(choices[0].display_name())
        .build();
    let mut first: Option<gtk::CheckButton> = None;
    for (i, g) in choices.iter().enumerate() {
        let radio = gtk::CheckButton::new();
        match &first {
            None => first = Some(radio.clone()),
            Some(f) => radio.set_group(Some(f)),
        }
        radio.set_active(i == 0);
        let row = adw::ActionRow::builder()
            .title(g.display_name())
            .subtitle(if Some(*g) == detected { "detected" } else { "" })
            .activatable_widget(&radio)
            .build();
        row.add_prefix(&radio);
        drivers.add_row(&row);
        let state = state.clone();
        let g = *g;
        let drivers = drivers.clone();
        radio.connect_toggled(move |r| {
            if r.is_active() {
                // The detected GPU (index 0) leaves the override unset, so a failed detection at install
                // time still falls through to the normal path.
                state.gpu.set(if i == 0 && detected.is_some() {
                    None
                } else {
                    Some(g)
                });
                drivers.set_subtitle(g.display_name());
                drivers.set_expanded(false);
            }
        });
    }
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    list.append(&drivers);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);

    if !adapters.is_empty() {
        let found = adw::PreferencesGroup::builder()
            .title(if adapters.len() > 1 {
                "Graphics cards found"
            } else {
                "Graphics card found"
            })
            .build();
        for a in &adapters {
            found.add(
                &adw::ActionRow::builder()
                    .use_markup(false)
                    .title(&a.description)
                    .subtitle(&a.pci_addr)
                    .build(),
            );
        }
        content.append(&found);
    }
    if adapters.len() > 1 {
        // Which one the desktop is drawn on; drivers for every card found are installed either way.
        let render = adw::PreferencesGroup::builder()
            .title("Draw the desktop on")
            .description("Drivers for all the cards are installed. Games and programs can still use the other one.")
            .build();
        let auto = gtk::CheckButton::new();
        let integrated = gtk::CheckButton::new();
        integrated.set_group(Some(&auto));
        auto.set_active(state.render.get() == installer_core::gpu::RenderPreference::Auto);
        integrated
            .set_active(state.render.get() == installer_core::gpu::RenderPreference::Integrated);
        let auto_row = adw::ActionRow::builder()
            .title("The most capable card")
            .subtitle("The discrete one: smoother, uses more power")
            .activatable_widget(&auto)
            .build();
        auto_row.add_prefix(&auto);
        let integrated_row = adw::ActionRow::builder()
            .title("The integrated card")
            .subtitle("Saves battery on a laptop; the screen is usually wired to it")
            .activatable_widget(&integrated)
            .build();
        integrated_row.add_prefix(&integrated);
        render.add(&auto_row);
        render.add(&integrated_row);
        {
            let state = state.clone();
            auto.connect_toggled(move |r| {
                if r.is_active() {
                    state
                        .render
                        .set(installer_core::gpu::RenderPreference::Auto);
                }
            });
        }
        {
            let state = state.clone();
            integrated.connect_toggled(move |r| {
                if r.is_active() {
                    state
                        .render
                        .set(installer_core::gpu::RenderPreference::Integrated);
                }
            });
        }
        content.append(&render);
    }
    content.append(&list);
    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let clamp = adw::Clamp::builder()
        .child(&scroll)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Graphics")
        .child(&clamp)
        .build();
    page.set_tag(Some("gpu"));
    page
}

/// Advanced-setup only: optional software, one switch per group in `installer_core::packages`.
/// The defaults (terminal, browser, CLI tools) are ticked; with Advanced off they are
/// installed anyway.
pub(super) fn packages_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    next_page: adw::NavigationPage,
    normal_next_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Software")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("Installed after the desktop. Prebuilt where a binary exists; everything else compiles.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    for group in installer_core::packages::GROUPS {
        if !installer_core::offline::group_available(group.id) {
            continue; // not on an offline medium
        }
        let row = adw::ActionRow::builder()
            .title(group.name)
            .subtitle(group.description)
            .build();
        let switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(state.packages.borrow().iter().any(|id| id == group.id))
            .build();
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

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&list);
    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| {
            // Advanced continues to the optimization level; the normal flow skips it.
            if state.advanced.get() {
                nav.push(&next_page);
            } else {
                nav.push(&normal_next_page);
            }
        });
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let clamp = adw::Clamp::builder()
        .child(&scroll)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Software")
        .child(&clamp)
        .build();
    page.set_tag(Some("packages"));
    page
}

/// Advanced-setup only: `-O2`/`-O3` for `make.conf`, O2 preselected (the silent default
/// when Advanced setup is off entirely — see `make_conf::OptLevel`'s doc comment for why
/// O2 is the recommended choice, not just an arbitrary default).
pub(super) fn opt_level_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    next_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Build optimization")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("-O3 rarely measurably outperforms -O2 outside numeric-heavy code, and makes builds slower and binaries larger. Leave this off unless you have a specific reason.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    let switch_label = gtk::Label::builder()
        .label("Use -O3")
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    let switch = gtk::Switch::builder()
        .active(state.opt_level.get() == OptLevel::O3)
        .valign(gtk::Align::Center)
        .build();
    {
        let state = state.clone();
        switch.connect_state_set(move |_, active| {
            state
                .opt_level
                .set(if active { OptLevel::O3 } else { OptLevel::O2 });
            glib::Propagation::Proceed
        });
    }
    row.append(&switch_label);
    row.append(&switch);

    let pkg_body = gtk::Label::builder()
        .label(
            "By default the packages come down prebuilt, from Gentoo's package host and from this \
             project's own (niri, Noctalia and the packages built with different options), instead \
             of compiling from source. Turning this on compiles everything on this computer, which \
             takes hours.",
        )
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let pkg_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(12)
        .build();
    let pkg_switch_label = gtk::Label::builder()
        .label("Build everything from source")
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    let pkg_switch = gtk::Switch::builder()
        .active(state.package_mode.get() == PackageMode::Source)
        .valign(gtk::Align::Center)
        .build();
    {
        let state = state.clone();
        pkg_switch.connect_state_set(move |_, active| {
            state.package_mode.set(if active {
                PackageMode::Source
            } else {
                PackageMode::default()
            });
            glib::Propagation::Proceed
        });
    }
    pkg_row.append(&pkg_switch_label);
    pkg_row.append(&pkg_switch);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&row);
    content.append(&pkg_body);
    content.append(&pkg_row);

    // Secure Boot only means something on UEFI.
    if std::path::Path::new("/sys/firmware/efi").is_dir() {
        let sb_body = gtk::Label::builder()
            .label(
                "Signs the bootloader with a key made on this computer, so only the kernel installed here can start. \
                 Afterwards you add the certificate (/boot/secureboot/simple-linux-db.cer) to the firmware's Secure Boot \
                 keys and turn Secure Boot on. The key is stored unencrypted on the disk.",
            )
            .css_classes(vec!["dim-label".to_string()])
            .halign(gtk::Align::Start)
            .wrap(true)
            .build();
        let sb_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(12)
            .build();
        let sb_label = gtk::Label::builder()
            .label("Prepare Secure Boot")
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        let sb_switch = gtk::Switch::builder()
            .active(state.secure_boot.get())
            .valign(gtk::Align::Center)
            .build();
        {
            let state = state.clone();
            sb_switch.connect_state_set(move |_, active| {
                state.secure_boot.set(active);
                glib::Propagation::Proceed
            });
        }
        sb_row.append(&sb_label);
        sb_row.append(&sb_switch);
        content.append(&sb_body);
        content.append(&sb_row);
    }

    let next_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&next_button);
    {
        let nav = nav.clone();
        next_button.connect_clicked(move |_| nav.push(&next_page));
    }

    // Secure Boot and the source/binary switch make this page taller than the window on a small screen.
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let clamp = adw::Clamp::builder()
        .child(&scroll)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Optimization")
        .child(&clamp)
        .build();
    page.set_tag(Some("opt-level"));
    page
}
