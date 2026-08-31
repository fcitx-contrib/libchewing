use std::{
    io::{BufRead, Write, stdin, stdout},
    thread,
};

use anyhow::Result;

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
                loop {
                    let Ok(line) = work_recv.recv() else {
                        return;
                    };
                    result_send
                        .send(clean_line(&line))
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

fn clean_line(line: &str) -> String {
    let line = zhconv::zhconv(&line, zhconv::Variant::ZhTW);
    let line = line.replace("瘔", "苦");
    line
}
