use super::*;

/// Reads the partition tables of `disks` (read-only, `parted print free`).
pub(super) fn inspect_disks(paths: Vec<String>) -> HashMap<String, disk::Inspection> {
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(async {
                let runner = installer_core::command::RealCommandRunner;
                let mut out = HashMap::new();
                for path in paths {
                    let inspection = disk::inspect(&runner, &path).await.unwrap_or_default();
                    out.insert(path, inspection);
                }
                out
            })
    })
    .join()
    .unwrap_or_default()
}

fn gib(mib: u64) -> u64 {
    mib / 1024
}

fn program_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// "How to install on this disk": erase it, or use its free space and keep what is there (other systems, files). Returns the
/// widget and the function to call whenever another disk is selected.
pub(super) fn placement_group(state: Rc<WizardState>) -> (gtk::Box, Rc<dyn Fn(&disk::Disk)>) {
    let group = adw::PreferencesGroup::builder()
        .title("How to install")
        .build();

    let erase = gtk::CheckButton::new();
    let free = gtk::CheckButton::new();
    free.set_group(Some(&erase));
    erase.set_active(true);

    let erase_row = adw::ActionRow::builder()
        .title("Erase the whole disk")
        .subtitle("Everything on it is lost. The simplest choice for an empty or spare disk.")
        .activatable_widget(&erase)
        .build();
    erase_row.add_prefix(&erase);
    let free_row = adw::ActionRow::builder()
        .title("Next to what is already there")
        .activatable_widget(&free)
        .build();
    free_row.add_prefix(&free);
    group.add(&erase_row);
    group.add(&free_row);

    let size_row = adw::SpinRow::builder()
        .title("Space for Simple Linux")
        .subtitle("GiB taken from the free space")
        .adjustment(&gtk::Adjustment::new(40.0, 40.0, 100.0, 1.0, 10.0, 0.0))
        .build();
    group.add(&size_row);

    let existing = adw::ExpanderRow::builder()
        .title("What is on this disk now")
        .build();
    group.add(&existing);

    let gparted = gtk::Button::builder()
        .label("Make room with GParted")
        .css_classes(vec!["flat".to_string()])
        .tooltip_text("Opens GParted: shrink or move a partition to free space, then press Rescan")
        .sensitive(program_exists("gparted"))
        .build();
    let rescan = gtk::Button::builder()
        .label("Rescan")
        .css_classes(vec!["flat".to_string()])
        .build();
    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&gparted);
    buttons.append(&rescan);

    let note = gtk::Label::builder()
        .css_classes(vec!["dim-label".to_string(), "caption".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let current: Rc<RefCell<Option<disk::Disk>>> = Rc::new(RefCell::new(None));

    // What `state` should say, from the radios and the size.
    let sync_state = {
        let state = state.clone();
        let free = free.clone();
        let size_row = size_row.clone();
        let note = note.clone();
        Rc::new(move || {
            if free.is_active() {
                state.free_space_gib.set(Some(size_row.value() as u64));
                note.set_label("Nothing outside the new partitions is changed. The other systems stay in the boot menu.");
            } else {
                state.free_space_gib.set(None);
                note.set_label("Everything on the disk you choose will be erased.");
            }
        })
    };
    {
        let sync = sync_state.clone();
        free.connect_toggled(move |_| sync());
    }
    {
        let sync = sync_state.clone();
        size_row.connect_value_notify(move |_| sync());
    }

    // Fills the rows for `d` from what was read of its partition table.
    let update: Rc<dyn Fn(&disk::Disk)> = {
        let state = state.clone();
        let erase = erase.clone();
        let free = free.clone();
        let free_row = free_row.clone();
        let size_row = size_row.clone();
        let existing = existing.clone();
        let sync = sync_state.clone();
        let current = current.clone();
        Rc::new(move |d: &disk::Disk| {
            *current.borrow_mut() = Some(d.clone());
            let inspections = state.inspections.borrow();
            let inspection = inspections.get(&d.path).cloned().unwrap_or_default();

            // The list of what is there.
            for row in std::mem::take(&mut *state.existing_rows.borrow_mut()) {
                existing.remove(&row);
            }
            let mut rows = Vec::new();
            if inspection.partitions.is_empty() {
                let r = adw::ActionRow::builder().title("No partitions").build();
                existing.add_row(&r);
                rows.push(r);
            }
            for p in &inspection.partitions {
                let what = if p.is_esp {
                    "EFI system partition".to_string()
                } else if p.name.is_empty() {
                    p.fs.clone()
                } else {
                    format!("{} · {}", p.name, p.fs)
                };
                let r = adw::ActionRow::builder()
                    .use_markup(false)
                    .title(&p.device)
                    .subtitle(format!(
                        "{} · {}",
                        disk::format_size((p.size_mib() * 1024.0 * 1024.0) as u64),
                        what
                    ))
                    .build();
                existing.add_row(&r);
                rows.push(r);
            }
            *state.existing_rows.borrow_mut() = rows;
            existing.set_visible(!inspection.partitions.is_empty());

            // Whether the free space is usable.
            let esp = inspection.esp().is_some();
            let need = partition::min_free_mib(esp);
            let largest = inspection.largest_free();
            let (possible, why) = if inspection.table != "gpt" {
                (
                    false,
                    if inspection.table.is_empty() || inspection.table == "unknown" {
                        "No partition table could be read (the disk is empty, or this is not run as root): use the whole disk.".to_string()
                    } else {
                        format!(
                            "This disk has a {} partition table; installing next to other systems needs GPT.",
                            inspection.table
                        )
                    },
                )
            } else {
                match largest {
                    Some(r) if r.size_mib() >= need => (true, String::new()),
                    Some(r) => (
                        false,
                        format!(
                            "Only {} GiB is free; at least {} GiB is needed. Make room with GParted.",
                            gib(r.size_mib()),
                            need.div_ceil(1024)
                        ),
                    ),
                    None => (
                        false,
                        format!(
                            "There is no free space; at least {} GiB is needed. Make room with GParted.",
                            need.div_ceil(1024)
                        ),
                    ),
                }
            };
            free_row.set_sensitive(possible);
            size_row.set_visible(possible);
            if possible {
                let region = largest.expect("possible implies a region");
                let (min, max) = (need.div_ceil(1024) as f64, gib(region.size_mib()) as f64);
                let adj = size_row.adjustment();
                adj.set_lower(min);
                adj.set_upper(max);
                adj.set_value(max);
                free_row.set_subtitle(&format!(
                    "{} GiB is free. {} stay{} as {} now.",
                    gib(region.size_mib()),
                    if inspection.partitions.is_empty() {
                        "Nothing else is on the disk and it"
                    } else {
                        "The other systems and files"
                    },
                    if inspection.partitions.is_empty() {
                        "s"
                    } else {
                        ""
                    },
                    if inspection.partitions.is_empty() {
                        "it is"
                    } else {
                        "they are"
                    },
                ));
            } else {
                free_row.set_subtitle(&why);
                erase.set_active(true);
            }
            let _ = &free;
            sync();
        })
    };

    {
        let state = state.clone();
        let update = update.clone();
        let current = current.clone();
        rescan.connect_clicked(move |_| {
            let Some(d) = current.borrow().clone() else {
                return;
            };
            let fresh = inspect_disks(vec![d.path.clone()]);
            state.inspections.borrow_mut().extend(fresh);
            update(&d);
        });
    }
    gparted.connect_clicked(move |_| {
        let _ = std::process::Command::new("gparted").spawn();
    });

    let wrapper = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(8)
        .build();
    wrapper.append(&group);
    wrapper.append(&buttons);
    wrapper.append(&note);
    (wrapper, update)
}
