use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    str::FromStr,
};

use anyhow::{Context, Result};
use chewing::{dictionary::StringTable, lm::StaticDictBuilder, model::WordId, zhuyin::Syllable};

pub(crate) fn create_index(src: &Path, words: &Path, out: &Path) -> Result<()> {
    let read_err = || format!("Failed to read dictionary source from {}", src.display());
    let write_err = || format!("Failed to write dictionary index to {}", out.display());

    let string_table = StringTable::open(words)?;
    let reader = BufReader::new(File::open(src).with_context(read_err)?);
    let mut builder = StaticDictBuilder::new();

    for io in reader.lines() {
        let line = io?;
        if line.starts_with('#') {
            continue;
        }
        let mut cols = line.split(',');
        let word = cols
            .next()
            .with_context(|| format!("Failed to parse line: {}", &line))?;
        let zhuyin = cols
            .next()
            .with_context(|| format!("Failed to parse line: {}", &line))?;

        let mut syllables = vec![];
        for syl_str in zhuyin.split_whitespace() {
            syllables.push(Syllable::from_str(syl_str)?);
        }

        if let Some(wid) = string_table.get_wid(word) {
            builder.insert(&syllables, WordId(*wid));
        }
    }

    builder
        .to_writer(File::create(out).with_context(write_err)?)
        .with_context(write_err)?;

    Ok(())
}
