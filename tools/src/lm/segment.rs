use std::{
    fs::File,
    io::{BufRead, Write, stdin, stdout},
    path::Path,
    thread,
};

use anyhow::Result;
use chewing::{
    conversion::{Decoder, Lattice},
    dictionary::StringTable,
    lm::{LoadMode, StaticLm},
    model::Candidate,
};

pub(crate) fn segment(static_lm: &Path, words: &Path) -> Result<()> {
    let lm = StaticLm::from_reader(File::open(static_lm)?, LoadMode::Eager)?;
    let string_table = StringTable::open_txt(words)?;

    let stdin = stdin().lock();

    let decoder = Decoder {
        lm,
        lambda: Decoder::LAMBDA,
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
                    let lattice = Lattice::from_str(&line, |s| string_table.get_wid(s));
                    let hypotheses = decoder.decoden(lattice, 1);
                    for hyp in hypotheses {
                        let mut segmented = String::new();
                        for (i, segstr) in hyp
                            .candidates
                            .iter()
                            .map(|cand| match cand {
                                Candidate::Word { wid, .. } => {
                                    string_table.get_text(*wid).unwrap_or("<unk>".into())
                                }
                                Candidate::Grapheme(c) => c.to_string(),
                                Candidate::None => "<unk>".into(),
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
