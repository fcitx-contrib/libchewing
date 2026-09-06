use std::{
    any::Any,
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::bail;
use anyhow::{Context, Result};
use chewing::{
    dictionary::{DictionaryBuilder, DictionaryInfo, TrieBuilder},
    zhuyin::{Bopomofo, Syllable},
};

use crate::flags;

const MAX_LEN: usize = 6;

pub(crate) fn run(args: flags::InitDatabase) -> Result<()> {
    let error = "Failed to build dictionary file.";
    let parse_error = |line_num, line: &str, msg| {
        anyhow::Error::msg(format!("{line_num:>5} | {line}\n{msg} at line {line_num}"))
    };

    let mut builder: Box<dyn DictionaryBuilder> = match args.db_type {
        flags::DbType::Sqlite => {
            bail!("sqlite3 dictionary format support was not removed.");
        }
        flags::DbType::Trie => Box::new(TrieBuilder::new()),
    };

    let mut name = args.name;
    let mut copyright = args.copyright;
    let mut license = args.license;
    let mut version = args.version;
    let mut usage = args.usage;
    let software = format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    let tsi = File::open(args.tsi_src).context(error)?;
    let reader = BufReader::new(tsi);
    let delimiter = if args.csv { ',' } else { ' ' };
    let mut read_front_matter = true;
    let mut errors = vec![];
    let mut phrase_rows: HashMap<String, Vec<(i32, usize)>> = HashMap::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line.context(error)?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Read front matter until first non-comment line
        if read_front_matter && !line.starts_with('#') {
            read_front_matter = false;
        } else if read_front_matter {
            let Some((key, value)) = line.trim_start_matches('#').trim().split_once(delimiter)
            else {
                errors.push(parse_error(line_num, line, "Invalid metadata"));
                continue;
            };
            let value = value.trim_end_matches(delimiter).to_string();
            match key.trim() {
                "dc:title" => name = value,
                "dc:rights" => copyright = value,
                "dc:license" => license = value,
                "dc:identifier" => version = value,
                "dc:type" => usage = value.parse().unwrap(),
                _ => (),
            }
            continue;
        } else if line.starts_with('#') {
            continue;
        }
        match parse_line(delimiter, &line, args.fix) {
            Ok((syllables, phrase, freq)) => {
                if syllables.len() != phrase.chars().count() {
                    errors.push(parse_error(line_num, line, "Word count doesn't match"));
                    continue;
                }
                phrase_rows
                    .entry(phrase.to_string())
                    .or_default()
                    .push((freq, syllables.len()));
                builder
                    .insert(&syllables, (phrase, freq).into())
                    .context(error)?;
            }
            Err(error) => errors.push(error.context(parse_error(line_num, line, "Parse error"))),
        };
    }
    if !errors.is_empty() {
        for err in errors {
            eprintln!("{:#}", err);
        }
        if !args.fix {
            eprintln!();
            eprintln!("Hint: Use --csv flag to enable CSV parsing.");
            eprintln!("Hint: Use --fix to automatically fix common errors.");
        }
        if !args.skip_invalid {
            std::process::exit(1)
        }
    }
    let path: &Path = args.output.as_ref();
    if path.exists() {
        fs::remove_file(path).context("unable to overwrite output")?;
    }

    let info = DictionaryInfo {
        name,
        copyright,
        license,
        version,
        software,
        usage,
    };
    builder.set_info(info.clone())?;

    builder.build(path)?;

    if let Some(trie_builder) = (builder as Box<dyn Any>).downcast_ref::<TrieBuilder>() {
        let stats = trie_builder.statistics();
        eprintln!("== Trie Dictionary Statistics ==");
        eprintln!("Name                 : {}", info.name);
        eprintln!("Copyright            : {}", info.copyright);
        eprintln!("License              : {}", info.license);
        eprintln!("Version              : {}", info.version);
        eprintln!("Usage                : {}", info.usage);
        eprintln!("Node count           : {}", stats.node_count);
        eprintln!("Leaf count           : {}", stats.leaf_count);
        eprintln!("Phrase count         : {}", stats.phrase_count);
        eprintln!("Max height           : {}", stats.max_height);
        eprintln!("Average height       : {}", stats.avg_height);
        eprintln!("Root branch count    : {}", stats.root_branch_count);
        eprintln!("Max branch count     : {}", stats.max_branch_count);
        eprintln!("Average branch count : {}", stats.avg_branch_count);
    }

    compute_and_print_length_prob(&phrase_rows);

    Ok(())
}

