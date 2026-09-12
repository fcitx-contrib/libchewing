use std::{
    fs::File,
    io::{BufWriter, Write, stdout},
    path::PathBuf,
};

use anyhow::Result;
use chewing::dictionary::Trie;

use crate::flags;

pub(crate) fn run(args: flags::Dump) -> Result<()> {
    let dict = Trie::open(&args.path)?;
    let sink: Box<dyn Write> = if let Some(output) = args.output {
        if output == PathBuf::from("-") {
            Box::new(stdout())
        } else {
            Box::new(File::create(output)?)
        }
    } else {
        Box::new(stdout())
    };
    let sink = BufWriter::new(sink);
    dump_dict_csv(sink, &dict)?;
    Ok(())
}

fn dump_dict_csv(mut sink: BufWriter<Box<dyn Write>>, dict: &Trie) -> Result<()> {
    for (syllables, phrase) in dict.entries() {
        writeln!(
            sink,
            "{},{},{}",
            phrase,
            phrase.freq(),
            syllables
                .iter()
                .map(|syl| syl.to_string())
                .collect::<Vec<_>>()
                .join("　")
        )?;
    }
    Ok(())
}
