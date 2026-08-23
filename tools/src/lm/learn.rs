use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufWriter, Write, stdin},
    path::Path,
};

use anyhow::Result;
use chewing::{dictionary::StringTable, model::WordId};
use fxhash::FxHashMap;

type LocalCounts = (FxHashMap<WordId, u64>, FxHashMap<(WordId, WordId), u64>);

pub(crate) fn learn_lm(words: &Path, output: &Path) -> Result<()> {
    let words_table = StringTable::open(words)?;
    let words_map: FxHashMap<&str, u32> = words_table.iter().collect();

    let stdin = stdin();
    let n_threads = std::thread::available_parallelism()?.get();

    let (work_send, work_recv) = crossbeam_channel::bounded::<String>(n_threads * 2);
    let (result_send, result_recv) = crossbeam_channel::bounded::<LocalCounts>(n_threads);

    std::thread::scope(|s| {
        for _ in 0..n_threads {
            let work_recv = work_recv.clone();
            let result_send = result_send.clone();
            let words_map = &words_map;

            s.spawn(move || {
                let mut local_uni = FxHashMap::default();
                let mut local_bi = FxHashMap::default();

                while let Ok(line) = work_recv.recv() {
                    // Process line without collecting into a Vec to avoid allocations
                    let mut prev_wid = None;

                    for word in line.split_whitespace() {
                        if let Some(&wid) = words_map.get(word) {
                            let wid = WordId(wid);

                            // Unigram count
                            *local_uni.entry(wid).or_insert(0) += 1;

                            // Bigram count
                            if let Some(p_wid) = prev_wid {
                                *local_bi.entry((p_wid, wid)).or_insert(0) += 1;
                            }
                            prev_wid = Some(wid);
                        } else {
                            // Break bigram chain on unknown word
                            prev_wid = None;
                        }
                    }
                }
                // Send the local aggregation to the reducer
                result_send.send((local_uni, local_bi)).unwrap();
            });
        }
        drop(result_send);

        // Producer: Read stdin and distribute work
        for line in stdin.lock().lines() {
            if let Ok(l) = line {
                work_send.send(l).unwrap();
            }
        }
        drop(work_send);
    });

    // Reducer: Merge local results into final sorted BTreeMaps
    let mut unigram_total = 0;
    let mut unigrams = BTreeMap::new();
    let mut bigrams = BTreeMap::new();

    while let Ok((u, b)) = result_recv.recv() {
        for (wid, count) in u {
            *unigrams.entry(wid).or_insert(0) += count;
            unigram_total += count;
        }
        for (pair, count) in b {
            *bigrams.entry(pair).or_insert(0) += count;
        }
    }

    let mut out = BufWriter::new(File::create(output)?);

    writeln!(out, r"\data\")?;
    writeln!(out, "ngram 1={}", unigrams.len())?;
    writeln!(out, "ngram 2={}", bigrams.len())?;
    writeln!(out, "")?;

    writeln!(out, r"\1-grams:")?;
    for (&wid, &count) in &unigrams {
        let word = words_table.get(*wid).expect("should have word");
        let log10prob = (count as f64 / unigram_total as f64).log10();
        writeln!(out, "{} {}", log10prob, word)?;
    }

    const MIN_COUNT: u64 = 10;
    const ALPHA: f64 = 0.4;
    let log10_alpha = ALPHA.log10();

    writeln!(out, "")?;
    writeln!(out, r"\2-grams:")?;
    for ((wid1, wid2), count) in bigrams {
        // min occurrence pruning
        if count < MIN_COUNT {
            continue;
        }

        let cwp = unigrams.get(&wid1).expect("should have unigram");
        let log10prob = (count as f64 / *cwp as f64).log10();

        // stupid-back-off pruning
        let count_w2 = unigrams.get(&wid2).expect("should have unigram");
        let log10_unigram_w2 = (*count_w2 as f64 / unigram_total as f64).log10();

        let backoff_threshold = log10_alpha + log10_unigram_w2;

        if log10prob < backoff_threshold {
            // this bigram is worse than backing off!
            continue;
        }

        let word1 = words_table.get(*wid1).expect("should have word");
        let word2 = words_table.get(*wid2).expect("should have word");
        writeln!(out, "{} {} {}", log10prob, word1, word2)?;
    }

    writeln!(out, "")?;
    writeln!(out, r"\end\")?;
    out.flush()?;

    Ok(())
}
