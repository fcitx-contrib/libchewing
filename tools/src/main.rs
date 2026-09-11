use anyhow::Result;
use clap::Parser;

use crate::lm::PruningConfig;

mod dump;
mod flags;
mod index;
mod info;
mod list;
mod lm;

fn main() -> Result<()> {
    env_logger::init();
    #[cfg(feature = "mangen")]
    {
        use clap::CommandFactory;
        if let Ok(_) = std::env::var("UPDATE_MANPAGE") {
            clap_mangen::generate_to(
                flags::ChewingCli::command(),
                std::env::args().nth(1).unwrap(),
            )?;
            return Ok(());
        }
    }
    let cli = flags::ChewingCli::parse();
    match cli.subcommand {
        flags::ChewingCliCmd::Info(args) => info::run(args)?,
        flags::ChewingCliCmd::List(args) => list::run(args)?,
        flags::ChewingCliCmd::Dump(args) => dump::run(args)?,
        flags::ChewingCliCmd::Index(sub) => match sub {
            flags::Index::CreateDict(args) => {
                index::create_index_dict(&args.tsi_csv, &args.words_txt, &args.dict_output)?
            }
            flags::Index::CreateStringTable(args) => {
                index::create_string_table(&args.words_txt, &args.words_output)?
            }
        },
        flags::ChewingCliCmd::Lm(sub) => match sub {
            flags::Lm::PrepareEval(args) => {
                lm::prepare_eval(&args.tsi_csv, &args.rare_csv)?;
            }
            flags::Lm::Eval(args) => {
                lm::eval(&args.search_path, &args.model_bin, args.alpha, args.verbose)?;
            }
            flags::Lm::Compile(args) => {
                lm::compile_lm(&args.lm_arpa, &args.words_txt, &args.output)?;
            }
            flags::Lm::Segment(args) => {
                lm::segment(&args.static_lm, &args.words_txt)?;
            }
            flags::Lm::Learn(args) => {
                let config = PruningConfig {
                    min_count: args.min_count,
                    keep_fraction: args.keep_fraction,
                };
                lm::learn_lm(&args.words_txt, &args.output, &config)?;
            }
        },
    }
    Ok(())
}
