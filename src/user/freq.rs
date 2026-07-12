//! User word preferences
//!
//! The word preferences can be loaded from a file in this format:
//!
//!     <word>,<freq>
//!
//! Each line is separated by a new line (LF). The freq must be an integer
//! between [`UserFreq::MIN`] and [`UserFreq::MAX`].
//!
//! In Chewing, the user word preferences are used to seed the user history
//! table. It works like declaring the user has already type the word N times.
//! when freq is N.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    sync::{Arc, RwLock},
};

use scoped_error::{expect_error, impl_context_error};

use crate::model::WordId;

/// User word preferences
///
/// The UserFreq type implements [`Clone`] and can be cheaply cloned and
/// shared between components.
#[derive(Debug, Clone)]
pub struct UserFreq {
    inner: Arc<RwLock<UserFreqInner>>,
}

#[derive(Debug, Default)]
struct UserFreqInner {
    freq_table: BTreeMap<WordId, u32>,
}

impl UserFreq {
    pub const MIN: u32 = 1;
    pub const MAX: u32 = 9_999_999;

    // Creates an empty UserFreq
    pub fn new() -> UserFreq {
        UserFreq {
            inner: Default::default(),
        }
    }
    // Reads user freq from an IO stream
    pub fn from_reader<R, F>(read: R, widmap: F) -> Result<UserFreq, UserFreqError>
    where
        R: Read,
        F: Fn(&str) -> WordId,
    {
        expect_error("Failed to parse user freq", || {
            let reader = BufReader::new(read);
            let mut freq_table = BTreeMap::new();
            for (i, io) in reader.lines().enumerate() {
                let line = io?;
                let (word, freq) = line
                    .split_once(',')
                    .ok_or_else(|| format!("invalid format at line {i}: {line}"))?;
                let freq = freq.parse::<u32>()?.clamp(Self::MIN, Self::MAX);
                let wid = widmap(&word);
                freq_table.insert(wid, freq);
            }
            Ok(UserFreq {
                inner: Arc::new(RwLock::new(UserFreqInner { freq_table })),
            })
        })
    }
    // Gets the user freq of word
    pub fn get(&self, wid: WordId) -> Option<u32> {
        let lock = self
            .inner
            .read()
            .expect("Unable to acquire UserFreq reader lock");
        lock.freq_table.get(&wid).copied()
    }
}

impl_context_error!(pub UserFreqError);

#[cfg(test)]
mod test {
    use std::{
        error::Error,
        io::{Seek, Write},
    };

    use crate::model::WordId;

    use super::UserFreq;
    use scoped_error::ErrorExt;

    fn widmap(word: &str) -> WordId {
        match word {
            "測試" => WordId(0),
            "酷音" => WordId(1),
            "通過" => WordId(2),
            _ => panic!(),
        }
    }

    #[test]
    fn load_from_file() -> Result<(), Box<dyn Error>> {
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all("測試,0\n酷音,10000000\n通過,1000".as_bytes())?;
        file.flush()?;
        file.seek(std::io::SeekFrom::Start(0))?;

        let user_freq = UserFreq::from_reader(file, widmap)?;
        assert_eq!(Some(1), user_freq.get(WordId(0)));
        assert_eq!(Some(9999999), user_freq.get(WordId(1)));
        assert_eq!(Some(1000), user_freq.get(WordId(2)));
        assert_eq!(None, user_freq.get(WordId(3)));

        Ok(())
    }
    #[test]
    fn load_from_file_invalid() -> Result<(), Box<dyn Error>> {
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all("測試 0\n酷音 10000000\n通過 1000".as_bytes())?;
        file.flush()?;
        file.seek(std::io::SeekFrom::Start(0))?;

        let res = UserFreq::from_reader(file, widmap);
        eprintln!("{}", res.unwrap_err().report());

        Ok(())
    }
}
