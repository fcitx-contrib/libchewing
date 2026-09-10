use std::path::PathBuf;

use crate::flags::List;

use chewing::path::SearchPath;
use scoped_error::{Error, expect_error};

pub(crate) fn run(args: List) -> Result<(), Error> {
    expect_error("Failed to list libchewing files", || {
        let sp = SearchPath::from_env();
        let list_all = !args.user && !args.system;
        let mut reports = vec![];

        if args.user || list_all {
            if let Some(path) = sp.find_user_file("user_dict.csv") {
                reports.push(Report {
                    typ: "UserDict",
                    path,
                });
            }
            if let Some(path) = sp.find_user_file("history_dict.bin") {
                reports.push(Report {
                    typ: "HistoryDict",
                    path,
                });
            }
        }
        if args.system || list_all {
            if let Some(path) = sp.find_file("static_words.bin") {
                reports.push(Report {
                    typ: "StringTable",
                    path,
                });
            }
            if let Some(path) = sp.find_file("static_dict.bin") {
                reports.push(Report {
                    typ: "StaticDict",
                    path,
                });
            }
            if let Some(path) = sp.find_file("rare_dict.bin") {
                reports.push(Report {
                    typ: "StaticDict",
                    path,
                });
            }
            if let Some(path) = sp.find_file("static_lm.bin") {
                reports.push(Report {
                    typ: "StaticLm",
                    path,
                });
            }
            if let Some(path) = sp.find_file("swkb.dat") {
                reports.push(Report {
                    typ: "Abbrev",
                    path,
                });
            }
            if let Some(path) = sp.find_file("symbols.dat") {
                reports.push(Report {
                    typ: "SymbolSelector",
                    path,
                });
            }
        }

        let end = reports.len();
        if args.json {
            println!("[");
            for i in 0..end {
                print!(
                    r#"    {{ "type": "{}", "path": "{}" }}"#,
                    reports[i].typ,
                    reports[i].path.display()
                );
                if i == end - 1 {
                    println!("");
                } else {
                    println!(",");
                }
            }
            println!("]");
        } else {
            for i in 0..end {
                println!("{:>15}: {}", reports[i].typ, reports[i].path.display());
            }
        }

        Ok(())
    })
}

struct Report {
    typ: &'static str,
    path: PathBuf,
}