fn compute_and_print_length_prob(phrase_rows: &HashMap<String, Vec<(i32, usize)>>) {
    let mut d = 0u64;
    let mut len_type: HashMap<usize, usize> = HashMap::new();
    let mut n_aggregate = 0usize;
    let mut n_per_reading = 0usize;
    let mut n_mixed_len = 0usize;

    for entries in phrase_rows.values() {
        let freqs: Vec<i32> = entries.iter().map(|(f, _)| *f).collect();
        let lens: HashSet<usize> = entries.iter().map(|(_, n)| *n).collect();

        if lens.len() > 1 {
            n_mixed_len += 1;
        }
        let bucket = std::cmp::min(*lens.iter().min().unwrap_or(&MAX_LEN), MAX_LEN);

        let phrase_freq = if entries.len() > 1 && freqs.iter().all(|&f| f == freqs[0]) {
            n_aggregate += 1;
            freqs[0]
        } else {
            if entries.len() > 1 {
                n_per_reading += 1;
            }
            freqs.iter().map(|&f| f as u64).sum::<u64>() as i32
        };

        d += phrase_freq as u64;
        *len_type.entry(bucket).or_insert(0) += 1;
    }

    let total = len_type.values().sum::<usize>() as f64;
    let mut p_len: Vec<(usize, f64)> = len_type
        .iter()
        .map(|(&n, &count)| (n, count as f64 / total))
        .collect();
    p_len.sort_by_key(|(n, _)| *n);

    let s: f64 = p_len.iter().map(|(_, p)| p).sum();
    assert!((s - 1.0).abs() < 1e-9, "length prob must sum to 1, got {s}");

    let total_rows: usize = phrase_rows.values().map(|v| v.len()).sum();

    eprintln!("");
    eprintln!("== Length Prior Weighting ==");
    eprintln!("Distinct phrases       : {}", phrase_rows.len());
    eprintln!("Total rows (edges)     : {}", total_rows);
    eprintln!(
        "Duplicated-aggregate   : {} phrases (counted once)",
        n_aggregate
    );
    eprintln!(
        "Multi-reading          : {} phrases (summed)",
        n_per_reading
    );
    if n_mixed_len > 0 {
        eprintln!(
            "Phrases w/ inconsistent syllable length across readings: {}",
            n_mixed_len
        );
    }
    eprintln!("D = corrected sum(freq): {}", d);
    eprintln!("Sum(P_len)             : {:.12}\n", s);

    eprintln!(
        "{:>4} {:>16} {:>12} {:>14}",
        "len", "count", "P(len)", "log P(len)"
    );
    for (n, p) in &p_len {
        let label = if *n == MAX_LEN {
            format!("{}+", n)
        } else {
            n.to_string()
        };
        let count = len_type.get(n).copied().unwrap_or(0);
        eprintln!("{:>4} {:>16} {:>12.6} {:>14.6}", label, count, p, p.ln());
    }
}

fn parse_line(delimiter: char, line: &str, fix: bool) -> Result<(Vec<Syllable>, &str, i32)> {
    let phrase = line
        .split(delimiter)
        .find(|s| !s.is_empty())
        .context("failed to parse phrase")?
        .trim_matches('"');

    let freq: i32 = line
        .split(delimiter)
        .filter(|s| !s.is_empty())
        .nth(1)
        .context("failed to parse frequency")?
        .trim_matches('"')
        .parse()
        .context("failed to parse frequency")?;

    let mut syllables = vec![];

    for syllable_str in line
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .skip(2)
    // skip phrase and freq
    {
        let syllable_str = syllable_str.trim_matches('"');
        if syllable_str.is_empty() {
            continue;
        }
        let mut syllable_builder = Syllable::builder();
        if syllable_str.starts_with('#') {
            break;
        }
        for c in syllable_str.chars() {
            let c = if fix {
                fix_common_syllable_errors(c)
            } else {
                c
            };
            syllable_builder = syllable_builder
                .insert(Bopomofo::try_from(c)?)
                .with_context(|| format!("failed to parse syllables {}", syllable_str))?;
        }
        syllables.push(syllable_builder.build());
    }

    Ok((syllables, phrase, freq))
}

fn fix_common_syllable_errors(c: char) -> char {
    match c {
        '一' => 'ㄧ',
        '丫' => 'ㄚ',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use chewing::syl;
    use chewing::zhuyin::Bopomofo::*;

    use super::parse_line;

    #[test]
    fn parse_ssv() {
        let line = "鑰匙 668 ㄧㄠˋ ㄔˊ # not official";
        if let Ok((syllables, phrase, freq)) = parse_line(' ', &line, false) {
            assert_eq!(syllables, vec![syl![I, AU, TONE4], syl![CH, TONE2]]);
            assert_eq!("鑰匙", phrase);
            assert_eq!(668, freq);
        } else {
            panic!()
        }
    }

    #[test]
    fn parse_ssv_multiple_whitespace() {
        let line = "鑰匙     668 ㄧㄠˋ ㄔˊ # not official";
        if let Ok((syllables, phrase, freq)) = parse_line(' ', &line, false) {
            assert_eq!(syllables, vec![syl![I, AU, TONE4], syl![CH, TONE2]]);
            assert_eq!("鑰匙", phrase);
            assert_eq!(668, freq);
        } else {
            panic!()
        }
    }

    #[test]
    fn parse_ssv_syllable_errors() {
        let line = "地永天長 50 ㄉ一ˋ ㄩㄥˇ ㄊ一ㄢ ㄔ丫ˊ";
        if let Ok((syllables, phrase, freq)) = parse_line(' ', &line, true) {
            assert_eq!(
                syllables,
                vec![
                    syl![D, I, TONE4],
                    syl![IU, ENG, TONE3],
                    syl![T, I, AN],
                    syl![CH, A, TONE2]
                ]
            );
            assert_eq!("地永天長", phrase);
            assert_eq!(50, freq);
        } else {
            panic!()
        }
    }

    #[test]
    fn parse_csv() {
        let line = "鑰匙,668,ㄧㄠˋ ㄔˊ # not official";
        if let Ok((syllables, phrase, freq)) = parse_line(',', &line, false) {
            assert_eq!(syllables, vec![syl![I, AU, TONE4], syl![CH, TONE2]]);
            assert_eq!("鑰匙", phrase);
            assert_eq!(668, freq);
        } else {
            panic!()
        }
    }

    #[test]
    fn parse_csv_quoted() {
        let line = "\"鑰匙\",668,\"ㄧㄠˋ ㄔˊ # not official\"";
        if let Ok((syllables, phrase, freq)) = parse_line(',', &line, false) {
            assert_eq!(syllables, vec![syl![I, AU, TONE4], syl![CH, TONE2]]);
            assert_eq!("鑰匙", phrase);
            assert_eq!(668, freq);
        } else {
            panic!()
        }
    }
}
