//! Ratatui render loop. Network/disk steps pull real data from installer-core (iwd scan,
//! lsblk, hardware detection); Confirm kicks off `installer_core::install::run` in the
//! background and Installing streams its progress until it finishes or errors.

use crate::account_form::{AccountForm, Outcome as AccountOutcome};
use crate::steps::Step;
use crate::wifi::{Action, Mode as WifiMode, WifiState};
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
    /// Wi-Fi screen (only used while there is no Ethernet link).
    pub wifi: WifiState,
    pub iwd: Option<network::IwdClient>,
    /// The next `Action` for the loop to perform after the frame showing "Scanning…" is drawn.
    pub wifi_action: Option<Action>,
    /// Set when the user chose to reboot after a finished install; `run` then returns.
    pub exit: bool,
    /// The "create your user" screen, and its result once submitted.
    pub account_form: AccountForm,
    pub account: Option<installer_core::account::Account>,
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
            wifi: WifiState::new(),
            iwd: None,
            wifi_action: None,
            exit: false,
            account_form: AccountForm::new(),
            account: None,
        }
    }
}

pub async fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut state = AppState::new();
    if !state.ethernet_up {
        state.wifi.begin("Scanning…");
        state.wifi_action = Some(Action::Scan);
    }

    loop {
        drain_install_progress(&mut state).await;

        terminal.draw(|frame| draw(frame, &state))?;
        perform_wifi(&mut state).await;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // The Wi-Fi screen owns the keyboard (passphrase entry needs `q` and Esc).
                if state.step == Step::Network && !state.ethernet_up {
                    match state.wifi.handle_key(key.code) {
                        Some(Action::Quit) => return Ok(()),
                        Some(Action::Continue) => advance(&mut state).await,
                        Some(a) => state.wifi_action = Some(a),
                        None => {}
                    }
                    continue;
                }
                if state.step == Step::Account {
                    match state.account_form.handle_key(key.code) {
                        AccountOutcome::Submit(a) => {
                            state.account = Some(a);
                            state.step = Step::Confirm;
                        }
                        AccountOutcome::Back => state.step = Step::DiskSelect,
                        AccountOutcome::None => {}
                    }
                    continue;
                }
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

        if state.exit {
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

    // A failed install never sends `Progress::Done`, so waiting for it alone left the screen frozen on the
    // last step with no error (found in a real VM run). The task ending is the signal; its result says how.
    let ended = state.install_task.as_ref().is_some_and(|t| t.is_finished());
    if state.install_finished || ended {
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

/// Runs the Wi-Fi action the key handler queued. Called right after a frame was drawn, so the
/// screen already says "Scanning…"/"Connecting…" while `iwd` works.
async fn perform_wifi(state: &mut AppState) {
    let Some(action) = state.wifi_action.take() else { return };
    if state.iwd.is_none() {
        match network::IwdClient::connect().await {
            Ok(c) => state.iwd = Some(c),
            Err(e) => {
                state.wifi.scan_failed(&format!("iwd is not reachable ({e})"));
                return;
            }
        }
    }
    let Some(iwd) = state.iwd.as_ref() else { return };
    match action {
        Action::Scan => {
            let result = async {
                iwd.request_scan().await?;
                // iwd's Scan() returns as soon as scanning starts; give it time to fill in.
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                iwd.scan().await
            }
            .await;
            match result {
                Ok(nets) => state.wifi.set_networks(nets),
                Err(e) => state.wifi.scan_failed(&e.to_string()),
            }
        }
        Action::Connect { path, passphrase } => match iwd.connect_to(&path, passphrase.as_deref()).await {
            Ok(()) => {
                state.ethernet_up = true; // "the network is up" — the screen then just says so
                state.status = "Connected.".into();
                advance(state).await;
            }
            Err(e) => state.wifi.connect_failed(&e.to_string()),
        },
        Action::Continue | Action::Quit => {}
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
                let from_env = std::env::var("GENTOO_INSTALLER_USERNAME").is_ok() && std::env::var("GENTOO_INSTALLER_PASSWORD").is_ok();
                state.step = if from_env || simulate_mode() { Step::Confirm } else { Step::Account };
            }
        }
        // The form submits itself (see the key loop); Enter here is never reached for it.
        Step::Account => {}
        Step::Confirm => start_install(state),
        Step::Installing => {}
        // After a finished install: Enter reboots (OpenRC's `reboot`; the disk was unmounted by
        // the installer). Not offered if the log has an error — rebooting into a half-installed
        // system is not what anyone wants from a stray Enter.
        Step::Done => {
            if install_failed(&state.install_log) {
                state.status = "The install failed - see the log above. [q] quit".into();
            } else {
                let _ = std::process::Command::new("reboot").spawn();
                state.exit = true;
            }
        }
    }
}

fn install_failed(log: &[String]) -> bool {
    log.iter().any(|l| l.starts_with("ERROR"))
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
    } else if let Some(a) = state.account.clone() {
        a
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
        Ok("dwl") => WmChoice::Dwl,
        _ => WmChoice::default(),
    };
    // One name for this everywhere: the store configuration's `GENTOO_WM_CONFIGS_URL` (the TUI used to read a
    // different variable, `GENTOO_INSTALLER_WM_CONFIGS_URL`, so a correctly configured store still gave an
    // empty URL here and the desktop step died on `git clone ""`).
    let wm_configs_git_url = store_env
        .as_ref()
        .map(|e| e.wm_configs_git_url.clone())
        .or_else(|| std::env::var("GENTOO_INSTALLER_WM_CONFIGS_URL").ok())
        .unwrap_or_default();
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

    // Custom stage3 (your own tarball): GENTOO_INSTALLER_STAGE3_URL=<https://…|file://…|/path>,
    // optionally GENTOO_INSTALLER_STAGE3_SHA512. Unset = Gentoo's latest.
    let stage3 = std::env::var("GENTOO_INSTALLER_STAGE3_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .map(|u| installer_core::stage3::Stage3Source::custom(u.trim(), std::env::var("GENTOO_INSTALLER_STAGE3_SHA512").ok()));

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
        stage3,
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
        Step::Account => draw_account(frame, chunks[1], state),
        Step::Confirm => draw_confirm(frame, chunks[1], state),
        Step::Installing | Step::Done => draw_installing(frame, chunks[1], state),
    }
}

