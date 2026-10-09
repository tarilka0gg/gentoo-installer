use super::*;

/// Screen script §10: "styled deliberately plainer and denser than every other page —
/// no tiles, no large heading, tighter leading. Reading it is the point." Plain sentence
/// lines instead of a StatusPage; only lines backed by something real are shown.
pub(super) fn confirm_page_build(
    nav: adw::NavigationView,
    state: Rc<WizardState>,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Review before installing")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();

    let simulate = simulate_mode();
    let disk = state.selected_disk.borrow().clone();
    let sentences = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .margin_top(20)
        .build();

    let sentence = |text: &str, destructive: bool| {
        let label = gtk::Label::builder()
            .label(text)
            .halign(gtk::Align::Start)
            .wrap(true)
            .build();
        if destructive {
            label.add_css_class("error");
        }
        label
    };

    if simulate {
        if let Some(d) = &disk {
            sentences.append(&sentence(
                &format!(
                    "SIMULATION MODE — no real changes will be made to {} ({}).",
                    d.path,
                    disk::format_size(d.size_bytes)
                ),
                false,
            ));
        }
    } else if let Some(d) = &disk {
        sentences.append(&sentence(
            &format!(
                "Install Gentoo on {} ({}).",
                d.path,
                disk::format_size(d.size_bytes)
            ),
            false,
        ));

        let erase_line = match state.existing_os.borrow().as_ref() {
            Some(os) => format!("Erase everything on this disk, including {os}."),
            None => "Erase everything on this disk.".to_string(),
        };
        sentences.append(&sentence(&erase_line, true));

        let username = state.username.borrow().clone();
        if !username.is_empty() {
            sentences.append(&sentence(
                &format!("Create an account for {username}."),
                false,
            ));
        }
        sentences.append(&sentence(
            &format!("Set the time zone to {}.", state.timezone.borrow()),
            false,
        ));
        sentences.append(&sentence(
            &format!("Install {} as your desktop.", state.wm.get().display_name()),
            false,
        ));
        if state.opt_level.get() == OptLevel::O3 {
            sentences.append(&sentence("Build with -O3 instead of -O2.", false));
        }
        if state.package_mode.get() == PackageMode::Source {
            sentences.append(&sentence(
                "Build everything from source instead of using prebuilt binary packages.",
                false,
            ));
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
        content.append(&partition_bar(
            d.size_bytes,
            state.swap_gib(),
            state.manual_root_fs.get(),
        ));
    }

    let error_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .wrap(true)
        .visible(false)
        .css_classes(vec!["error".to_string()])
        .build();
    content.append(&error_label);

    let back_secondary = gtk::Button::builder()
        .label("Back")
        .css_classes(vec!["flat".to_string()])
        .build();
    let install_button = gtk::Button::builder()
        .label(if simulate {
            "Simulate install"
        } else {
            "Erase and install"
        })
        .css_classes(vec![
            (if simulate {
                "suggested-action"
            } else {
                "destructive-action"
            })
            .to_string(),
            "pill".to_string(),
        ])
        .build();
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    footer.append(&back_secondary);
    footer.append(&install_button);
    content.append(&footer);

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Confirm")
        .child(&clamp)
        .build();
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
            let Some(disk) = state.selected_disk.borrow().clone() else {
                return;
            };
            btn.set_sensitive(false);

            let layout =
                partition::plan_with_swap(&disk.path, state.manual_root_fs.get(), state.swap_gib());
            let account = if simulate {
                Account {
                    username: "gentoo".into(),
                    password: String::new(),
                }
            } else {
                Account {
                    username: state.username.borrow().clone(),
                    password: state.password.borrow().clone(),
                }
            };
            let opts = install::InstallOptions {
                layout,
                target: "/mnt/gentoo".into(),
                store: store::StoreConfig {
                    binhost_url: store_env
                        .as_ref()
                        .map(|e| e.binhost_url.clone())
                        .unwrap_or_default(),
                    overlay_git_url: store_env
                        .as_ref()
                        .map(|e| e.overlay_git_url.clone())
                        .unwrap_or_default(),
                    overlay_name: store_env
                        .as_ref()
                        .map(|e| e.overlay_name.clone())
                        .unwrap_or_default(),
                },
                wm_configs_git_url: store_env
                    .as_ref()
                    .map(|e| e.wm_configs_git_url.clone())
                    .unwrap_or_default(),
                kernel_base_name: store_env
                    .map(|e| e.kernel_base_name)
                    .unwrap_or_else(|| "gentoo-diy-kernel".into()),
                keyboard_layout: state.keyboard_layout.borrow().clone(),
                timezone: state.timezone.borrow().clone(),
                hostname: "gentoo".into(),
                locales: installer_core::autodetect::guess_for_layout(
                    &state.keyboard_layout.borrow(),
                )
                .locales,
                account,
                gpu_override: state.gpu.get(),
                secure_boot: state.secure_boot.get(),
                render: Default::default(),
                packages: state.packages.borrow().clone(),
                stage3: std::env::var("GENTOO_INSTALLER_STAGE3_URL")
                    .ok()
                    .filter(|u| !u.trim().is_empty())
                    .map(|u| {
                        installer_core::stage3::Stage3Source::custom(
                            u.trim(),
                            std::env::var("GENTOO_INSTALLER_STAGE3_SHA512").ok(),
                        )
                    }),
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
            spawn_install(opts, widgets, installer_core::phase::RunMode::Fresh);
        });
    }

    page
}
