#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(elided_lifetimes_in_paths)]
#![deny(macro_use_extern_crate)]
#![deny(missing_abi)]
#![warn(missing_debug_implementations)]
// #![deny(missing_docs)]
#![warn(noop_method_call)]
#![warn(single_use_lifetimes)]
#![deny(unreachable_pub)]
#![deny(unsafe_code)]
#![warn(unused_import_braces)]
#![deny(unused_lifetimes)]
#![warn(unused_macro_rules)]
#![warn(unused_qualifications)]
#![warn(variant_size_differences)]

//! The Chewing (酷音) intelligent phonetic input method library.
//!
//! This crate provides the core algorithms and facilities that can be used to
//! implement input methods and manipulate user dictionaries.
//!
//! # Behavior
//!
//! The [Editor][editor::Editor] implements the behavior of the Chewing input
//! method. Chewing is a bopomofo phonetics input method that can convert
//! keystrokes to Zhuyin/Bopomofo and then to Chinese characters. The state
//! machine powering the input method can detect the current input context and
//! translate the input to symbols, latin characters, or Chinese characters. The
//! Editor also has an option that is enabled by default to auto-learn new
//! phrases from users' input to provide more personalized intelligence.
//!
//! ```rust,no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use chewing::editor::{BasicEditor, Editor};
//! use chewing::input::{keycode, keysym, KeyboardEvent};
//! use chewing::input::keymap::{map_ascii, QWERTY_MAP};
//!
//! let mut editor = Editor::chewing(None, None)?;
//!
//! editor.process_keyevent(map_ascii(&QWERTY_MAP, b'd'));
//! editor.process_keyevent(map_ascii(&QWERTY_MAP, b'j'));
//! editor.process_keyevent(map_ascii(&QWERTY_MAP, b'4'));
//! editor.process_keyevent(KeyboardEvent::builder()
//!   .code(keycode::KEY_DOWN)
//!   .ksym(keysym::SYM_DOWN)
//!   .build());
//! editor.process_keyevent(map_ascii(&QWERTY_MAP, b'3'));
//!
//! assert_eq!("酷", editor.display());
//! # Ok(()) }
//! ```
//!
//! # Dictionary Files
//!
//! Chewing requires dictionary files at runtime. By default
//! [SearchPath](`crate::path::SearchPath`) will search
//! dictionaries from pre-configured system paths. See the [path] module level
//! document for the search order.
//!
//! `chewing-cli` can be used to compile the dictionary files.

mod bare;
pub mod conversion;
pub mod dictionary;
pub mod editor;
pub mod input;
pub mod lm;
pub mod model;
pub mod path;
pub mod user;
pub mod zhuyin;
