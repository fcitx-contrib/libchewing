use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Write, stdin},
    path::Path,
};

use anyhow::{Context, Result, bail};
use chewing::{dictionary::StringTable, model::WordId};

pub(crate) fn learn_lm(words: &Path, output: &Path) -> Result<()> {
    let words_table = StringTable::open(words)?;
    let words_map = words_table.to_map();

    let mut unigram_total: u64 = 0;
    let mut unigrams: BTreeMap<WordId, u64> = BTreeMap::new();

    let mut bigrams: BTreeMap<(WordId, WordId), u64> = BTreeMap::new();

    let stdin = stdin().lock();

    for io in stdin.lines() {
        let line = io?;
        let words = line
            .trim()
            .split_whitespace()
            .filter(|ss| !ss.is_empty())
            .collect::<Vec<_>>();

        // count unigrams
        for word in &words {
            if let Some(wid) = words_map.get(word) {
                unigram_total += 1;
                unigrams
                    .entry(WordId(*wid))
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }

        // count bigrams
        for window in words.windows(2) {
            let word1 = window[0];
            let word2 = window[1];
            if let Some(wid1) = words_map.get(word1)
                && let Some(wid2) = words_map.get(word2)
            {
                bigrams
                    .entry((WordId(*wid1), WordId(*wid2)))
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }
    }

    let mut out = File::create(output)?;

    writeln!(out, r"\data\")?;
    writeln!(out, "ngram 1={}", unigrams.len())?;
    writeln!(out, "")?;

    writeln!(out, r"\1-grams:")?;
    for (&wid, &count) in &unigrams {
        let word = words_table.get(*wid).expect("should have word");
        let log10prob = (count as f64 / unigram_total as f64).log10();
        writeln!(out, "{} {}", log10prob, word)?;
    }

    writeln!(out, "")?;
    writeln!(out, r"\2-grams:")?;
    for ((wid1, wid2), count) in bigrams {
        let cwp = unigrams.get(&wid1).expect("should have unigram");
        let log10prob = (count as f64 / *cwp as f64).log10();
        let word1 = words_table.get(*wid1).expect("should have word");
        let word2 = words_table.get(*wid2).expect("should have word");
        writeln!(out, "{} {} {}", log10prob, word1, word2)?;
    }

    writeln!(out, "")?;
    writeln!(out, r"\end\")?;
    Ok(())
}
