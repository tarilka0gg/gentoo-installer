use super::*;

/// Card-style disk picker (ported from elementary installer's `DiskGrid`/`DiskButton`):
/// a radio-grouped `ToggleButton` per disk with an icon, bold name, and path/size
/// subtitle, instead of a plain row list — the selected card gets an accent border via
/// the `:checked` CSS above.
pub(super) fn disk_card(
    disk: &disk::Disk,
    group_with: Option<&gtk::ToggleButton>,
) -> gtk::ToggleButton {
    let icon = gtk::Image::builder()
        .icon_name("drive-harddisk-symbolic")
        .pixel_size(32)
        .build();

    let name_label = gtk::Label::builder()
        .label(&disk.path)
        .halign(gtk::Align::Start)
        .css_classes(vec!["heading".to_string()])
        .build();
    let subtitle_label = gtk::Label::builder()
        .label(format!(
            "{}  {}",
            disk::format_size(disk.size_bytes),
            disk.model
        ))
        .halign(gtk::Align::Start)
        .css_classes(vec!["dim-label".to_string()])
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();

    let text_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .build();
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
pub(super) fn min_disk_bytes() -> u64 {
    use installer_core::partition::{ESP_SIZE_MIB, SWAP_MIN_GIB};
    (ESP_SIZE_MIB + SWAP_MIN_GIB * 1024 + 8 * 1024) * 1024 * 1024
}

pub(super) fn disk_select_page(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
    page_menus: PageMenus,
) -> adw::NavigationPage {
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

    let cards_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .build();

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
        tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(async {
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
                    .label(format!(
                        "Too small — needs at least {}",
                        disk::format_size(min_bytes)
                    ))
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

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(700)
        .build();

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
    page_menus
        .borrow_mut()
        .insert("disk-select".to_string(), manual_popover);

    page
}

/// Advanced-setup only, reached from the Disk page's "⋯" menu: the only manual control
/// our partitioning backend actually supports is root filesystem + swap size — it
/// creates exactly ESP+swap+root, not an arbitrary partition table, so that's the honest
/// scope of "manual" here rather than a full GParted-style editor this codebase can't
/// back.
pub(super) fn manual_partition_page_build(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Partitions")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("Gentoo needs a root partition, and an EFI system partition on this machine.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let btrfs_toggle = gtk::ToggleButton::builder()
        .label("btrfs")
        .active(state.manual_root_fs.get() == partition::RootFs::Btrfs)
        .build();
    let ext4_toggle = gtk::ToggleButton::builder()
        .label("ext4")
        .group(&btrfs_toggle)
        .active(state.manual_root_fs.get() == partition::RootFs::Ext4)
        .build();
    let fs_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    fs_row.append(
        &gtk::Label::builder()
            .label("Root filesystem")
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build(),
    );
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
    let swap_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    swap_row.append(
        &gtk::Label::builder()
            .label("Swap size (GiB)")
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build(),
    );
    swap_row.append(&swap_spin);
    {
        let state = state.clone();
        swap_spin.connect_value_changed(move |s| {
            *state.manual_swap_gib.borrow_mut() = Some(s.value() as u64);
        });
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    content.append(&heading);
    content.append(&body);
    content.append(&fs_row);
    content.append(&swap_row);

    let save_button = gtk::Button::builder()
        .label("Use these partitions")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    content.append(&save_button);
    {
        let nav = nav.clone();
        save_button.connect_clicked(move |_| {
            nav.pop();
        });
    }

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Partitions")
        .child(&clamp)
        .build();
    page.set_tag(Some("manual-partition"));
    page
}

/// GParted/elementary-style layout preview: ESP/swap/root as proportioned colored
/// segments, with size/label text moved into a legend list below the bar rather than
/// crammed inside the (often very narrow) segments themselves — real disks make ESP
/// (512MiB) and swap tiny relative to root, so text has to live outside the bar to stay
/// legible. Each segment still gets a minimum pixel width so it stays visible at all.
pub(super) fn partition_bar(
    disk_size_bytes: u64,
    swap_gib: u64,
    root_fs: partition::RootFs,
) -> gtk::Box {
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

    let legend = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    let legend_row = |css_class: &str, title: &str, size_bytes: u64, fs: &str| -> gtk::Box {
        let swatch = gtk::Box::builder()
            .width_request(14)
            .height_request(14)
            .valign(gtk::Align::Center)
            .css_classes(vec![css_class.to_string()])
            .build();
        let label = gtk::Label::builder()
            .label(format!(
                "{title} — {} ({fs})",
                disk::format_size(size_bytes)
            ))
            .halign(gtk::Align::Start)
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        row.append(&swatch);
        row.append(&label);
        row
    };
    legend.append(&legend_row(
        "gentoo-part-esp",
        "ESP",
        esp_mib * 1024 * 1024,
        "vfat",
    ));
    legend.append(&legend_row(
        "gentoo-part-swap",
        "swap",
        swap_mib * 1024 * 1024,
        "swap",
    ));
    legend.append(&legend_row(
        "gentoo-part-root",
        "root",
        root_mib * 1024 * 1024,
        root_fs_name,
    ));

    let wrapper = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    wrapper.append(&bar);
    wrapper.append(&legend);
    wrapper
}