fn draw_network(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    if state.ethernet_up {
        let body = Paragraph::new("Ethernet link detected — network already up.\n\n[Enter] continue  [q] quit")
            .block(Block::default().borders(Borders::ALL).title("Network"));
        frame.render_widget(body, area);
        return;
    }

    let w = &state.wifi;
    let chunks = RtLayout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)]).split(area);
    let header = match &w.mode {
        WifiMode::Busy(what) => what.clone(),
        WifiMode::Passphrase { ssid, input, .. } => {
            format!("Passphrase for \"{ssid}\": {}▏   [Enter] connect  [Esc] back", "*".repeat(input.chars().count()))
        }
        WifiMode::List => "No Ethernet link. Pick a Wi-Fi network.".to_string(),
    };
    frame.render_widget(Paragraph::new(header).block(Block::default().borders(Borders::ALL).title("Network")), chunks[0]);

    let items: Vec<ListItem> = w
        .networks
        .iter()
        .enumerate()
        .map(|(i, n)| {
            // iwd reports signal in hundredths of a dBm (-3000 = -30 dBm).
            let bars = match n.signal_strength / 100 {
                s if s > -50 => "▂▄▆█",
                s if s > -60 => "▂▄▆ ",
                s if s > -70 => "▂▄  ",
                _ => "▂   ",
            };
            let lock = if n.secured { "🔒" } else { "  " };
            let style = if i == w.selected { Style::default().fg(Color::Black).bg(Color::Cyan) } else { Style::default() };
            ListItem::new(Line::from(format!(" {bars} {lock} {}", n.ssid))).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title("Networks")), chunks[1]);

    let hint = if w.message.is_empty() { "[↑↓] select  [Enter] connect  [r] rescan  [s] skip  [q] quit".to_string() } else { w.message.clone() };
    frame.render_widget(Paragraph::new(hint).block(Block::default().borders(Borders::ALL)), chunks[2]);
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

fn draw_account(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &AppState) {
    use crate::account_form::Field;
    let f = &state.account_form;
    let mark = |field: Field| if f.field == field { "> " } else { "  " };
    let text = format!(
        "Create the user for the new system (it joins the wheel group and can use doas; root stays locked).\n\n\
         {}Username:         {}\n\
         {}Password:         {}\n\
         {}Repeat password:  {}\n\n\
         [Tab/Enter] next field  [Enter on the last] continue  [Esc] back\n\n{}",
        mark(Field::Username),
        f.username,
        mark(Field::Password),
        "*".repeat(f.password.chars().count()),
        mark(Field::Confirm),
        "*".repeat(f.confirm.chars().count()),
        f.message,
    );
    frame.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Account")), area);
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
    let title = match (state.step == Step::Done, install_failed(&state.install_log)) {
        (true, true) => "Failed [q] quit",
        (true, false) => "Done - [Enter] reboot  [q] quit",
        _ => "Installing",
    };
    frame.render_widget(
        Paragraph::new(log).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_install_task_shows_its_error_even_though_done_never_arrives() {
        let mut state = AppState::new();
        state.step = Step::Installing;
        state.install_task = Some(tokio::spawn(async { Err(installer_core::Error::Other(anyhow::anyhow!("git clone failed"))) }));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        drain_install_progress(&mut state).await;

        assert_eq!(state.step, Step::Done);
        assert!(install_failed(&state.install_log), "{:?}", state.install_log);
        assert!(state.install_log.iter().any(|l| l.contains("git clone failed")));
    }

    #[test]
    fn only_a_log_without_errors_counts_as_a_finished_install() {
        assert!(!install_failed(&["Installing Limine...".into(), "Install complete.".into()]));
        assert!(install_failed(&["Partitioning disk...".into(), "ERROR: disk vanished".into()]));
    }
}
