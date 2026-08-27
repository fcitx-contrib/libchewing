use std::{
    borrow::Cow,
    fs::File,
    io::{BufRead, Write, stdin, stdout},
    path::Path,
    thread,
};

use anyhow::Result;
use chewing::{
    conversion::{Decoder, WordLattice},
    dictionary::StringTable,
    lm::{LoadMode, StaticLm},
    model::{Surface, WordId},
    user::HistoryFreq,
};
use fxhash::FxHashMap;

pub(crate) fn segment(static_lm: &Path, words: &Path) -> Result<()> {
    let lm = StaticLm::from_reader(File::open(static_lm)?, LoadMode::Eager)?;
    let words_table = StringTable::open(words)?;
    let words_map: FxHashMap<Cow<'_, str>, u32> = words_table.iter().collect();

    let stdin = stdin().lock();

    let decoder = Decoder {
        history_freq: HistoryFreq::new(),
        lm,
    };

    let n_threads = thread::available_parallelism()?.get();
    let (work_send, work_recv) = crossbeam_channel::bounded::<String>(n_threads * 2);
    let (result_send, result_recv) = crossbeam_channel::bounded::<String>(4096);

    let output_handle = thread::spawn(move || {
        let mut stdout = stdout().lock();
        loop {
            let Ok(res) = result_recv.recv() else {
                return;
            };
            writeln!(stdout, "{}", res).expect("unable to write to stdout");
        }
    });
    thread::scope(|s| {
        for _ in 0..n_threads {
            s.spawn(|| {
                loop {
                    let Ok(line) = work_recv.recv() else {
                        return;
                    };
                    let lattice =
                        WordLattice::from_str(&line, |s| words_map.get(s).map(|wid| WordId(*wid)));
                    let hypotheses = decoder.decoden(&lattice, 1);
                    for hyp in hypotheses {
                        let mut segmented = String::new();
                        for (i, segstr) in hyp
                            .edges
                            .iter()
                            .map(|e| match e.surface {
                                Surface::Word(wid) => {
                                    words_table.get(wid).unwrap_or("<unk>".into())
                                }
                                Surface::Char(c) => Cow::Owned(c.to_string()),
                                Surface::None => Cow::Borrowed("<unk>"),
                            })
                            .enumerate()
                        {
                            if i == 0 {
                                segmented.push_str(&segstr);
                            } else {
                                segmented.push(' ');
                                segmented.push_str(&segstr);
                            }
                        }
                        result_send
                            .send(segmented)
                            .expect("unable to send output to stdout");
                    }
                }
            });
        }
        for io in stdin.lines() {
            if let Ok(line) = io {
                work_send
                    .send(line)
                    .expect("unable to send input to worker thread");
            }
        }
        drop(work_send);
    });
    drop(result_send);
    output_handle.join().expect("failed to write to stdout");

    Ok(())
}
