use std::{convert::Infallible, str::FromStr};

#[derive(Debug, Clone)]
pub enum Action {
    MoveCursorLeft,
    MoveCursorRight,
    QuickSelect,
    /// Ignore this key combination.
    Ignore,
    Other(String),
}

impl FromStr for Action {
    type Err = Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "move_cursor_left" => Ok(Action::MoveCursorLeft),
            "move_cursor_right" => Ok(Action::MoveCursorRight),
            "quick_select" => Ok(Action::QuickSelect),
            "ignore" => Ok(Action::Ignore),
            _ => Ok(Action::Other(s.to_string())),
        }
    }
}
