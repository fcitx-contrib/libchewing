use std::{
    error::Error,
    ffi::OsStr,
    fmt::Display,
    fs, io,
    path::{Path, PathBuf},
};

use log::{error, info};

use super::{Dictionary, Trie};
use crate::{
    dictionary::DictionaryUsage,
    editor::{AbbrevTable, SymbolSelector},
    exn::{Exn, ResultExt},
    path::custom_search_path_and_env_var,
    path::{find_files_by_names, find_path_by_files, search_path_from_env_var, userphrase_path},
};

// const UD_TRIE_FILE_NAME: &str = "chewing.dat";
const ABBREV_FILE_NAME: &str = "swkb.dat";
const SYMBOLS_FILE_NAME: &str = "symbols.dat";

pub const DEFAULT_DICT_NAMES: &[&str] = &["word.dat", "tsi.dat", "chewing.dat"];

/// Automatically searchs and loads dictionaries.
#[derive(Debug, Default)]
pub struct AssetLoader {
    search_path: Option<String>,
}

impl AssetLoader {
    /// Creates a new dictionary loader.
    pub fn new() -> AssetLoader {
        AssetLoader::default()
    }
    /// Override the default dictionary search path.
    pub fn search_path(mut self, search_path: impl Into<String>) -> AssetLoader {
        self.search_path = Some(search_path.into());
        self
    }
    /// Loads the abbrev table.
    pub fn load_abbrev(&self) -> Result<AbbrevTable, LoadDictionaryError> {
        let error = || LoadDictionaryError::new("failed to load abbrev table");
        let not_found = || error().with_source(io::Error::from(io::ErrorKind::NotFound));
        let search_path = if let Some(path) = &self.search_path {
            path.to_owned()
        } else {
            search_path_from_env_var()
        };
        let parent_path =
            find_path_by_files(&search_path, &[ABBREV_FILE_NAME]).or_raise(not_found)?;
        let abbrev_path = parent_path.join(ABBREV_FILE_NAME);
        info!("Load abbrev table: {}", abbrev_path.display());
        AbbrevTable::open(abbrev_path).or_raise(error)
    }
    /// Loads the symbol table.
    pub fn load_symbol_selector(&self) -> Result<SymbolSelector, LoadDictionaryError> {
        let error = || LoadDictionaryError::new("failed to load symbol table");
        let not_found = || error().with_source(io::Error::from(io::ErrorKind::NotFound));
        let search_path = if let Some(path) = &self.search_path {
            path.to_owned()
        } else {
            search_path_from_env_var()
        };
        let parent_path =
            find_path_by_files(&search_path, &[SYMBOLS_FILE_NAME]).or_raise(not_found)?;
        let symbol_path = parent_path.join(SYMBOLS_FILE_NAME);
        info!("Load symbol table: {}", symbol_path.display());
        SymbolSelector::open(symbol_path).or_raise(error)
    }
}
