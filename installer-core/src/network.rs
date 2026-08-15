//! Wifi/network setup via iwd over D-Bus (net.connman.iwd). No NetworkManager —
//! too heavy for the live environment, and iwd's D-Bus API avoids scraping `iwctl` stdout.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const IWD_DEST: &str = "net.connman.iwd";
const STATION_IFACE: &str = "net.connman.iwd.Station";
const NETWORK_IFACE: &str = "net.connman.iwd.Network";
const AGENT_MANAGER_IFACE: &str = "net.connman.iwd.AgentManager";
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER_IFACE: &str = "org.freedesktop.DBus.ObjectManager";
const AGENT_PATH: &str = "/net/connman/iwd/agent/gentoo_installer";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub path: String,
    pub ssid: String,
    pub signal_strength: i16,
    pub secured: bool,
}

pub struct IwdClient {
    conn: zbus::Connection,
    /// Set by `connect_to` right before it asks iwd to connect, read back by `Agent`
    /// when iwd calls `RequestPassphrase` as part of that same connection attempt — the
    /// GUI collects the passphrase inline (per screen script §4, in the row itself)
    /// before ever calling `connect_to`, so the answer is always ready when iwd asks.
    pending_passphrase: Arc<Mutex<Option<String>>>,
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// `net.connman.iwd.Agent`: the interface iwd calls back into for credentials it needs
/// mid-connection. Only `RequestPassphrase` is implemented — the WPA-PSK case covers the
/// overwhelming majority of home/office networks this installer will see; enterprise
/// (802.1X username+password) and WEP-key setups aren't handled.
#[derive(Clone)]
struct Agent {
    pending_passphrase: Arc<Mutex<Option<String>>>,
}

#[zbus::interface(name = "net.connman.iwd.Agent")]
impl Agent {
    async fn request_passphrase(&self, _network: ObjectPath<'_>) -> zbus::fdo::Result<String> {
        self.pending_passphrase
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| zbus::fdo::Error::Failed("no passphrase was provided before connecting".into()))
    }

    async fn cancel(&self, _reason: &str) {}

    async fn release(&self) {}
}

impl IwdClient {
    /// Connects to the system bus and registers our `Agent` with iwd's AgentManager —
    /// from this point on, iwd will call back into this process for passphrases on any
    /// secured-network connect attempt for the lifetime of `conn`.
    pub async fn connect() -> crate::Result<Self> {
        let conn = zbus::Connection::system()
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;

        let pending_passphrase = Arc::new(Mutex::new(None));
        let agent = Agent { pending_passphrase: pending_passphrase.clone() };
        conn.object_server()
            .at(AGENT_PATH, agent)
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;

        let agent_path = ObjectPath::try_from(AGENT_PATH).map_err(|e| crate::Error::Network(e.to_string()))?;
        conn.call_method(Some(IWD_DEST), "/net/connman/iwd", Some(AGENT_MANAGER_IFACE), "RegisterAgent", &(agent_path,))
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;

        Ok(Self { conn, pending_passphrase })
    }

    /// The first `net.connman.iwd.Station` object exposed by iwd — in practice the sole
    /// wifi interface on installer hardware, so no multi-adapter picker is offered.
    async fn station_path(&self) -> crate::Result<OwnedObjectPath> {
        let objects = self.managed_objects().await?;
        objects
            .into_iter()
            .find(|(_, ifaces)| ifaces.contains_key(STATION_IFACE))
            .map(|(path, _)| path)
            .ok_or_else(|| crate::Error::Network("no wifi station found via iwd".into()))
    }

    async fn managed_objects(&self) -> crate::Result<ManagedObjects> {
        let reply = self
            .conn
            .call_method(Some(IWD_DEST), "/", Some(OBJECT_MANAGER_IFACE), "GetManagedObjects", &())
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        reply
            .body()
            .deserialize::<ManagedObjects>()
            .map_err(|e| crate::Error::Network(e.to_string()))
    }

    /// Triggers an active scan and gives iwd a few seconds to populate results before
    /// `scan()`'s caller reads `GetOrderedNetworks` — iwd's Scan() call returns as soon as
    /// scanning *starts*, not when it finishes, and there's no blocking "wait for scan" call.
    pub async fn request_scan(&self) -> crate::Result<()> {
        let station = self.station_path().await?;
        self.conn
            .call_method(Some(IWD_DEST), &station, Some(STATION_IFACE), "Scan", &())
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        Ok(())
    }

    pub async fn scan(&self) -> crate::Result<Vec<Network>> {
        let station = self.station_path().await?;
        let reply = self
            .conn
            .call_method(
                Some(IWD_DEST),
                &station,
                Some(STATION_IFACE),
                "GetOrderedNetworks",
                &(),
            )
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        let ordered: Vec<(OwnedObjectPath, i16)> = reply
            .body()
            .deserialize()
            .map_err(|e| crate::Error::Network(e.to_string()))?;

        let mut networks = Vec::with_capacity(ordered.len());
        for (path, signal_strength) in ordered {
            let props = self.network_properties(&path).await?;
            let ssid = props
                .get("Name")
                .and_then(|v| v.downcast_ref::<&str>().ok())
                .unwrap_or_default()
                .to_string();
            let secured = props
                .get("Type")
                .and_then(|v| v.downcast_ref::<&str>().ok())
                .map(|t| t != "open")
                .unwrap_or(true);
            networks.push(Network {
                path: path.to_string(),
                ssid,
                signal_strength,
                secured,
            });
        }
        Ok(networks)
    }

    async fn network_properties(
        &self,
        path: &OwnedObjectPath,
    ) -> crate::Result<HashMap<String, OwnedValue>> {
        let reply = self
            .conn
            .call_method(
                Some(IWD_DEST),
                path,
                Some(PROPERTIES_IFACE),
                "GetAll",
                &(NETWORK_IFACE,),
            )
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        reply
            .body()
            .deserialize()
            .map_err(|e| crate::Error::Network(e.to_string()))
    }

    /// Connects to an already-scanned network by its D-Bus object path (`Network::path`).
    /// Open networks connect with no further input. For secured networks, `passphrase`
    /// is stashed for our registered `Agent` to hand back the moment iwd asks for it as
    /// part of this same `Connect` call.
    pub async fn connect_to(&self, network_path: &str, passphrase: Option<&str>) -> crate::Result<()> {
        *self.pending_passphrase.lock().unwrap() = passphrase.map(str::to_string);
        self.conn
            .call_method(Some(IWD_DEST), network_path, Some(NETWORK_IFACE), "Connect", &())
            .await
            .map_err(|e| crate::Error::Network(e.to_string()))?;
        Ok(())
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
