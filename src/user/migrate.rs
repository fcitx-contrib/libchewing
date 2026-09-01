use std::{
    fmt::Display,
    fs::{self, File},
    io::Write,
    path::Path,
};

#[cfg(target_family = "unix")]
use std::os::unix::fs::OpenOptionsExt;

use log::info;
use scoped_error::{expect_error, impl_context_error};

use crate::{
    dictionary::{Dictionary, Trie},
    zhuyin::Syllable,
};

pub fn should_migrate_v3(base_path: &Path) -> bool {
    let chewing_dat_path = base_path.join("chewing.dat");
    let v4_path = base_path.join("v4");

    chewing_dat_path.exists() && !v4_path.exists()
}

pub fn migrate_v3_to_v4(base_path: &Path) -> Result<(), MigrateV4Error> {
    expect_error("Unable to migrate v3 user data to v4 format", || {
        let v4_path = base_path.join("v4");
        fs::create_dir_all(&v4_path)?;

        let chewing_dat_path = base_path.join("chewing.dat");
        let deleted_dat_path = base_path.join("chewing-deleted.dat");

        let chewing_dat = Trie::open(&chewing_dat_path)?;
        let deleted_dat = Trie::open(&deleted_dat_path)?;

        let user_dict_path = v4_path.join("user_dict.csv");

        info!("Migrate {} to v4 format", chewing_dat_path.display());

        let mut file_options = File::options();
        file_options.create(true).write(true);

        #[cfg(target_family = "unix")]
        {
            file_options.mode(0o600);
        }

        let mut user_dict = file_options.open(&user_dict_path)?;

        for (syllables, phrase) in chewing_dat.entries() {
            writeln!(
                user_dict,
                "{},{},{}",
                phrase,
                display_syllables(&syllables),
                phrase.freq()
            )?;
        }

        for (syllables, phrase) in deleted_dat.entries() {
            writeln!(
                user_dict,
                "{},{},-{}",
                phrase,
                display_syllables(&syllables),
                phrase.freq()
            )?;
        }

        user_dict.sync_all()?;

        Ok(())
    })
}

fn display_syllables(syllables: &[Syllable]) -> impl Display {
    syllables
        .iter()
        .map(|syl| syl.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

impl_context_error!(pub MigrateV4Error);
