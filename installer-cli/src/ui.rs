//! Ratatui render loop. Network/disk steps pull real data from installer-core (iwd scan,
//! lsblk, hardware detection); Confirm kicks off `installer_core::install::run` in the
//! background and Installing streams its progress until it finishes or errors.

use crate::steps::Step;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use installer_core::{
    config::StoreEnv, disk, hardware, install,
    make_conf::{OptLevel, PackageMode},
    network, partition, store,
    wm::WmChoice,
};
use ratatui::{
    layout::{Constraint, Layout as RtLayout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    DefaultTerminal,
};
use tokio::sync::mpsc;

pub struct AppState {
    pub step: Step,
    pub ethernet_up: bool,
    pub disks: Vec<disk::Disk>,
    pub profile: Option<hardware::Profile>,
    pub selected_disk: usize,
    pub status: String,
    pub install_log: Vec<String>,
    pub install_rx: Option<mpsc::UnboundedReceiver<install::Progress>>,
    pub install_task: Option<tokio::task::JoinHandle<installer_core::Result<()>>>,
    pub install_finished: bool,
}

impl AppState {
    fn new() -> Self {
        Self {
            step: Step::Network,
            ethernet_up: network::IwdClient::ethernet_link_up(),
            disks: Vec::new(),
            profile: None,
            selected_disk: 0,
            status: String::new(),
            install_log: Vec::new(),
            install_rx: None,
            install_task: None,
            install_finished: false,
        }
    }
}

pub async fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut state = AppState::new();

    loop {
        drain_install_progress(&mut state).await;

        terminal.draw(|frame| draw(frame, &state))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Down => {
                        if state.selected_disk + 1 < state.disks.len() {
                            state.selected_disk += 1;
                        }
                    }
                    KeyCode::Up => {
                        state.selected_disk = state.selected_disk.saturating_sub(1);
                    }
                    KeyCode::Enter => advance(&mut state).await,
                    _ => {}
                }
            }
        }

        if state.step == Step::Done {
            return Ok(());
        }
    }
}

/// Non-blocking drain of whatever progress events have arrived since the last frame —
/// the install runs on its own tokio task, this just reflects it into the log.
async fn drain_install_progress(state: &mut AppState) {
    if let Some(rx) = &mut state.install_rx {
        while let Ok(progress) = rx.try_recv() {
            state.install_log.push(describe(&progress));
            if matches!(progress, install::Progress::Done) {
                state.install_finished = true;
            }
        }
    }

    if state.install_finished {
        if let Some(task) = state.install_task.take() {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => state.install_log.push(format!("ERROR: {e}")),
                Err(e) => state.install_log.push(format!("ERROR: install task panicked: {e}")),
            }
            state.step = Step::Done;
        }
    }
}

fn describe(p: &install::Progress) -> String {
    match p {
        install::Progress::Partitioning => "Partitioning disk...".into(),
        install::Progress::DownloadingStage3 => "Downloading stage3...".into(),
        install::Progress::UnpackingStage3 => "Unpacking stage3...".into(),
        install::Progress::WritingMakeConf => "Tuning make.conf for your hardware...".into(),
        install::Progress::ConfiguringStore => "Configuring portage store...".into(),
        install::Progress::InstallingKernel { atom, degraded_by } => {
            if *degraded_by == 0 {
                format!("Installing kernel: {atom} (exact hardware match)")
            } else {
                format!("Installing kernel: {atom} (generalized, degraded {degraded_by} step(s))")
            }
        }
        install::Progress::InstallingGpuDriver => "Building the Nvidia driver for your kernel...".into(),
        install::Progress::WritingFstab => "Writing fstab...".into(),
        install::Progress::SettingKeyboard => "Setting keyboard layout...".into(),
        install::Progress::SettingTimezone => "Setting time zone...".into(),
        install::Progress::CreatingAccount => "Creating your account...".into(),
        install::Progress::InstallingDesktop => "Installing desktop environment...".into(),
        install::Progress::InstallingBootloader => "Installing Limine...".into(),
        install::Progress::Done => "Install complete.".into(),
    }
}

