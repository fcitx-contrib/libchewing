use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct ChewingCli {
    #[command(subcommand)]
    pub(crate) subcommand: ChewingCliCmd,
}

#[derive(Subcommand)]
pub(crate) enum ChewingCliCmd {
    /// Display information about the file
    Info(Info),
    /// List detected system and user file
    List(List),
    /// Convert a binary format to its text format
    Dump(Dump),
    /// Subcommands for dictionary index manipulation
    #[command(subcommand)]
    Index(Index),
    /// Subcommands for language model manipulation
    #[command(subcommand)]
    Lm(Lm),
}

#[derive(Subcommand)]
pub(crate) enum Index {
    /// Create dictionary index file
    CreateDict(IndexCreateDict),
    /// Create string table index file
    CreateStringTable(IndexCreateStringTable),
}

#[derive(Args)]
pub(crate) struct IndexCreateDict {
    /// Path to the dictionary source file (tsi.csv)
    pub(crate) tsi_csv: PathBuf,
    /// Path to the words list (static_words.txt)
    pub(crate) words_txt: PathBuf,
    /// Path to the output file (static_dict.bin)
    pub(crate) dict_output: PathBuf,
}

#[derive(Args)]
pub(crate) struct IndexCreateStringTable {
    /// Path to the words list (static_words.txt)
    pub(crate) words_txt: PathBuf,
    /// Path to the output file (static_words.bin)
    pub(crate) words_output: PathBuf,
}

#[derive(Subcommand)]
pub(crate) enum Lm {
    /// Clean-up input to prepare for training
    Clean,
    /// Create binary language model from ARPA file
    Compile(LmCompile),
    /// Segment a string
    Segment(LmSegment),
    /// Learn unigram and bigram language model and output ARPA file
    Learn(LmLearn),
}

#[derive(Args)]
pub(crate) struct LmCompile {
    /// Path to the language model ARPA file (static_lm.arpa)
    pub(crate) lm_arpa: PathBuf,
    /// Path to the words list (static_words.txt)
    pub(crate) words_txt: PathBuf,
    /// Path to the output file (static_lm.bin)
    pub(crate) output: PathBuf,
}

#[derive(Args)]
pub(crate) struct LmSegment {
    /// Path to the language model file (static_lm.bin)
    pub(crate) static_lm: PathBuf,
    /// Path to the words list (static_words.txt)
    pub(crate) words_txt: PathBuf,
}

#[derive(Args)]
pub(crate) struct LmLearn {
    /// Path to the words list (static_words.txt)
    pub(crate) words_txt: PathBuf,
    /// Path to the output ARPA file (static_lm.arpa)
    pub(crate) output: PathBuf,
}

#[derive(Args)]
pub(crate) struct Info {
    /// Location of the file
    #[arg(short, long)]
    pub(crate) path: PathBuf,
    /// Output in JSON format
    #[arg(short, long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct List {
    /// Display information of detected user dictionary
    #[arg(short, long)]
    pub(crate) user: bool,
    /// Display information of detected system dictionary
    #[arg(short, long)]
    pub(crate) system: bool,
    /// Output in JSON format
    #[arg(short, long)]
    pub(crate) json: bool,
}

#[derive(Args)]
pub(crate) struct Dump {
    /// Location of the dictionary file
    pub(crate) path: PathBuf,
    /// Location of the output file
    ///
    /// If OUTPUT equals to `-` then standard output will be used.
    pub(crate) output: Option<PathBuf>,
}
