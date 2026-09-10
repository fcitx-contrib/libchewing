use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, stdin},
    path::Path,
};

use anyhow::{Context, Result};

pub(crate) fn prepare_eval(tsi_csv: &Path, rare_csv: &Path) -> Result<()> {
    let mut rare_map = HashMap::new();
    let mut bopomofo_map = HashMap::new();

    let rare = BufReader::new(File::open(rare_csv)?);
    for io in rare.lines() {
        let line = io?;
        if line.starts_with('#') {
            continue;
        }
        if let Some((word, bopomofo)) = line.split_once(',') {
            rare_map
                .entry(word.trim().to_owned())
                .and_modify(|e: &mut Vec<_>| e.push(bopomofo.trim().to_owned()))
                .or_insert(vec![bopomofo.trim().to_owned()]);
        }
    }

    let tsi = BufReader::new(File::open(tsi_csv)?);
    for io in tsi.lines() {
        let line = io?;
        if line.starts_with('#') {
            continue;
        }
        if let Some((word, bopomofo)) = line.split_once(',') {
            if let Some(es) = rare_map.get(word.trim())
                && es.iter().any(|s| s == bopomofo.trim())
            {
                continue;
            }
            bopomofo_map
                .entry(word.trim().to_owned())
                .or_insert(bopomofo.trim().to_owned());
        }
    }

    let lock = stdin().lock();
    for io in lock.lines() {
        let line = io?;
        for part in line.split_whitespace() {
            if part == "，" {
                print!("{} ", part);
            } else {
                let bopomofo = bopomofo_map.get(part).context("load bopomofo")?;
                print!("{} ", bopomofo);
            }
        }
        println!("");
        for part in line.split_whitespace() {
            print!("{}", part.trim());
        }
        println!("");
    }
    Ok(())
}
