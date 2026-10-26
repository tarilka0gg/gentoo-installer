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
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(a.display_name())
                .subtitle(format!(
                    "{} · {}",
                    if a.is_discrete(&adapters) {
                        "Discrete card"
                    } else {
                        "Integrated graphics"
                    },
                    a.pci_addr
                ))
                .build();
            let icon = gtk::Image::from_icon_name("video-display-symbolic");
            icon.set_pixel_size(20);
            row.add_prefix(&icon);
            found.add(&row);
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

/// A screenshot at its own size in a window of its own, over the wizard; a click on it or Esc closes it.
fn show_screenshot(parent: &gtk::Window, title: &str, texture: &gtk::gdk::Texture) {
    let picture = gtk::Picture::builder()
        .paintable(texture)
        .content_fit(gtk::ContentFit::Contain)
        .can_shrink(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(texture.width() + 24)
        .default_height(texture.height() + 24)
        .child(&picture)
        .build();
    let click = gtk::GestureClick::new();
    let closing = window.clone();
    click.connect_released(move |_, _, _, _| closing.close());
    picture.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let closing = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            closing.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
}

/// One package of a group, as the Software page shows it: the name, a sentence about it, and its screenshot when there is one.
fn app_row(group_id: &str, app: &installer_core::packages::AppEntry) -> gtk::ListBoxRow {
    let name = gtk::Label::builder()
        .label(app.name)
        .css_classes(vec!["heading".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let text = gtk::Label::builder()
        .label(app.description)
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .xalign(0.0)
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(16)
        .margin_end(16)
        .build();
    column.append(&name);
    column.append(&text);
    if let Some(bytes) = installer_core::packages::screen(group_id, app.atom) {
        // a screenshot that cannot be read is simply left out
        if let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(bytes)) {
            let picture = gtk::Picture::builder()
                .paintable(&texture)
                .can_shrink(true)
                .content_fit(gtk::ContentFit::ScaleDown)
                .halign(gtk::Align::Start)
                .css_classes(vec!["card".to_string()])
                .build();
            picture.set_size_request(-1, 220);
            // a click opens the picture at its own size in a window of its own
            picture.set_cursor_from_name(Some("zoom-in"));
            picture.set_tooltip_text(Some("Click to enlarge"));
            let click = gtk::GestureClick::new();
            let (weak, title) = (picture.downgrade(), app.name.to_string());
            click.connect_released(move |_, _, _, _| {
                if let Some(parent) = weak
                    .upgrade()
                    .and_then(|p| p.root())
                    .and_downcast::<gtk::Window>()
                {
                    show_screenshot(&parent, &title, &texture);
                }
            });
            picture.add_controller(click);
            column.append(&picture);
        }
    }
    gtk::ListBoxRow::builder()
        .child(&column)
        .activatable(false)
        .selectable(false)
        .build()
}

/// Advanced-setup only: the system type (flavor) and the optional software, one switch per group in `installer_core::packages`.
/// The defaults of the chosen flavor are ticked; with Advanced off they are installed anyway.
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

    // The system type: one flavor of the profiles. Choosing one ticks what it ticks by default and hides what needs a desktop on a
    // console system; `slp` on the installed system changes it later.
    let flavors = Rc::new(installer_core::packages::flavors());
    let flavor_names: Vec<&str> = flavors.iter().map(|f| f.name.as_str()).collect();
    let flavor_row = adw::ComboRow::builder()
        .title("System type")
        .model(&gtk::StringList::new(&flavor_names))
        .build();
    let current = state.flavor.borrow().clone();
    flavor_row.set_selected(flavors.iter().position(|f| f.id == current).unwrap_or(0) as u32);
    if let Some(f) = flavors.iter().find(|f| f.id == current) {
        flavor_row.set_subtitle(&f.description);
    }
    let flavor_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    flavor_list.append(&flavor_row);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    // (group id, its switch, its row, whether it needs a desktop). Each group is a row that opens inline into its packages: a name, a
    // sentence about each, and a screenshot where there is one (compiled in with the profiles).
    let mut group_rows: Vec<(String, gtk::Switch, adw::ExpanderRow, bool)> = Vec::new();
    for group in installer_core::packages::GROUPS.iter() {
        // On an offline image the groups that do not fit on the medium stay on the page, as in the online installer, but cannot be
        // ticked: they are installed on the finished system, with a network.
        let on_medium = installer_core::offline::group_available(group.id);
        let subtitle = if on_medium {
            group.description.to_string()
        } else {
            format!(
                "{} Not on this offline image: needs a network connection, and is installed on the finished system.",
                group.description
            )
        };
        let row = adw::ExpanderRow::builder()
            .title(group.name)
            .subtitle(subtitle)
            .build();
        let switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .sensitive(on_medium)
            .active(on_medium && state.packages.borrow().iter().any(|id| id == group.id))
            .build();
        let state = state.clone();
        let id = group.id.to_string();
        switch.connect_state_set({
            let id = id.clone();
            move |_, on| {
                let mut ids = state.packages.borrow_mut();
                ids.retain(|i| *i != id);
                if on {
                    ids.push(id.clone());
                }
                glib::Propagation::Proceed
            }
        });
        row.add_suffix(&switch);
        for app in group.apps {
            row.add_row(&app_row(group.id, app));
        }
        list.append(&row);
        group_rows.push((id, switch, row, group.needs_desktop));
    }
    let group_rows = Rc::new(group_rows);
    {
        let state = state.clone();
        let flavors = flavors.clone();
        let group_rows = group_rows.clone();
        flavor_row.connect_selected_notify(move |row| {
            let Some(f) = flavors.get(row.selected() as usize) else {
                return;
            };
            row.set_subtitle(&f.description);
            *state.flavor.borrow_mut() = f.id.clone();
            for (id, switch, group_row, needs_desktop) in group_rows.iter() {
                let usable = f.desktop || !*needs_desktop;
                group_row.set_visible(usable);
                // the switch handler keeps `state.packages` in step
                switch.set_active(
                    usable
                        && installer_core::offline::group_available(id)
                        && f.defaults.iter().any(|d| d == id),
                );
            }
        });
    }
    // The page opens with the rows of the current flavor.
    if let Some(f) = flavors.iter().find(|f| f.id == current) {
        for (_, _, row, needs_desktop) in group_rows.iter() {
            row.set_visible(f.desktop || !*needs_desktop);
        }
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
    content.append(&flavor_list);
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
        .label("The defaults suit almost everyone. Change these only when you have a reason.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    // One switch per choice, its explanation under it.
    let group = adw::PreferencesGroup::new();
    let switch = |title: &str, subtitle: &str, active: bool| {
        let row = adw::SwitchRow::builder()
            .use_markup(false)
            .title(title)
            .subtitle(subtitle)
            .active(active)
            .build();
        group.add(&row);
        row
    };

    let o3 = switch(
        "Use -O3",
        "-O3 rarely measurably outperforms -O2 outside numeric-heavy code, and makes builds slower and binaries larger.",
        state.opt_level.get() == OptLevel::O3,
    );
    {
        let state = state.clone();
        o3.connect_active_notify(move |r| {
            state.opt_level.set(if r.is_active() {
                OptLevel::O3
            } else {
                OptLevel::O2
            });
        });
    }

    let source = switch(
        "Build everything from source",
        "By default the packages come down prebuilt, from Gentoo's package host and from this project's own (niri, Noctalia and the packages built with different options). Turning this on compiles everything on this computer, which takes hours.",
        state.package_mode.get() == PackageMode::Source,
    );
    {
        let state = state.clone();
        source.connect_active_notify(move |r| {
            state.package_mode.set(if r.is_active() {
                PackageMode::Source
            } else {
                PackageMode::default()
            });
        });
    }

    // Not on an offline image: building needs the kernel sources and the matrix from the network.
    if !installer_core::offline::is_active() {
        let kernel = switch(
            "Build the kernel for this computer",
            "A ready-made kernel for exactly your hardware is used when there is one, and the closest generic one otherwise. Turn this on to build one from the kernel matrix when there is no ready one: 10 to 60 minutes (it depends on the machine) and about 10 GB of disk space. If the build fails, the generic kernel stays.",
            state.build_kernel.get(),
        );
        let state = state.clone();
        kernel.connect_active_notify(move |r| state.build_kernel.set(r.is_active()));
    }

    // Secure Boot only means something on UEFI.
    if std::path::Path::new("/sys/firmware/efi").is_dir() {
        let sb = switch(
            "Prepare Secure Boot",
            "Signs the bootloader with a key made on this computer, so only the kernel installed here can start. Afterwards you add the certificate (/boot/secureboot/simple-linux-db.cer) to the firmware's Secure Boot keys and turn Secure Boot on. The key is stored unencrypted on the disk.",
            state.secure_boot.get(),
        );
        let state = state.clone();
        sb.connect_active_notify(move |r| state.secure_boot.set(r.is_active()));
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
    content.append(&group);

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
