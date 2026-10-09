use super::*;

#[derive(Clone)]
pub(super) struct InstallingWidgets {
    pub(super) nav: adw::NavigationView,
    pub(super) page: adw::NavigationPage,
    pub(super) progress: gtk::ProgressBar,
    pub(super) status_label: gtk::Label,
    pub(super) log_label: gtk::Label,
    /// Shown after a failed install: continues it from where it stopped instead of starting over.
    pub(super) retry_button: gtk::Button,
    /// What the retry button re-runs; set by `spawn_install` on every start.
    pub(super) retry_opts: Rc<RefCell<Option<install::InstallOptions>>>,
}

/// Ported from elementary installer's `ProgressView`: a determinate progress bar with a
/// short status line by default, and the full step-by-step log tucked behind a
/// collapsed `Expander` — most users never need to open it, but it's there for anyone
/// debugging a failed install.
pub(super) fn installing_page_build(nav: adw::NavigationView) -> InstallingWidgets {
    let status_label = gtk::Label::builder()
        .label("Getting ready")
        .css_classes(vec!["title-2".to_string()])
        .wrap(true)
        .build();

    let progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .show_text(false)
        .build();

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
    let details = gtk::Expander::builder()
        .label("Show details")
        .child(&log_scroller)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_start(24)
        .margin_end(24)
        .margin_top(48)
        .margin_bottom(24)
        .build();
    let retry_button = gtk::Button::builder()
        .label("Resume the install")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .visible(false)
        .build();
    content.append(&status_label);
    content.append(&progress);
    content.append(&details);
    content.append(&retry_button);

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(600)
        .build();

    let page = adw::NavigationPage::builder()
        .title("Installing")
        .child(&clamp)
        .build();
    page.set_tag(Some("installing"));

    let widgets = InstallingWidgets {
        nav,
        page,
        progress,
        status_label,
        log_label,
        retry_button: retry_button.clone(),
        retry_opts: Rc::new(RefCell::new(None)),
    };
    {
        let widgets = widgets.clone();
        retry_button.connect_clicked(move |b| {
            b.set_visible(false);
            let opts = widgets.retry_opts.borrow().clone();
            if let Some(opts) = opts {
                spawn_install(
                    opts,
                    widgets.clone(),
                    installer_core::phase::RunMode::Resume,
                );
            }
        });
    }
    widgets
}

/// Bridges `install::run`'s tokio-channel progress into the GTK main loop: the install
/// itself runs on a background thread with its own tokio runtime (GTK's own loop isn't
/// async), and progress is relayed through a `std::sync::mpsc` channel that a
/// `glib::timeout_add_local` on the main thread drains every 200ms — the same
/// non-blocking-poll approach the CLI uses per ratatui frame.
pub(super) fn spawn_install(
    opts: install::InstallOptions,
    widgets: InstallingWidgets,
    mode: installer_core::phase::RunMode,
) {
    use installer_core::event::{Event as InstallEvent, Level};
    use installer_core::phase::{self, PhaseId};

    #[derive(Debug)]
    enum Msg {
        Event(InstallEvent),
        Finished(Result<(), String>),
    }

    *widgets.retry_opts.borrow_mut() = Some(opts.clone());
    let (tx, rx) = std_mpsc::channel::<Msg>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
            let handle = tokio::spawn(install::run(opts, events_tx, mode));
            while let Some(e) = events_rx.recv().await {
                if tx.send(Msg::Event(e)).is_err() {
                    return;
                }
            }
            let result = match handle.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(format!("install task panicked: {e}")),
            };
            let _ = tx.send(Msg::Finished(result));
        });
    });

    let mut finished: Vec<PhaseId> = Vec::new();
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let mut log = widgets.log_label.label().to_string();
        let mut push = |line: &str| {
            if !log.is_empty() {
                log.push('\n');
            }
            log.push_str(line);
        };
        let mut keep_polling = true;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Event(InstallEvent::PhaseStarted { label, .. }) => {
                    widgets.status_label.set_label(&label);
                    push(&label);
                }
                Msg::Event(InstallEvent::PhaseFinished { id, .. }) => {
                    finished.push(id);
                    widgets
                        .progress
                        .set_fraction(phase::fraction_done(&finished));
                }
                Msg::Event(InstallEvent::Log { line, level }) => {
                    push(&match level {
                        Level::Info => format!("  {line}"),
                        Level::Warn => format!("  warning: {line}"),
                        Level::Error => format!("  error: {line}"),
                    });
                }
                Msg::Event(
                    InstallEvent::Failed { .. }
                    | InstallEvent::Progress { .. }
                    | InstallEvent::Complete,
                ) => {}
                Msg::Finished(Ok(())) => {
                    widgets.progress.set_fraction(1.0);
                    let done_page = done_page_build();
                    widgets.nav.add(&done_page);
                    widgets.nav.push(&done_page);
                    keep_polling = false;
                }
                Msg::Finished(Err(e)) => {
                    widgets
                        .status_label
                        .set_label(&format!("Install failed: {e}"));
                    push(&format!("ERROR: {e}"));
                    // A failure is not the end: what is done stays done, and the install can go on.
                    widgets.retry_button.set_visible(!simulate_mode());
                    keep_polling = false;
                }
            }
        }
        widgets.log_label.set_label(&log);
        if keep_polling {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

/// Screen script §12. `Restart` is a real `reboot` (OpenRC/sysvinit; this system has no systemd)
/// outside simulate mode —
/// guarded so clicking it during UI iteration never reboots the development machine.
pub(super) fn done_page_build() -> adw::NavigationPage {
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
        let _ = std::process::Command::new("reboot").spawn();
    });

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .halign(gtk::Align::Center)
        .build();
    content.append(&note);
    content.append(&restart_button);
    status.set_child(Some(&content));

    let page = adw::NavigationPage::builder()
        .title("Done")
        .child(&status)
        .build();
    page.set_tag(Some("done"));
    page
}
