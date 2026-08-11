//! Wifi/network setup via iwd over D-Bus (net.connman.iwd). No NetworkManager —
//! too heavy for the live environment, and iwd's D-Bus API avoids scraping `iwctl` stdout.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub ssid: String,
    pub signal_strength: i16,
    pub secured: bool,
}

pub struct IwdClient {
    conn: zbus::Connection,
}

impl IwdClient {
    pub async fn connect() -> crate::Result<Self> {
        let conn = zbus::Connection::system()
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        Ok(Self { conn })
    }

    pub async fn scan(&self) -> crate::Result<Vec<Network>> {
        let _ = &self.conn;
        todo!("call net.connman.iwd.Station.Scan then GetOrderedNetworks over D-Bus")
    }

    pub async fn connect_to(&self, ssid: &str, passphrase: Option<&str>) -> crate::Result<()> {
        let _ = (&self.conn, ssid, passphrase);
        todo!("net.connman.iwd.Network.Connect, feeding the agent passphrase if secured")
    }

    /// Wired interfaces normally just come up via DHCP with no user action; this only
    /// confirms link state so the installer can skip the wifi step entirely.
    pub fn ethernet_link_up() -> bool {
        std::fs::read_dir("/sys/class/net")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name() != "lo")
            .any(|e| {
                std::fs::read_to_string(e.path().join("operstate"))
                    .map(|s| s.trim() == "up")
                    .unwrap_or(false)
            })
    }
}
