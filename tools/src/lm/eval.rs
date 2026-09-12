use std::{
    io::{BufRead, stdin},
    thread,
};

use anyhow::{Context, Result};
use chewing::{
    conversion::{ChewingEngine, Composition, ConversionEngine, Decoder, LatticeBuilder, Symbol},
    dictionary::{CompositeDict, LookupStrategy, StringTable},
    lm::{LoadMode, StaticDict, StaticLm},
    path::SearchPath,
    user::{HistoryDict, UserDict},
    zhuyin::Syllable,
};

pub(crate) fn eval(search_path: &str, model_bin: &str, alpha: f64, verbose: bool) -> Result<()> {
    let sp = SearchPath::from_system_path_and_env(search_path);

    let string_table = StringTable::open_bin(
        sp.find_file("static_words.bin")
            .context("find static_words.bin")?,
    )?;
    let static_dict = StaticDict::open(
        sp.find_file("static_dict.bin")
            .context("find static_dict.bin")?,
    )?;
    let rare_dict = StaticDict::open(
        sp.find_file("rare_dict.bin")
            .context("find rare_dict.bin")?,
    )?;
    let lm = StaticLm::open(
        sp.find_file(model_bin).context("find static_lm.bin")?,
        LoadMode::Lazy,
    )?;
    let history_dict = HistoryDict::new(string_table.clone());
    let user_dict = UserDict::new(string_table.clone());

    let dict = CompositeDict::new(static_dict, rare_dict, history_dict, user_dict);

    let lattice_builder = LatticeBuilder {
        dict,
        lookup_strategy: LookupStrategy::Standard,
    };
    let decoder = Decoder { lm, alpha };

    let engine = ChewingEngine {
        word_lattice_builder: lattice_builder,
        decoder,
        string_table,
    };

    let n_threads = thread::available_parallelism()?.get();
    let (work_send, work_recv) = crossbeam_channel::bounded::<(String, String)>(n_threads * 2);
    let (result_send, result_recv) = crossbeam_channel::bounded::<(usize, usize)>(4096);

    let output_handle = thread::spawn(move || {
        let mut total_char = 0;
        let mut total_dist = 0;
        loop {
            let Ok(res) = result_recv.recv() else {
                println!(
                    "{:.4}",
                    (total_char - total_dist) as f64 / total_char as f64
                );
                return;
            };
            total_char += res.0;
            total_dist += res.1;
        }
    });
    thread::scope(|s| {
        for _ in 0..n_threads {
            s.spawn(|| {
                loop {
                    let Ok((input, answer)) = work_recv.recv() else {
                        return;
                    };

                    let mut comp = Composition::new();

                    for part in input.split_whitespace() {
                        if part == "，" {
                            comp.push(Symbol::Grapheme('，'));
                        } else {
                            let syl: Syllable = part.parse().unwrap();
                            comp.push(Symbol::Syllable(syl));
                        }
                    }

                    if let Some(outcome) = engine.convert(&comp).get(0) {
                        let result: String = outcome
                            .intervals
                            .iter()
                            .map(|it| it.text.to_string())
                            .collect();
                        let chars = answer.chars().count();
                        let dist = distance(&result, answer.trim());
                        if verbose && dist > 0 {
                            println!("Difference {}:", dist);
                            println!("I:  {}", input);
                            println!("A:  {}", answer);
                            println!("R:  {}", result);
                            print_diff(answer.trim(), &result);
                        }
                        result_send
                            .send((chars, dist))
                            .expect("unable to send result");
                    } else {
                        println!("Failed to convert:");
                        println!("  {}", input);
                        println!("  {}", answer);
                    }
                }
            });
        }
        let mut lock = stdin().lock();
        loop {
            let mut input = String::new();
            let mut answer = String::new();

            if lock.read_line(&mut input).unwrap() == 0 {
                break;
            }
            if lock.read_line(&mut answer).unwrap() == 0 {
                break;
            }
            work_send
                .send((input, answer))
                .expect("unable to send input to worker thread");
        }
        drop(work_send);
    });
    drop(result_send);
    output_handle.join().expect("failed to write to stdout");

    Ok(())
}

// Char by char comparison
#[inline]
fn distance(str1: &str, str2: &str) -> usize {
    str1.chars()
        .zip(str2.chars())
        .map(|(a, b)| if a == b { 0 } else { 1 })
        .sum()
}

#[inline]
fn print_diff(str1: &str, str2: &str) {
    str1.chars().zip(str2.chars()).for_each(|(a, b)| {
        if a == b {
            print!(" ")
        } else {
            print!("-{}/+{}", a, b)
        }
    });
    println!("");
}
