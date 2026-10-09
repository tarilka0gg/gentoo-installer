use super::*;

pub(super) enum NetCmd {
    Scan,
    Connect {
        path: String,
        passphrase: Option<String>,
    },
}

pub(super) enum NetEvt {
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
pub(super) fn spawn_network_worker() -> (
    tokio::sync::mpsc::UnboundedSender<NetCmd>,
    std_mpsc::Receiver<NetEvt>,
) {
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
                    NetCmd::Connect { path, passphrase } => {
                        match client.connect_to(&path, passphrase.as_deref()).await {
                            Ok(()) => {
                                let _ = evt_tx.send(NetEvt::Connected);
                            }
                            Err(e) => {
                                let _ = evt_tx.send(NetEvt::ConnectFailed(e.to_string()));
                            }
                        }
                    }
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
pub(super) fn network_page_build(
    nav: adw::NavigationView,
    disk_page: adw::NavigationPage,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label("Connect to a network")
        .css_classes(vec!["hero-title".to_string()])
        .halign(gtk::Align::Start)
        .build();
    let body = gtk::Label::builder()
        .label("Gentoo installs everything it needs from this device, so a connection is optional. It's used to check for updates on first start.")
        .css_classes(vec!["dim-label".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();

    let status_label = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .css_classes(vec!["dim-label".to_string()])
        .visible(false)
        .build();
    let networks_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .build();

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
    content.append(&status_label);
    content.append(&networks_box);

    let continue_button = gtk::Button::builder()
        .label("Continue")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::End)
        .margin_top(24)
        .build();
    let skip_button = gtk::Button::builder()
        .label("Continue without a network")
        .css_classes(vec!["flat".to_string()])
        .halign(gtk::Align::End)
        .build();
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::End)
        .margin_top(12)
        .build();
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

    let (cmd_tx, evt_rx): (
        Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<NetCmd>>>>,
        _,
    ) = {
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

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(560)
        .build();
    let page = adw::NavigationPage::builder()
        .title("Network")
        .child(&clamp)
        .build();
    page.set_tag(Some("network"));
    page
}

/// One scanned network: SSID + signal + lock icon, with a `Connect` button that reveals
/// an inline password field for secured networks (per screen script §4 — "an inline
/// password field ... within the row") rather than a separate dialog.
pub(super) fn network_row(
    net: network::Network,
    cmd_tx: Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<NetCmd>>>>,
    status_label: gtk::Label,
) -> gtk::Box {
    let icon = gtk::Image::from_icon_name(if net.secured {
        "network-wireless-encrypted-symbolic"
    } else {
        "network-wireless-symbolic"
    });
    let ssid_label = gtk::Label::builder()
        .label(&net.ssid)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    let connect_button = gtk::Button::builder()
        .label("Connect")
        .css_classes(vec!["flat".to_string()])
        .build();

    let header_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    header_row.append(&icon);
    header_row.append(&ssid_label);
    header_row.append(&connect_button);

    let password_entry = gtk::PasswordEntry::builder()
        .placeholder_text("Password")
        .show_peek_icon(true)
        .visible(false)
        .build();

    let wrapper = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
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
        let passphrase = if secured {
            Some(password_entry.text().to_string())
        } else {
            None
        };
        status_label.set_label("Connecting…");
        status_label.set_visible(true);
        if let Some(tx) = cmd_tx.borrow().as_ref() {
            let _ = tx.send(NetCmd::Connect {
                path: path.clone(),
                passphrase,
            });
        }
    });

    wrapper
}
