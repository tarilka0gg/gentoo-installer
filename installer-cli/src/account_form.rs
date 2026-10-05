//! The TUI's "create your user" screen as a pure state machine (same idea as `wifi.rs`): keys in,
//! an [`Outcome`] out, no terminal needed to test it. Every character is input here — `q` and Esc
//! must not quit while someone is typing a password; Esc goes back one screen instead.

use crossterm::event::KeyCode;
use installer_core::account::{validate_username, Account};

const MAX_USERNAME: usize = 32;
const MAX_PASSWORD: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Username,
    Password,
    Confirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    Submit(Account),
    Back,
}

pub struct AccountForm {
    pub username: String,
    pub password: String,
    pub confirm: String,
    pub field: Field,
    pub message: String,
}

impl AccountForm {
    pub fn new() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            confirm: String::new(),
            field: Field::Username,
            message: String::new(),
        }
    }

    fn current(&mut self) -> (&mut String, usize) {
        match self.field {
            Field::Username => (&mut self.username, MAX_USERNAME),
            Field::Password => (&mut self.password, MAX_PASSWORD),
            Field::Confirm => (&mut self.confirm, MAX_PASSWORD),
        }
    }

    fn next(&mut self) {
        self.field = match self.field {
            Field::Username => Field::Password,
            _ => Field::Confirm,
        };
    }

    fn prev(&mut self) {
        self.field = match self.field {
            Field::Confirm => Field::Password,
            _ => Field::Username,
        };
    }

    pub fn handle_key(&mut self, key: KeyCode) -> Outcome {
        match key {
            KeyCode::Esc => return Outcome::Back,
            KeyCode::Tab | KeyCode::Down => self.next(),
            KeyCode::BackTab | KeyCode::Up => self.prev(),
            KeyCode::Backspace => {
                self.current().0.pop();
            }
            KeyCode::Char(c) => {
                let (s, max) = self.current();
                if s.chars().count() < max {
                    s.push(c);
                }
                self.message.clear();
            }
            KeyCode::Enter if self.field != Field::Confirm => self.next(),
            KeyCode::Enter => return self.submit(),
            _ => {}
        }
        Outcome::None
    }

    fn submit(&mut self) -> Outcome {
        if let Err(e) = validate_username(&self.username) {
            self.message = e.to_string();
            self.field = Field::Username;
        } else if self.password.is_empty() {
            self.message = "The password is empty.".into();
            self.field = Field::Password;
        } else if self.password != self.confirm {
            self.message = "The two passwords differ.".into();
            self.confirm.clear();
            self.field = Field::Confirm;
        } else {
            return Outcome::Submit(Account {
                username: self.username.clone(),
                password: self.password.clone(),
            });
        }
        Outcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(f: &mut AccountForm, t: &str) {
        for c in t.chars() {
            f.handle_key(KeyCode::Char(c));
        }
    }

    fn filled(user: &str, pass: &str, confirm: &str) -> AccountForm {
        let mut f = AccountForm::new();
        type_text(&mut f, user);
        f.handle_key(KeyCode::Enter);
        type_text(&mut f, pass);
        f.handle_key(KeyCode::Enter);
        type_text(&mut f, confirm);
        f
    }

    #[test]
    fn a_good_form_submits_exactly_what_was_typed() {
        let mut f = filled("solomiya", "q-hunter two", "q-hunter two");
        assert_eq!(
            f.handle_key(KeyCode::Enter),
            Outcome::Submit(Account {
                username: "solomiya".into(),
                password: "q-hunter two".into()
            })
        );
    }

    #[test]
    fn q_and_spaces_are_just_input_and_esc_goes_back() {
        let mut f = AccountForm::new();
        type_text(&mut f, "quiet q");
        assert_eq!(f.username, "quiet q");
        assert_eq!(f.handle_key(KeyCode::Esc), Outcome::Back);
    }

    #[test]
    fn a_bad_username_is_refused_and_the_cursor_goes_back_to_it() {
        let mut f = filled("Solomiya", "pw", "pw");
        assert_eq!(f.handle_key(KeyCode::Enter), Outcome::None);
        assert_eq!(f.field, Field::Username);
        assert!(f.message.contains("invalid username"), "{}", f.message);
    }

    #[test]
    fn mismatching_or_empty_passwords_are_refused() {
        let mut f = filled("solomiya", "one", "two");
        assert_eq!(f.handle_key(KeyCode::Enter), Outcome::None);
        assert!(f.message.contains("differ"), "{}", f.message);
        assert!(
            f.confirm.is_empty(),
            "the confirmation is cleared so it is typed again"
        );

        let mut f = filled("solomiya", "", "");
        assert_eq!(f.handle_key(KeyCode::Enter), Outcome::None);
        assert!(f.message.contains("empty"));
    }

    #[test]
    fn tab_and_arrows_move_between_fields_and_backspace_edits() {
        let mut f = AccountForm::new();
        f.handle_key(KeyCode::Tab);
        assert_eq!(f.field, Field::Password);
        type_text(&mut f, "abc");
        f.handle_key(KeyCode::Backspace);
        assert_eq!(f.password, "ab");
        f.handle_key(KeyCode::Up);
        assert_eq!(f.field, Field::Username);
    }

    #[test]
    fn lengths_are_capped() {
        let mut f = AccountForm::new();
        type_text(&mut f, &"a".repeat(80));
        assert_eq!(f.username.len(), 32);
    }
}
