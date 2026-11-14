//! What Aria remembers between runs: the queue, the volume and the layout.

use serde::{Deserialize, Serialize};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{consts::CACHE_DIR, systems::player::Repeat};

#[derive(Serialize, Deserialize)]
pub struct SavedState {
    #[serde(default)]
    pub queue: Vec<YoutubeMusicVideoRef>,
    #[serde(default)]
    pub current: usize,
    #[serde(default)]
    pub volume: Option<i32>,
    #[serde(default = "yes")]
    pub panel_open: bool,
    #[serde(default)]
    pub lyrics_tab: bool,
    #[serde(default)]
    pub repeat: Repeat,
    /// Section that was open, as its `Screens` number.
    #[serde(default = "home")]
    pub screen: u8,
    /// Seconds into the current track.
    #[serde(default)]
    pub position: u64,
    /// The playlist that was open in the Playlist section.
    #[serde(default)]
    pub viewed: Option<Viewed>,
}

#[derive(Serialize, Deserialize)]
pub struct Viewed {
    pub name: String,
    #[serde(default)]
    pub browse_id: String,
    pub videos: Vec<YoutubeMusicVideoRef>,
}

fn home() -> u8 {
    1
}

fn yes() -> bool {
    true
}

impl Default for SavedState {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            current: 0,
            volume: None,
            panel_open: true,
            lyrics_tab: false,
            repeat: Repeat::Off,
            screen: home(),
            position: 0,
            viewed: None,
        }
    }
}

fn path() -> std::path::PathBuf {
    CACHE_DIR.join("state.json")
}

pub fn load() -> SavedState {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the state through a temporary file, so a crash never leaves half a file.
pub fn save(state: &SavedState) {
    let Ok(json) = serde_json::to_vec(state) else {
        return;
    };
    let tmp = path().with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(tmp, path());
    }
}
