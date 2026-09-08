use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    str::FromStr,
};

use anyhow::{Context, Result};
use chewing::{
    dictionary::{StringTable, StringTableBuilder},
    lm::StaticDictBuilder,
    model::WordId,
    zhuyin::Syllable,
};

pub(crate) fn create_index_dict(src: &Path, words: &Path, dict_out: &Path) -> Result<()> {
    let read_err = || format!("Failed to read dictionary source from {}", src.display());
    let dict_write_err = || format!("Failed to write dictionary index to {}", dict_out.display());

    let string_table = StringTable::open_txt(words)?;

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
        .to_writer(File::create(dict_out).with_context(dict_write_err)?)
        .with_context(dict_write_err)?;

    Ok(())
}

pub(crate) fn create_string_table(words: &Path, words_out: &Path) -> Result<()> {
    let read_err = || {
        format!(
            "Failed to read string table source from {}",
            words.display()
        )
    };
    let words_write_err = || {
        format!(
            "Failed to write string table index to {}",
            words_out.display()
        )
    };

    let reader = BufReader::new(File::open(words).with_context(read_err)?);
    let mut builder = StringTableBuilder::new();

    for io in reader.lines() {
        let line = io?;
        builder.insert(line.trim());
    }

    builder
        .to_writer(File::create(words_out).with_context(words_write_err)?)
        .with_context(words_write_err)?;

    Ok(())
}
