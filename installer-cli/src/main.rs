//! TUI installer for the minimal ISO (no GUI/DE/WM on that image at all).
//! Steps: network (skip if ethernet already up) -> disk select -> confirm -> install.

mod account_form;
mod headless;
mod steps;
mod ui;
mod wifi;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--headless") {
        return headless::run().await;
    }
    let mut terminal = ratatui::init();
    let result = ui::run(&mut terminal).await;
    ratatui::restore();
    result
}
