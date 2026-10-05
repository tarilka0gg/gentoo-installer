//! The TUI's Wi-Fi screen as a pure state machine, so it can be tested without a terminal or
//! an `iwd`: key presses go in, an [`Action`] for the caller to perform comes out. The caller
//! (`ui.rs`) owns the `IwdClient` and does the async work, then feeds results back with
//! [`WifiState::set_networks`], [`WifiState::connect_failed`] and so on.

use crossterm::event::KeyCode;
use installer_core::network::Network;

/// WPA-PSK passphrases are 8..=63 characters.
const MIN_PASSPHRASE: usize = 8;
const MAX_PASSPHRASE: usize = 63;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    List,
    Passphrase {
        path: String,
        ssid: String,
        input: String,
    },
    /// A scan or a connection is running; keys are ignored until it reports back.
    Busy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Scan,
    Connect {
        path: String,
        passphrase: Option<String>,
    },
    /// Leave the screen without (or after) connecting.
    Continue,
    Quit,
}

pub struct WifiState {
    pub networks: Vec<Network>,
    pub selected: usize,
    pub mode: Mode,
    pub message: String,
}

impl WifiState {
    pub fn new() -> Self {
        Self {
            networks: Vec::new(),
            selected: 0,
            mode: Mode::List,
            message: String::new(),
        }
    }

    /// Strongest first; iwd already orders them, this keeps the list stable if it does not.
    pub fn set_networks(&mut self, mut networks: Vec<Network>) {
        networks.sort_by(|a, b| b.signal_strength.cmp(&a.signal_strength));
        self.selected = 0;
        self.message = if networks.is_empty() {
            "No networks found. [r] rescan".into()
        } else {
            String::new()
        };
        self.networks = networks;
        self.mode = Mode::List;
    }

    pub fn scan_failed(&mut self, why: &str) {
        self.message = format!("Scan failed: {why}. [r] retry  [s] skip");
        self.mode = Mode::List;
    }

    pub fn connect_failed(&mut self, why: &str) {
        self.message = format!("Could not connect: {why}");
        self.mode = Mode::List;
    }

    pub fn begin(&mut self, what: &str) {
        self.mode = Mode::Busy(what.to_string());
        self.message.clear();
    }

