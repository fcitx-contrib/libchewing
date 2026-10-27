use std::{
    io::{BufRead, Write, stdin, stdout},
    thread,
};

use aho_corasick::AhoCorasick;
use anyhow::Result;

const PATTERNS: &[&str] = &[
    "麽", "瘔", "上麵", "前麵", "下麵", "裡麵", "裏麵", "對麵", "後麵", "市麵", "麵子",
];
const REPLACE: &[&str] = &[
    "麼", "苦", "上面", "前面", "下面", "裡面", "裡面", "對面", "後面", "市面", "面子",
];

pub(crate) fn clean() -> Result<()> {
    let stdin = stdin().lock();

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
                let ac = AhoCorasick::new(PATTERNS).unwrap();
                loop {
                    let Ok(line) = work_recv.recv() else {
                        return;
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    result_send
                        .send(ac.replace_all(&line, REPLACE))
                        .expect("unable to send output to stdout");
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
