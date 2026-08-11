//! TUI installer for the minimal ISO (no GUI/DE/WM on that image at all).
//! Steps: network (skip if ethernet already up) -> disk select -> confirm -> install.

mod steps;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = ui::run(&mut terminal).await;
    ratatui::restore();
    result
}
