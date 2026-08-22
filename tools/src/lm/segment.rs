use std::{
    borrow::Cow,
    fs::File,
    io::{BufRead, stdin},
    path::Path,
};

use anyhow::Result;
use chewing::{
    conversion::{Decoder, WordLattice},
    dictionary::StringTable,
    lm::StaticLm,
    model::{Surface, WordId},
    user::{HistoryFreq, UserFreq},
};

pub(crate) fn segment(static_lm: &Path, words: &Path) -> Result<()> {
    let lm = StaticLm::from_reader(File::open(static_lm)?)?;
    let words_table = StringTable::open(words)?;
    let words_map = words_table.to_map();

    let handle = stdin().lock();
    let decoder = Decoder {
        user_freq: UserFreq::new(),
        history_freq: HistoryFreq::new(),
        lm,
    };

    for io in handle.lines() {
        let line = io?;
        let lattice = WordLattice::from_str(&line, |s| words_map.get(s).map(|wid| WordId(*wid)));
        let hypotheses = decoder.decode(&lattice);
        for hyp in hypotheses {
            for (i, segstr) in hyp
                .edges
                .iter()
                .map(|e| match e.surface {
                    Surface::Word(wid) => Cow::Borrowed(words_table.get(wid.0).unwrap_or("<unk>")),
                    Surface::Char(c) => Cow::Owned(c.to_string()),
                    Surface::None => Cow::Borrowed("<unk>"),
                })
                .enumerate()
            {
                if i == 0 {
                    print!("{}", segstr);
                } else {
                    print!(" {}", segstr);
                }
            }
            println!("");
            // TODO: add option to output all hypotheses
            // break;
        }
    }

    Ok(())
}
