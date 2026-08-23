use anyhow::Result;
use clap::Parser;

mod dump;
mod flags;
mod index;
mod info;
mod init_database;
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
        flags::ChewingCliCmd::InitDatabase(args) => init_database::run(args)?,
        flags::ChewingCliCmd::Info(args) => info::run(args)?,
        flags::ChewingCliCmd::Dump(args) => dump::run(args)?,
        flags::ChewingCliCmd::Index(sub) => match sub {
            flags::Index::Create(args) => {
                index::create_index(&args.tsi_csv, &args.words_txt, &args.output)?
            }
        },
        flags::ChewingCliCmd::Lm(sub) => match sub {
            flags::Lm::Compile(args) => {
                lm::compile_lm(&args.lm_arpa, &args.words_txt, &args.output)?;
            }
            flags::Lm::Segment(args) => {
                lm::segment(&args.static_lm, &args.words_txt)?;
            }
            flags::Lm::Learn(args) => {
                lm::learn_lm(&args.words_txt, &args.output)?;
            }
        },
    }
    Ok(())
}