    pub fn handle_key(&mut self, key: KeyCode) -> Option<Action> {
        match &mut self.mode {
            Mode::Busy(_) => None,
            Mode::Passphrase { path, input, .. } => match key {
                KeyCode::Char(c) if input.chars().count() < MAX_PASSPHRASE => {
                    input.push(c);
                    None
                }
                KeyCode::Backspace => {
                    input.pop();
                    None
                }
                KeyCode::Esc => {
                    self.mode = Mode::List;
                    self.message.clear();
                    None
                }
                KeyCode::Enter if input.chars().count() >= MIN_PASSPHRASE => {
                    let action = Action::Connect {
                        path: path.clone(),
                        passphrase: Some(input.clone()),
                    };
                    self.mode = Mode::Busy("Connecting…".into());
                    Some(action)
                }
                KeyCode::Enter => {
                    self.message =
                        format!("The passphrase needs at least {MIN_PASSPHRASE} characters.");
                    None
                }
                _ => None,
            },
            Mode::List => match key {
                KeyCode::Down if self.selected + 1 < self.networks.len() => {
                    self.selected += 1;
                    None
                }
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    None
                }
                KeyCode::Char('r') => {
                    self.begin("Scanning…");
                    Some(Action::Scan)
                }
                KeyCode::Char('s') => Some(Action::Continue),
                KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
                KeyCode::Enter => {
                    let net = self.networks.get(self.selected)?.clone();
                    if net.secured {
                        self.mode = Mode::Passphrase {
                            path: net.path,
                            ssid: net.ssid,
                            input: String::new(),
                        };
                        self.message.clear();
                        None
                    } else {
                        self.begin("Connecting…");
                        Some(Action::Connect {
                            path: net.path,
                            passphrase: None,
                        })
                    }
                }
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(ssid: &str, signal: i16, secured: bool) -> Network {
        Network {
            path: format!("/net/{ssid}"),
            ssid: ssid.into(),
            signal_strength: signal,
            secured,
        }
    }

    fn state() -> WifiState {
        let mut s = WifiState::new();
        s.set_networks(vec![
            net("weak", -8000, true),
            net("strong", -3000, true),
            net("open", -5000, false),
        ]);
        s
    }

    fn type_text(s: &mut WifiState, text: &str) {
        for c in text.chars() {
            s.handle_key(KeyCode::Char(c));
        }
    }

    #[test]
    fn networks_are_listed_strongest_first() {
        let s = state();
        assert_eq!(
            s.networks
                .iter()
                .map(|n| n.ssid.as_str())
                .collect::<Vec<_>>(),
            ["strong", "open", "weak"]
        );
    }

    #[test]
    fn an_open_network_connects_straight_away() {
        let mut s = state();
        s.handle_key(KeyCode::Down);
        assert_eq!(
            s.handle_key(KeyCode::Enter),
            Some(Action::Connect {
                path: "/net/open".into(),
                passphrase: None
            })
        );
        assert!(matches!(s.mode, Mode::Busy(_)));
    }

    #[test]
    fn a_secured_network_asks_for_the_passphrase_and_q_is_just_a_letter_there() {
        let mut s = state();
        assert_eq!(s.handle_key(KeyCode::Enter), None);
        assert!(matches!(s.mode, Mode::Passphrase { .. }));
        type_text(&mut s, "quiet-pass");
        // `q` must not quit while typing.
        let Mode::Passphrase { input, .. } = &s.mode else {
            panic!()
        };
        assert_eq!(input, "quiet-pass");
        assert_eq!(
            s.handle_key(KeyCode::Enter),
            Some(Action::Connect {
                path: "/net/strong".into(),
                passphrase: Some("quiet-pass".into())
            })
        );
    }

    #[test]
    fn a_short_passphrase_is_refused_before_anything_is_sent() {
        let mut s = state();
        s.handle_key(KeyCode::Enter);
        type_text(&mut s, "short");
        assert_eq!(s.handle_key(KeyCode::Enter), None);
        assert!(s.message.contains("at least 8"), "{}", s.message);
        assert!(matches!(s.mode, Mode::Passphrase { .. }));
    }

    #[test]
    fn backspace_and_escape_work_while_typing() {
        let mut s = state();
        s.handle_key(KeyCode::Enter);
        type_text(&mut s, "abc");
        s.handle_key(KeyCode::Backspace);
        let Mode::Passphrase { input, .. } = &s.mode else {
            panic!()
        };
        assert_eq!(input, "ab");
        s.handle_key(KeyCode::Esc);
        assert_eq!(s.mode, Mode::List);
    }

    #[test]
    fn the_passphrase_is_capped_at_63_characters() {
        let mut s = state();
        s.handle_key(KeyCode::Enter);
        type_text(&mut s, &"x".repeat(80));
        let Mode::Passphrase { input, .. } = &s.mode else {
            panic!()
        };
        assert_eq!(input.len(), 63);
    }

    #[test]
    fn keys_are_ignored_while_busy() {
        let mut s = state();
        assert_eq!(s.handle_key(KeyCode::Char('r')), Some(Action::Scan));
        assert_eq!(
            s.handle_key(KeyCode::Char('q')),
            None,
            "no quitting mid-scan"
        );
        assert_eq!(s.handle_key(KeyCode::Enter), None);
    }

    #[test]
    fn a_failed_connection_returns_to_the_list_with_the_reason() {
        let mut s = state();
        s.handle_key(KeyCode::Enter);
        type_text(&mut s, "wrong-password");
        s.handle_key(KeyCode::Enter);
        s.connect_failed("authentication failed");
        assert_eq!(s.mode, Mode::List);
        assert!(s.message.contains("authentication failed"));
    }

    #[test]
    fn an_empty_scan_says_so_and_skip_always_works() {
        let mut s = WifiState::new();
        s.set_networks(vec![]);
        assert!(s.message.contains("No networks"));
        assert_eq!(s.handle_key(KeyCode::Enter), None, "nothing to select");
        assert_eq!(s.handle_key(KeyCode::Char('s')), Some(Action::Continue));
    }
}
