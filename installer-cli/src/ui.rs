//! Ratatui render loop. Network/disk steps pull real data from installer-core
//! (iwd scan, lsblk, hardware detection); Confirm/Installing wire up as the
//! mutating installer-core calls (partition::apply, stage3, bootloader) get exercised.

use crate::steps::Step;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use installer_core::{disk, hardware, network};
use ratatui::{
    layout::{Constraint, Layout as RtLayout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    DefaultTerminal,
};

pub struct AppState {
    pub step: Step,
    pub ethernet_up: bool,
    pub disks: Vec<disk::Disk>,
    pub profile: Option<hardware::Profile>,
    pub selected_disk: usize,
    pub status: String,
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
        }
    }
}

pub async fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut state = AppState::new();

    loop {
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

async fn advance(state: &mut AppState) {
    let next = state.step.next();
    match next {
        Step::DiskSelect => {
            state.status = "detecting hardware + listing disks...".into();
            state.profile = hardware::Profile::detect().ok();
            match disk::list().await {
                Ok(disks) => {
                    state.disks = disks;
                    state.status.clear();
                }
                Err(e) => state.status = format!("disk listing failed: {e}"),
            }
        }
        Step::Confirm | Step::Installing | Step::Done => {}
        Step::Network => {}
    }
    state.step = next;
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
        Step::Installing | Step::Done => draw_placeholder(frame, chunks[1], "Installing..."),
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
            "CPU: {:?}  GPU: {:?}  RAM: {} GiB  Laptop: {}  Kernel profile: {}",
            p.cpu,
            p.gpu,
            p.ram_bytes / 1024 / 1024 / 1024,
            p.is_laptop,
            p.kernel_suffix()
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
        Some(d) => format!(
            "About to WIPE {} ({}) and install: ESP 512MiB, swap, btrfs root with @/@home/@var/@log subvolumes, Limine bootloader.\n\n[Enter] confirm and install  [q] abort",
            d.path,
            disk::format_size(d.size_bytes)
        ),
        None => "No disk selected.".to_string(),
    };
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Confirm")),
        area,
    );
}

fn draw_placeholder(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, msg: &str) {
    frame.render_widget(
        Paragraph::new(msg).block(Block::default().borders(Borders::ALL)),
        area,
    );
}
