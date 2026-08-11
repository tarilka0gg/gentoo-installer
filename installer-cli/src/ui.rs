//! Minimal ratatui render loop. Real per-step widgets (network scan list, disk table,
//! progress bar) get filled in as installer-core's async operations land.

use crate::steps::Step;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    layout::{Constraint, Layout as RtLayout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal,
};

pub async fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut step = Step::Network;

    loop {
        terminal.draw(|frame| draw(frame, step))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Enter => step = step.next(),
                    _ => {}
                }
            }
        }

        if step == Step::Done {
            return Ok(());
        }
    }
}

fn draw(frame: &mut ratatui::Frame, step: Step) {
    let area = frame.area();
    let chunks = RtLayout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);

    let title = Paragraph::new(Line::from("Gentoo Installer (TUI) — minimal ISO"))
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let body = Paragraph::new(format!("Step: {step:?}\n\n[Enter] next  [q] quit"))
        .block(Block::default().borders(Borders::ALL).title("Wizard"));
    frame.render_widget(body, chunks[1]);
}
