// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 Kan-Ru Chen

use std::str::FromStr;

use crate::{
    editor::action::Action,
    input::{KeyState, KeyboardEvent, keysym::*},
};

use scoped_error::{Error, expect_error};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Prefix {
    /// Make the keybind trigger apply to only Entering context.
    Entering,
    /// Make the keybind trigger apply to only Selecting context.
    Selecting,
    /// Make the keybind trigger apply to all editor contexts.
    #[default]
    Any,
}

impl FromStr for Prefix {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "any" => Ok(Prefix::Any),
            _ => Err(Error::new(format!("Unknown prefix: {s}"))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keybinding {
    pub prefix: Prefix,
    pub trigger: KeyboardEvent,
    pub action: Action,
    pub param: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KeybindValue {
    pub prefix: Option<String>,
    pub trigger: String,
    pub action: String,
    pub param: Option<String>,
}

impl TryFrom<&KeybindValue> for Keybinding {
    type Error = Error;

    fn try_from(value: &KeybindValue) -> Result<Self, Self::Error> {
        expect_error("Failed to parse keybinding", || {
            let prefix = match &value.prefix {
                Some(s) => s.parse()?,
                None => Prefix::default(),
            };
            let trigger = key_from_str(&value.trigger)?;
            let action = value.action.parse()?;
            Ok(Keybinding {
                prefix,
                trigger,
                action,
                param: value.param.clone(),
            })
        })
    }
}

impl Keybinding {
    pub(crate) fn matches(&self, prefix: Prefix, evt: &KeyboardEvent) -> bool {
        (self.prefix == prefix || self.prefix == Prefix::Any)
            && (self.trigger.ksym == evt.ksym || self.trigger.ksym == SYM_NONE)
            && [
                KeyState::Alt,
                KeyState::Control,
                KeyState::Shift,
                KeyState::Super,
            ]
            .iter()
            .all(|&state| self.trigger.is_state_on(state) == evt.is_state_on(state))
    }
}

fn key_from_str(s: &str) -> Result<KeyboardEvent, Error> {
    let parts = s.split('+').rev();
    let mut builder = KeyboardEvent::builder();
    for key in parts {
        match key.trim().to_lowercase().as_str() {
            "ctrl" | "control" => {
                builder.control();
                continue;
            }
            "shift" => {
                builder.shift();
                continue;
            }
            "alt" | "opt" | "option" => {
                builder.alt_if(true);
                continue;
            }
            "super" | "cmd" | "command" => {
                builder.super_if(true);
                continue;
            }
            _ => {
                if let Some(ksym) = keysym_from_str(key) {
                    builder.ksym(ksym);
                    continue;
                }
            }
        }
        return Err(Error::new(format!("Unable to parse key: {s}")));
    }
    Ok(builder.build())
}

fn keysym_from_str(s: &str) -> Option<Keysym> {
    let s = s.trim();
    let ksym = match s {
        "Esc" => SYM_ESC,
        "F1" => SYM_F1,
        "F2" => SYM_F2,
        "F3" => SYM_F3,
        "F4" => SYM_F4,
        "F5" => SYM_F5,
        "F6" => SYM_F6,
        "F7" => SYM_F7,
        "F8" => SYM_F8,
        "F9" => SYM_F9,
        "F10" => SYM_F10,
        "F11" => SYM_F11,
        "F12" => SYM_F12,
        "Home" => SYM_HOME,
        "End" => SYM_END,
        "Delete" => SYM_DELETE,
        "Tab" => SYM_TAB,
        "Backspace" => SYM_BACKSPACE,
        "CapsLock" => SYM_CAPSLOCK,
        "Enter" | "Return" => SYM_RETURN,
        "Space" => SYM_SPACE,
        "LShift" | "ShiftLeft" | "LeftShift" => SYM_LEFTSHIFT,
        "RShift" | "ShiftRight" | "RightShift" => SYM_RIGHTSHIFT,
        _ if s.chars().count() == 1 => Keysym::from_char(s.chars().next().unwrap()),
        _ => return None,
    };
    Some(ksym)
}

#[cfg(test)]
mod tests {
    use crate::input::{
        KeyboardEvent,
        keysym::{Keysym, SYM_F12, SYM_LEFTALT, SYM_LEFTCTRL},
    };

    use super::KeybindValue;
    use super::Keybinding;
    use super::Prefix;
    use super::key_from_str;

    #[test]
    fn match_ctrl_f12() {
        let target = KeyboardEvent::builder().ksym(SYM_F12).control().build();
        assert_eq!(target, key_from_str("Ctrl+F12").unwrap());
        assert_eq!(target, key_from_str("ctrl+F12").unwrap());
        assert_eq!(target, key_from_str("control+F12").unwrap());
        let keybinding = Keybinding::try_from(&KeybindValue {
            prefix: None,
            trigger: "Ctrl+F12".to_string(),
            action: "ignore".to_string(),
            param: None,
        })
        .unwrap();
        assert!(keybinding.matches(Prefix::Any, &target));
        assert!(
            !keybinding.matches(
                Prefix::Any,
                &KeyboardEvent::builder()
                    .ksym(SYM_LEFTCTRL)
                    .control()
                    .build()
            )
        );
    }
    #[test]
    fn match_ctrl_shift_a() {
        let target = KeyboardEvent::builder()
            .ksym(Keysym::from_char('A'))
            .control()
            .shift()
            .build();
        assert_eq!(target, key_from_str("Ctrl+Shift+A").unwrap());
        assert_eq!(target, key_from_str("ctrl+shift+A").unwrap());
        assert_eq!(target, key_from_str("control+shift+A").unwrap());
        let keybinding = Keybinding::try_from(&KeybindValue {
            prefix: None,
            trigger: "Ctrl+Shift+A".to_string(),
            action: "ignore".to_string(),
            param: None,
        })
        .unwrap();
        assert!(keybinding.matches(Prefix::Any, &target));
    }
    #[test]
    fn match_shift_alt() {
        let target = KeyboardEvent::builder().shift().alt_if(true).build();
        assert_eq!(target, key_from_str("shift+alt").unwrap());
        assert_eq!(target, key_from_str("alt+shift").unwrap());
        assert_eq!(target, key_from_str("Shift+Alt").unwrap());
        let keybinding = Keybinding::try_from(&KeybindValue {
            prefix: None,
            trigger: "Shift+Alt".to_string(),
            action: "ignore".to_string(),
            param: None,
        })
        .unwrap();
        assert!(keybinding.matches(Prefix::Any, &target));
        assert!(
            keybinding.matches(
                Prefix::Any,
                &KeyboardEvent::builder()
                    .ksym(SYM_LEFTALT)
                    .shift()
                    .alt_if(true)
                    .build()
            )
        );
        assert!(!keybinding.matches(
            Prefix::Any,
            &KeyboardEvent::builder().ksym(SYM_LEFTALT).shift().build()
        ));
    }
}