async fn advance(state: &mut AppState) {
    match state.step {
        Step::Network => {
            state.step = Step::DiskSelect;
            state.status = "detecting hardware + listing disks...".into();
            state.profile = hardware::Profile::detect().ok();
            match disk::list(&installer_core::command::RealCommandRunner).await {
                Ok(disks) => {
                    state.disks = disks;
                    state.status.clear();
                }
                Err(e) => state.status = format!("disk listing failed: {e}"),
            }
        }
        Step::DiskSelect => {
            if state.disks.get(state.selected_disk).is_some() {
                state.step = Step::Confirm;
            }
        }
        Step::Confirm => start_install(state),
        Step::Installing | Step::Done => {}
    }
}

/// Set to click through the wizard without touching a real disk — see
/// `installer_core::install::InstallOptions::simulate`.
fn simulate_mode() -> bool {
    std::env::var("GENTOO_INSTALLER_SIMULATE").is_ok()
}

fn start_install(state: &mut AppState) {
    let Some(disk) = state.disks.get(state.selected_disk).cloned() else {
        return;
    };
    let Some(profile) = &state.profile else {
        state.status = "no hardware profile detected, cannot pick a kernel".into();
        return;
    };
    let simulate = simulate_mode();
    let store_env = if simulate {
        None
    } else {
        match StoreEnv::from_env() {
            Ok(env) => Some(env),
            Err(e) => {
                state.status = format!("store not configured: {e}");
                return;
            }
        }
    };

    // No account-creation screen in the minimal ISO's TUI yet — same "don't hardcode a
    // real destructive default" reasoning as GENTOO_STORE_BINHOST_URL. Simulate mode
    // doesn't need real credentials at all.
    let account = if simulate {
        installer_core::account::Account { username: "gentoo".into(), password: String::new() }
    } else {
        let username = std::env::var("GENTOO_INSTALLER_USERNAME");
        let password = std::env::var("GENTOO_INSTALLER_PASSWORD");
        match (username, password) {
            (Ok(username), Ok(password)) => installer_core::account::Account { username, password },
            _ => {
                state.status = "set GENTOO_INSTALLER_USERNAME / GENTOO_INSTALLER_PASSWORD (no account screen in this TUI yet)".into();
                return;
            }
        }
    };

    // No Advanced-setup picker in this TUI yet (same gap already true for account
    // creation, immediately above) — mirrors the GENTOO_INSTALLER_USERNAME/_PASSWORD
    // env-var pattern instead of a screen. Unset/unrecognized defaults to niri, the
    // same silent-default shape the GUI uses when Advanced setup is off.
    let wm = match std::env::var("GENTOO_INSTALLER_WM").as_deref() {
        Ok("hyprland") => WmChoice::Hyprland,
        Ok("sway") => WmChoice::Sway,
        Ok("labwc") => WmChoice::Labwc,
        Ok("mangowc") => WmChoice::MangoWc,
        _ => WmChoice::default(),
    };
    let wm_configs_git_url = std::env::var("GENTOO_INSTALLER_WM_CONFIGS_URL").unwrap_or_default();
    let opt_level = match std::env::var("GENTOO_INSTALLER_OPT_LEVEL").as_deref() {
        Ok("O3") | Ok("o3") => OptLevel::O3,
        _ => OptLevel::default(),
    };
    let package_mode = match std::env::var("GENTOO_INSTALLER_PACKAGE_MODE").as_deref() {
        Ok("source") => PackageMode::Source,
        _ => PackageMode::default(),
    };

    let hostname = std::env::var("GENTOO_INSTALLER_HOSTNAME").unwrap_or_else(|_| "gentoo".into());
    let locales: Vec<String> = match std::env::var("GENTOO_INSTALLER_LOCALES") {
        Ok(v) if !v.trim().is_empty() => v.split(',').map(|l| l.trim().to_string()).collect(),
        _ => vec![installer_core::locale::DEFAULT_LOCALE.to_string()],
    };

    let gpu_override = match std::env::var("GENTOO_INSTALLER_GPU").as_deref() {
        Ok("nvidia") => Some(installer_core::hardware::Gpu::Nvidia),
        Ok("nouveau") => Some(installer_core::hardware::Gpu::Nouveau),
        Ok("amd") => Some(installer_core::hardware::Gpu::Amd),
        Ok("intel") => Some(installer_core::hardware::Gpu::Intel),
        Ok("xe") => Some(installer_core::hardware::Gpu::Xe),
        Ok("none") => Some(installer_core::hardware::Gpu::None),
        _ => None,
    };
    let packages: Vec<String> = match std::env::var("GENTOO_INSTALLER_PACKAGES") {
        Ok(v) => v.split(',').map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect(),
        Err(_) => installer_core::packages::default_ids(),
    };

    let layout = partition::plan(&disk.path, partition::RootFs::Btrfs, profile.ram_bytes);
    let opts = install::InstallOptions {
        layout,
        target: "/mnt/gentoo".into(),
        store: store::StoreConfig {
            binhost_url: store_env.as_ref().map(|e| e.binhost_url.clone()).unwrap_or_default(),
            overlay_git_url: store_env.as_ref().map(|e| e.overlay_git_url.clone()).unwrap_or_default(),
            overlay_name: store_env.as_ref().map(|e| e.overlay_name.clone()).unwrap_or_default(),
        },
        kernel_base_name: store_env
            .map(|e| e.kernel_base_name)
            .unwrap_or_else(|| "gentoo-diy-kernel".into()),
        keyboard_layout: installer_core::keyboard::detect_current(),
        timezone: installer_core::timezone::detect_current().unwrap_or_else(|| "UTC".into()),
        hostname,
        locales,
        gpu_override,
        packages,
        account,
        wm,
        wm_configs_git_url,
        opt_level,
        package_mode,
        simulate,
    };

    let (tx, rx) = mpsc::unbounded_channel();
    state.install_rx = Some(rx);
    state.install_task = Some(tokio::spawn(install::run(opts, tx)));
    state.install_log.clear();
    state.step = Step::Installing;
}

