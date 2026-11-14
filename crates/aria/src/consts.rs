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

Shortcuts (press ? inside Aria for the full list):
        Space                     play/pause
        Left / Right              seek 5 seconds
        Ctrl + Left / Right       previous / next track
        - / +                     volume
        s / r                     shuffle the queue / repeat off, all, one
        F1 F2 F3 F4               Home / Search / Now playing / Playlist
        F6 / F7 / F8              right panel / lyrics / sign in
        Enter, double click       play or open
        m, right click            menu: play next, add to queue, copy link...
        Del                       remove from the queue (Now playing)
        n / Del / F5 (Home)       create / delete a playlist / sync with the account
        Tab / Shift+Tab           search filters (All, Songs, Videos, Playlists)
        /                         back to the search box
        Esc                       back
        Ctrl + C                  quit
"#;
