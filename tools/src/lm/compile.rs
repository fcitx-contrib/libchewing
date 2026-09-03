use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

use anyhow::{Context, Result, bail};
use chewing::{dictionary::StringTable, lm::StaticLmCompiler, model::WordId};

enum ArpaSection {
    Begin,
    Data,
    Unigram,
    Bigram,
    End,
}

pub(crate) fn compile_lm(arpa: &Path, words: &Path, output: &Path) -> Result<()> {
    let mut compiler = StaticLmCompiler::new();
    let arpa_reader = BufReader::new(File::open(arpa)?);
    let string_table = StringTable::open(words)?;

    let mut section = ArpaSection::Begin;
    for io in arpa_reader.lines() {
        let line = io?;

        if line.is_empty() {
            continue;
        }

        if line == r"\data\" {
            if matches!(section, ArpaSection::Begin) {
                section = ArpaSection::Data;
                continue;
            } else {
                bail!(r"The \data\ section should be the first file section");
            }
        } else if line == r"\1-grams:" {
            if matches!(section, ArpaSection::Data) {
                section = ArpaSection::Unigram;
                continue;
            } else {
                bail!(r"The \1-grams: section must follow the \data\ section");
            }
        } else if line == r"\2-grams:" {
            if matches!(section, ArpaSection::Unigram) {
                section = ArpaSection::Bigram;
                continue;
            } else {
                bail!(r"The \2-grams: section must follow the \1-grams: section");
            }
        } else if line == r"\end\" {
            section = ArpaSection::End;
            continue;
        }

        if matches!(section, ArpaSection::Data) {
            // TODO: save expected n-gram counts
            continue;
        }

        if matches!(section, ArpaSection::Unigram) {
            let mut cols = line.split_whitespace();
            let log_prob: f64 = cols.next().context("Expecting log probability")?.parse()?;
            let word = cols.next().context("Expecting word")?.trim();
            let wid = string_table
                .get_wid(word)
                .with_context(|| format!("Unknown word {word}"))?;
            compiler.insert(WordId(0), WordId(*wid), log_prob)?;
        }

        if matches!(section, ArpaSection::Bigram) {
            let mut cols = line.split_whitespace();
            let log_prob: f64 = cols.next().context("Expecting log probability")?.parse()?;
            let word1 = cols.next().context("Expecting word")?.trim();
            let word2 = cols.next().context("Expecting word")?.trim();
            let wid1 = string_table
                .get_wid(word1)
                .with_context(|| format!("Unknown word {word1}"))?;
            let wid2 = string_table
                .get_wid(word2)
                .with_context(|| format!("Unknown word {word2}"))?;
            compiler.insert(WordId(*wid1), WordId(*wid2), log_prob)?;
        }

        if matches!(section, ArpaSection::End) {
            continue;
        }
    }

    let mut static_lm_bin = BufWriter::new(File::create(output)?);
    compiler.to_writer(&mut static_lm_bin)?;
    static_lm_bin.flush()?;

    Ok(())
}
