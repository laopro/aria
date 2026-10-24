use common_structs::MusicDownloadStatus;
use ratatui::style::{Modifier, Style};

pub trait MusicDownloadStatusExt {
    fn style(&self, playing: Option<bool>) -> Style;
}

use crate::consts::CONFIG;

impl MusicDownloadStatusExt for MusicDownloadStatus {
    fn style(&self, playing: Option<bool>) -> Style {
        let k = match self {
            Self::NotDownloaded => CONFIG.player.text_waiting_style,
            Self::Downloaded => {
                if let Some(e) = playing {
                    if e {
                        CONFIG.player.text_playing_style
                    } else {
                        CONFIG.player.text_paused_style
                    }
                } else {
                    CONFIG.player.text_next_style
                }
            }
            Self::Downloading(_) => CONFIG.player.text_downloading_style,
            Self::DownloadFailed => CONFIG.player.text_error_style,
        };
        if playing.is_some() {
            k.add_modifier(Modifier::BOLD)
        } else {
            k
        }
    }
}

