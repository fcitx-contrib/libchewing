use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufRead, BufWriter, Write, stdin},
    path::Path,
};

use anyhow::Result;
use chewing::{dictionary::StringTable, model::WordId};

type LocalCounts = (HashMap<WordId, u64>, HashMap<(WordId, WordId), u64>);

/// Configuration for bigram pruning.
pub(crate) struct PruningConfig {
    /// Minimum bigram count to consider (hard floor).
    pub min_count: u64,
    /// Fraction of bigrams to keep after pruning (e.g. 0.2 = keep 20%).
    /// If `None`, falls back to the min_count threshold only.
    pub keep_fraction: Option<f64>,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            min_count: 10,
            keep_fraction: Some(0.3),
        }
    }
}

// Compute the distance between bigram probability and back-off probability.
// We can prune the entry if the back-off can mostly replace the bigram.
fn compute_d_scores(
    unigrams: &BTreeMap<WordId, u64>,
    bigrams: &BTreeMap<(WordId, WordId), u64>,
    unigram_total: u64,
    min_count: u64,
) -> Vec<((WordId, WordId), u64, f64)> {
    let mut scores = Vec::with_capacity(bigrams.len());

    for (&(wid1, wid2), &count) in bigrams {
        if count < min_count {
            continue;
        }

        let count_w1 = *unigrams.get(&wid1).expect("w1 must exist in unigrams");
        let count_w2 = *unigrams.get(&wid2).expect("w2 must exist in unigrams");

        // P(w2|w1) = count(w1,w2) / count(w1)
        let p_w2_given_w1 = count as f64 / count_w1 as f64;
        // P(w2) = count(w2) / N
        let p_w2 = count_w2 as f64 / unigram_total as f64;

        // stupid-backoff divergence contribution
        let d = (p_w2_given_w1 - 0.4 * p_w2).abs();

        scores.push(((wid1, wid2), count, d));
    }

    // Sort descending — largest first.
    scores.sort_unstable_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    scores
}

pub(crate) fn learn_lm(words: &Path, output: &Path, config: &PruningConfig) -> Result<()> {
    let string_table = StringTable::open_txt(words)?;

    let stdin = stdin();
    let n_threads = std::thread::available_parallelism()?.get();

    let (work_send, work_recv) = crossbeam_channel::bounded::<String>(n_threads * 2);
    let (result_send, result_recv) = crossbeam_channel::bounded::<LocalCounts>(n_threads);

    std::thread::scope(|s| {
        for _ in 0..n_threads {
            let work_recv = work_recv.clone();
            let result_send = result_send.clone();
            let string_table = &string_table;

            s.spawn(move || {
                let mut local_uni = HashMap::default();
                let mut local_bi = HashMap::default();

                while let Ok(line) = work_recv.recv() {
                    // Process line without collecting into a Vec to avoid allocations
                    let mut prev_wid = None;

                    for word in line.split_whitespace() {
                        if let Some(wid) = string_table.get_wid(word) {
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

    let d_scores = compute_d_scores(&unigrams, &bigrams, unigram_total, config.min_count);

    for frac in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
        eprintln!(
            "D at {}% = {}",
            frac * 100.0,
            d_scores[(d_scores.len() as f64 * frac).ceil() as usize].2
        );
    }
    for frac in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
        eprintln!(
            "SUM(D) at {}% = {}",
            frac * 100.0,
            d_scores[..(d_scores.len() as f64 * frac).ceil() as usize]
                .iter()
                .map(|v| v.2)
                .sum::<f64>()
        );
    }

    // Determine the threshold: keep the top `keep_fraction` of bigrams.
    let kl_threshold = if let Some(frac) = config.keep_fraction {
        let keep_count = (d_scores.len() as f64 * frac).ceil() as usize;
        if keep_count < d_scores.len() {
            // The cutoff is the score at the boundary.
            // Everything below this score gets pruned.
            Some(d_scores[keep_count].2)
        } else {
            None
        }
    } else {
        None
    };

    let keep_bigrams: HashMap<(WordId, WordId), u64> = if let Some(threshold) = kl_threshold {
        d_scores
            .iter()
            .filter(|(_, _, kl)| *kl >= threshold)
            .map(|&(pair, count, _)| (pair, count))
            .collect()
    } else {
        d_scores
            .iter()
            .map(|&(pair, count, _)| (pair, count))
            .collect()
    };

    let mut out = BufWriter::new(File::create(output)?);

    writeln!(out, r"\data\")?;
    writeln!(out, "ngram 1={}", unigrams.len())?;
    writeln!(out, "ngram 2={}", keep_bigrams.len())?;
    writeln!(out, "")?;

    writeln!(out, r"\1-grams:")?;
    for (&wid, &count) in &unigrams {
        let word = string_table.get_text(wid).expect("should have word");
        let log10prob = (count as f64 / unigram_total as f64).log10();
        writeln!(out, "{:.4} {}", log10prob, word)?;
    }

    writeln!(out, "")?;
    writeln!(out, r"\2-grams:")?;
    for (&(wid1, wid2), &count) in &keep_bigrams {
        let cwp = unigrams.get(&wid1).expect("should have unigram");
        let log10prob = (count as f64 / *cwp as f64).log10();

        let word1 = string_table.get_text(wid1).expect("should have word");
        let word2 = string_table.get_text(wid2).expect("should have word");
        writeln!(out, "{:.4} {} {}", log10prob, word1, word2)?;
    }

    writeln!(out, "")?;
    writeln!(out, r"\end\")?;
    out.flush()?;

    Ok(())
}