fn draw(frame: &mut ratatui::Frame, state: &AppState) {
    let area = frame.area();
    let chunks = RtLayout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);

    let title = Paragraph::new(Line::from("Gentoo Installer (TUI) — minimal ISO"))
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    match state.step {
        Step::Network => draw_network(frame, chunks[1], state),
        Step::DiskSelect => draw_disk_select(frame, chunks[1], state),
        Step::Confirm => draw_confirm(frame, chunks[1], state),
        Step::Installing | Step::Done => draw_installing(frame, chunks[1], state),
    }
}

fn draw_network(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    let text = if state.ethernet_up {
        "Ethernet link detected — network already up.\n\n[Enter] continue  [q] quit".to_string()
    } else {
        "No ethernet link. Wifi setup (iwd) not wired into this screen yet.\n\n[Enter] continue anyway  [q] quit".to_string()
    };
    let body = Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Network"));
    frame.render_widget(body, area);
}

fn draw_disk_select(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    let chunks = RtLayout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(area);

    let profile_text = match &state.profile {
        Some(p) => format!(
            "RAM: {} GiB  Kernel profile: {}",
            p.ram_bytes / 1024 / 1024 / 1024,
            p.combo()
        ),
        None => "hardware detection unavailable".to_string(),
    };
    frame.render_widget(
        Paragraph::new(profile_text).block(Block::default().borders(Borders::ALL).title("Detected hardware")),
        chunks[0],
    );

    let items: Vec<ListItem> = if state.disks.is_empty() {
        vec![ListItem::new(if state.status.is_empty() {
            "no disks found".to_string()
        } else {
            state.status.clone()
        })]
    } else {
        state
            .disks
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let marker = if i == state.selected_disk { ">" } else { " " };
                ListItem::new(format!(
                    "{marker} {} — {} {}",
                    d.path,
                    disk::format_size(d.size_bytes),
                    d.model
                ))
            })
            .collect()
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Select disk [Up/Down, Enter to confirm]"),
    );
    frame.render_widget(list, chunks[1]);
}

fn draw_confirm(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    let disk = state.disks.get(state.selected_disk);
    let text = match disk {
        Some(d) if simulate_mode() => format!(
            "SIMULATION MODE — no real changes will be made to {} ({}).\n\n[Enter] run simulated install  [q] abort\n\n{}",
            d.path,
            disk::format_size(d.size_bytes),
            state.status,
        ),
        Some(d) => format!(
            "About to WIPE {} ({}) and install: ESP 512MiB, swap, btrfs root with @/@home/@var/@log subvolumes, Limine bootloader.\n\n[Enter] confirm and install  [q] abort\n\n{}",
            d.path,
            disk::format_size(d.size_bytes),
            state.status,
        ),
        None => "No disk selected.".to_string(),
    };
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Confirm")),
        area,
    );
}

fn draw_installing(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    let log = state.install_log.join("\n");
    let title = if state.step == Step::Done { "Done [q] quit" } else { "Installing" };
    frame.render_widget(
        Paragraph::new(log).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}
