use super::*;
use installer_core::event::{Event as InstallEvent, Level};
use installer_core::phase::{self, PhaseId};
use std::time::{Duration, Instant};

/// A seconds counter as `m:ss` or `h:mm:ss`.
fn clock(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// How long a phase is expected to feel like it takes, in seconds per unit of weight, for the bar's slow creep while a phase
/// reports nothing finer (the bar then never stands still, but never claims more than 90 % of the phase either).
const CREEP_SECS_PER_WEIGHT: f64 = 12.0;
/// The log keeps this many lines; older ones are dropped in blocks.
const LOG_LINES_MAX: i32 = 4000;

/// One row of the step list: an icon slot that is a dot, a spinner, a tick or a cross, the label and the time it took.
struct StepRow {
    id: PhaseId,
    stack: gtk::Stack,
    spinner: gtk::Spinner,
    time: gtk::Label,
    label: gtk::Label,
}

impl StepRow {
    fn set(&self, state: &str) {
        self.stack.set_visible_child_name(state);
        self.spinner.set_spinning(state == "active");
        match state {
            "active" => self.label.add_css_class("heading"),
            _ => self.label.remove_css_class("heading"),
        }
        match state {
            "pending" => self.label.add_css_class("dim-label"),
            _ => self.label.remove_css_class("dim-label"),
        }
    }
}

#[derive(Clone)]
pub(super) struct InstallingWidgets {
    pub(super) nav: adw::NavigationView,
    pub(super) page: adw::NavigationPage,
    pub(super) progress: gtk::ProgressBar,
    pub(super) status_label: gtk::Label,
    /// What is happening right now (a download's size, "package 3 of 31", the last line of output).
    activity: gtk::Label,
    steps: Rc<Vec<StepRow>>,
    steps_scroller: gtk::ScrolledWindow,
    log: gtk::TextBuffer,
    log_view: gtk::TextView,
    /// Shown after a failed install: continues it from where it stopped instead of starting over.
    pub(super) retry_button: gtk::Button,
    /// What the retry button re-runs; set by `spawn_install` on every start.
    pub(super) retry_opts: Rc<RefCell<Option<install::InstallOptions>>>,
}

impl InstallingWidgets {
    fn append_log(&self, line: &str) {
        let mut end = self.log.end_iter();
        if self.log.char_count() > 0 {
            self.log.insert(&mut end, "\n");
        }
        self.log.insert(&mut end, line);
        if self.log.line_count() > LOG_LINES_MAX {
            let mut start = self.log.start_iter();
            let mut cut = self.log.iter_at_line(LOG_LINES_MAX / 4).unwrap_or(start);
            self.log.delete(&mut start, &mut cut);
        }
        let end = self.log.end_iter();
        let mark = self.log.create_mark(None, &end, false);
        self.log_view.scroll_mark_onscreen(&mark);
        self.log.delete_mark(&mark);
    }

    fn row(&self, id: PhaseId) -> Option<&StepRow> {
        self.steps.iter().find(|r| r.id == id)
    }

    fn reset(&self) {
        for r in self.steps.iter() {
            r.set("pending");
            r.time.set_label("");
        }
        self.log.set_text("");
        self.progress.set_fraction(0.0);
    }
}

/// The page shown while the install runs. A determinate bar with a percentage and the elapsed time, the current
/// step in words with what it is doing right now, a list of every step with a spinner on the active one, and the log
/// of everything the install runs, open and following the newest line.
pub(super) fn installing_page_build(nav: adw::NavigationView) -> InstallingWidgets {
    let status_label = gtk::Label::builder()
        .label("Getting ready")
        .css_classes(vec!["title-2".to_string()])
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    let activity = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(vec!["dim-label".to_string()])
        .label("Starting")
        .build();

    let progress = gtk::ProgressBar::builder()
        .hexpand(true)
        .show_text(true)
        .text("0 %")
        .build();

    let step_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    let mut rows = Vec::new();
    for p in phase::all_phases() {
        let stack = gtk::Stack::builder().width_request(20).build();
        let pending = gtk::Image::from_icon_name("media-record-symbolic");
        pending.add_css_class("dim-label");
        let spinner = gtk::Spinner::new();
        let done = gtk::Image::from_icon_name("object-select-symbolic");
        done.add_css_class("success");
        let fail = gtk::Image::from_icon_name("dialog-error-symbolic");
        fail.add_css_class("error");
        stack.add_named(&pending, Some("pending"));
        stack.add_named(&spinner, Some("active"));
        stack.add_named(&done, Some("done"));
        stack.add_named(&fail, Some("failed"));
        let label = gtk::Label::builder()
            .label(p.label())
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        let time = gtk::Label::builder()
            .css_classes(vec!["dim-label".to_string(), "numeric".to_string()])
            .build();
        let row_box = gtk::Box::builder()
            .spacing(10)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(10)
            .margin_end(10)
            .build();
        row_box.append(&stack);
        row_box.append(&label);
        row_box.append(&time);
        step_list.append(
            &gtk::ListBoxRow::builder()
                .child(&row_box)
                .activatable(false)
                .build(),
        );
        let row = StepRow {
            id: p.id(),
            stack,
            spinner,
            time,
            label,
        };
        row.set("pending");
        rows.push(row);
    }
    let steps_scroller = gtk::ScrolledWindow::builder()
        .child(&step_list)
        .min_content_height(150)
        .max_content_height(190)
        .propagate_natural_height(true)
        .build();

    let log_view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(6)
        .bottom_margin(6)
        .build();
    let log = log_view.buffer();
    let log_scroller = gtk::ScrolledWindow::builder()
        .child(&log_view)
        .min_content_height(170)
        .vexpand(true)
        .build();
    log_scroller.add_css_class("card");
    let details = gtk::Expander::builder()
        .label("Details")
        .expanded(true)
        .child(&log_scroller)
        .vexpand(true)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(24)
        .margin_end(24)
        .margin_top(20)
        .margin_bottom(16)
        .build();
    let retry_button = gtk::Button::builder()
        .label("Resume the install")
        .css_classes(vec!["suggested-action".to_string(), "pill".to_string()])
        .halign(gtk::Align::Center)
        .visible(false)
        .build();
    content.append(&status_label);
    content.append(&activity);
    content.append(&progress);
    content.append(&steps_scroller);
    content.append(&details);
    content.append(&retry_button);

    let clamp = adw::Clamp::builder()
        .child(&content)
        .maximum_size(720)
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
        activity,
        steps: Rc::new(rows),
        steps_scroller,
        log,
        log_view,
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

/// What the page knows about the running install, advanced by the events and by the animation tick.
struct Run {
    started: Instant,
    finished: Vec<PhaseId>,
    current: Option<(PhaseId, Instant)>,
    /// How far into the current phase, when the install said (a download, an unpack, "3 of 31").
    sub: Option<f64>,
    /// The bar as drawn: it eases toward `target` so it glides instead of jumping at phase boundaries.
    shown: f64,
    last_clock: u64,
}

impl Run {
    fn target(&self) -> f64 {
        let total = f64::from(phase::total_weight().max(1));
        let base = phase::fraction_done(&self.finished);
        let Some((id, since)) = self.current else {
            return base;
        };
        let weight = phase::all_phases()
            .iter()
            .find(|p| p.id() == id)
            .map_or(1, |p| p.weight());
        let w = f64::from(weight);
        let within = match self.sub {
            Some(f) => f.clamp(0.0, 1.0),
            None => {
                0.9 * (1.0 - (-since.elapsed().as_secs_f64() / (w * CREEP_SECS_PER_WEIGHT)).exp())
            }
        };
        (base + w / total * within).min(0.999)
    }
}

/// Bridges `install::run`'s tokio-channel progress into the GTK main loop: the install
/// itself runs on a background thread with its own tokio runtime (GTK's own loop isn't
/// async), and progress is relayed through a `std::sync::mpsc` channel that a
/// `glib::timeout_add_local` on the main thread drains ~30 times a second, which is also what animates the bar.
pub(super) fn spawn_install(
    opts: install::InstallOptions,
    widgets: InstallingWidgets,
    mode: installer_core::phase::RunMode,
) {
    #[derive(Debug)]
    enum Msg {
        Event(InstallEvent),
        Finished(Result<(), String>),
    }

    *widgets.retry_opts.borrow_mut() = Some(opts.clone());
    let (tx, rx) = std_mpsc::channel::<Msg>();
    widgets.reset();
    widgets.status_label.set_label("Getting ready");
    widgets.activity.set_label("Starting");

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

    let mut run = Run {
        started: Instant::now(),
        finished: Vec::new(),
        current: None,
        sub: None,
        shown: 0.0,
        last_clock: u64::MAX,
    };
    glib::timeout_add_local(Duration::from_millis(33), move || {
        let mut keep_polling = true;
        let mut ended: Option<Result<(), String>> = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Event(InstallEvent::PhaseStarted { id, label }) => {
                    // A resumed run skips phases without announcing them: whatever is before this one is done.
                    for r in widgets.steps.iter() {
                        if r.id == id {
                            break;
                        }
                        if !run.finished.contains(&r.id) {
                            run.finished.push(r.id);
                            r.set("done");
                        }
                    }
                    if let Some(r) = widgets.row(id) {
                        r.set("active");
                        let n = widgets.steps.iter().position(|s| s.id == id).unwrap_or(0) + 1;
                        widgets
                            .status_label
                            .set_label(&format!("{label}  ({n} of {})", widgets.steps.len()));
                        // Keep the active row in view in the (short) list.
                        let adj = widgets.steps_scroller.vadjustment();
                        let per = adj.upper() / widgets.steps.len() as f64;
                        adj.set_value((per * (n as f64 - 3.0)).max(0.0));
                    }
                    run.current = Some((id, Instant::now()));
                    run.sub = None;
                    widgets.activity.set_label(&label);
                    widgets.append_log(&format!("== {label}"));
                }
                Msg::Event(InstallEvent::PhaseFinished { id, .. }) => {
                    if !run.finished.contains(&id) {
                        run.finished.push(id);
                    }
                    if let Some(r) = widgets.row(id) {
                        r.set("done");
                        if let Some((cur, since)) = run.current {
                            if cur == id {
                                r.time.set_label(&clock(since.elapsed().as_secs()));
                            }
                        }
                    }
                    if run.current.map(|c| c.0) == Some(id) {
                        run.current = None;
                        run.sub = None;
                    }
                }
                Msg::Event(InstallEvent::Step { text, fraction, .. }) => {
                    widgets.activity.set_label(&text);
                    if fraction.is_some() {
                        run.sub = fraction;
                    }
                }
                Msg::Event(InstallEvent::Log { line, level }) => {
                    if let Some((n, m, package)) = installer_core::progress::emerge_position(&line)
                    {
                        run.sub = Some((n.saturating_sub(1)) as f64 / m.max(1) as f64);
                        widgets.activity.set_label(&format!(
                            "Building and installing packages: {n} of {m}, {package}"
                        ));
                    } else if !line.starts_with('$') {
                        // The latest line of output is the proof that something is going on.
                        widgets.activity.set_label(line.trim());
                    }
                    widgets.append_log(&match level {
                        Level::Info => line,
                        Level::Warn => format!("warning: {line}"),
                        Level::Error => format!("error: {line}"),
                    });
                }
                Msg::Event(InstallEvent::Failed { id, error }) => {
                    if let Some(r) = widgets.row(id) {
                        r.set("failed");
                    }
                    widgets.append_log(&format!("ERROR in {id:?}: {error}"));
                }
                Msg::Event(InstallEvent::Progress { .. } | InstallEvent::Complete) => {}
                Msg::Finished(r) => ended = Some(r),
            }
        }

        match ended {
            Some(Ok(())) => {
                widgets.progress.set_fraction(1.0);
                widgets.progress.set_text(Some("100 %"));
                let done_page = done_page_build();
                widgets.nav.add(&done_page);
                widgets.nav.push(&done_page);
                keep_polling = false;
            }
            Some(Err(e)) => {
                widgets.status_label.set_label("Install failed");
                widgets.activity.set_label(&e);
                widgets.append_log(&format!("ERROR: {e}"));
                if let Some((id, _)) = run.current {
                    if let Some(r) = widgets.row(id) {
                        r.set("failed");
                    }
                }
                // A failure is not the end: what is done stays done, and the install can go on.
                widgets.retry_button.set_visible(!simulate_mode());
                keep_polling = false;
            }
            None => {
                // Ease toward the target so the bar glides; never move backwards.
                let target = run.target().max(run.shown);
                run.shown += (target - run.shown) * 0.12;
                widgets.progress.set_fraction(run.shown);
                let secs = run.started.elapsed().as_secs();
                if secs != run.last_clock {
                    run.last_clock = secs;
                    widgets.progress.set_text(Some(&format!(
                        "{:.0} %   ·   {}",
                        run.shown * 100.0,
                        clock(secs)
                    )));
                }
            }
        }
        if keep_polling {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

/// `installer-gui --debug-progress`: only the installing page, running the simulated install, to look at the progress
/// display without clicking through the wizard.
pub fn build_debug_progress(app: &adw::Application) {
    install_css();
    let nav = adw::NavigationView::new();
    let widgets = installing_page_build(nav.clone());
    nav.add(&widgets.page);
    let win = adw::ApplicationWindow::builder()
        .application(app)
        .title("Gentoo Installer (progress preview)")
        .default_width(800)
        .default_height(640)
        .content(&nav)
        .build();
    win.present();
    let opts = install::InstallOptions {
        secure_boot: false,
        layout: partition::plan("/dev/vda", partition::RootFs::Btrfs, 16 << 30),
        target: "/mnt/gentoo".into(),
        store: store::StoreConfig {
            binhost_url: String::new(),
            overlay_git_url: String::new(),
            overlay_name: String::new(),
        },
        kernel_base_name: "k".into(),
        keyboard_layout: "us".into(),
        timezone: "UTC".into(),
        hostname: "gentoo".into(),
        locales: vec!["en_US.UTF-8".into()],
        gpu_override: None,
        render: Default::default(),
        packages: Vec::new(),
        stage3: None,
        account: Account {
            username: "u".into(),
            password: String::new(),
        },
        wm: WmChoice::default(),
        opt_level: OptLevel::default(),
        package_mode: PackageMode::default(),
        wm_configs_git_url: String::new(),
        simulate: true,
    };
    spawn_install(opts, widgets, installer_core::phase::RunMode::Fresh);
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
