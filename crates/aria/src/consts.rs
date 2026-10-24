use std::path::PathBuf;

use log::warn;
use once_cell::sync::Lazy;

use crate::{config, utils::get_project_dirs};

pub static CACHE_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let pdir = get_project_dirs();
    if let Some(dir) = pdir {
        return dir.cache_dir().to_path_buf();
    };
    warn!("Failed to get cache dir! Defaulting to './data'");
    PathBuf::from("./data")
});

pub static CONFIG: Lazy<config::Config> = Lazy::new(config::Config::new);

pub const INTRODUCTION: &str = r#"Usage: aria [options]

Aria is a lightweight TUI YouTube Music player written in Rust.
It works without an account: search and play public music right away.
Sign in (optional) to see your library and playlists.

Options:
        -h or --help        Show this menu
        --login [browser]   Sign in (optional) via your browser or a login window
        --files             Show the location of the Aria files
        --fix-db            Fix the local database
        --clear-cache       Erase all the files in cache
        --update            Update Aria to the latest release (Windows/Linux)

Shortcuts:
        Use your mouse to click in lists if your terminal has mouse support
        Space                     play/pause
        Enter                     select a playlist or a track
        f                         search
        s                         shuffle
        r                         remove a track from the queue
        Arrow Right or >          skip 5 seconds
        Arrow Left or <           go back 5 seconds
        CTRL + Arrow Right (>)    go to the next song
        CTRL + Arrow Left  (<)    go to the previous song
        +                         volume up
        -                         volume down
        Arrow down                scroll down
        Arrow up                  scroll up
        ESC                       exit the current menu
        CTRL + C or CTRL + D      quit
"#;
