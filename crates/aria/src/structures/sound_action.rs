use common_structs::MusicDownloadStatus;
use download_manager::{DownloadManagerMessage, MessageHandler};
use flume::Sender;
use std::{sync::Arc, time::Duration};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    errors::handle_error_option,
    systems::{player::PlayerState, DOWNLOAD_MANAGER},
    DATABASE,
};

/// Actions that can be sent to the player from other services
#[derive(Debug, Clone)]
pub enum SoundAction {
    /// Set the volume of the player to the given value
    SetVolume(f32),
    Cleanup,
    PlayPause,
    RestartPlayer,
    Plus,
    Minus,
    /// Seek to a specific time in the current song in seconds
    SeekTo(Duration),
    Previous(usize),
    Forward,
    Backward,
    Next(usize),
    AddVideosToQueue(Vec<YoutubeMusicVideoRef>),
    AddVideoUnary(YoutubeMusicVideoRef),
    PlayNow(YoutubeMusicVideoRef),
    /// Puts a track right after the current one.
    PlayNext(YoutubeMusicVideoRef),
    /// Removes the track at this queue index (the queue only, never the disk).
    RemoveAt(usize),
    /// Shuffles the tracks after the current one.
    ShuffleRest,
    /// Off -> all -> one -> off.
    CycleRepeat,
    /// Restores a saved queue without starting playback.
    RestoreQueue(Vec<YoutubeMusicVideoRef>, usize),
    ReplaceQueue(Vec<YoutubeMusicVideoRef>),
    VideoStatusUpdate(String, MusicDownloadStatus),
}

impl SoundAction {
    fn insert(player: &mut PlayerState, video: String, status: MusicDownloadStatus) {
        if matches!(
            player.music_status.get(&video),
            Some(&MusicDownloadStatus::DownloadFailed)
        ) {
            DOWNLOAD_MANAGER.remove_from_in_downloads(&video);
        }
        if matches!(
            player.music_status.get(&video),
            Some(&MusicDownloadStatus::Downloading(_) | &MusicDownloadStatus::Downloaded)
        ) && status == MusicDownloadStatus::NotDownloaded
        {
            return;
        }
        player.music_status.insert(video, status);
    }

    pub fn apply_sound_action(self, player: &mut PlayerState) {
        match self {
            Self::SetVolume(volume) => player.sink.set_volume((volume * 100.) as i32),
            Self::SeekTo(time) => player.sink.seek_to(time),
            Self::Backward => player.sink.seek_bw(),
            Self::Forward => player.sink.seek_fw(),
            Self::PlayPause => {
                // A restored queue loads on the first play, not at startup.
                if std::mem::take(&mut player.start_paused) {
                    return;
                }
                player.sink.toggle_playback()
            }
            Self::Cleanup => {
                player.start_paused = false;
                player.resume_at = None;
                player.list.clear();
                player.current = 0;
                player.music_status.clear();
                player.sink.stop();
            }
            Self::Plus => player.sink.volume_up(),
            Self::Minus => player.sink.volume_down(),
            Self::Next(a) => {
                player.start_paused = false;
                player.resume_at = None;
                player.sink.stop();

                player.set_relative_current(a as _);
            }
            Self::VideoStatusUpdate(video, status) => {
                player.music_status.insert(video, status);
            }
            Self::AddVideosToQueue(video) => {
                let db = DATABASE.read().unwrap();
                for v in video {
                    if player.list.iter().any(|x| x.video_id == v.video_id) {
                        continue;
                    }
                    Self::insert(
                        player,
                        v.video_id.clone(),
                        if db.iter().any(|e| e.video_id == v.video_id) {
                            MusicDownloadStatus::Downloaded
                        } else {
                            MusicDownloadStatus::NotDownloaded
                        },
                    );
                    player.list.push(v)
                }
            }
            Self::Previous(a) => {
                player.start_paused = false;
                player.resume_at = None;
                player.set_relative_current(-(a as isize));
                player.sink.stop();
            }
            Self::RestartPlayer => {
                player.sink =
                    handle_error_option(&player.updater, "update player", player.sink.update())
                        .unwrap();
                if let Some(e) = player.current().cloned() {
                    Self::AddVideoUnary(e).apply_sound_action(player);
                }
            }
            Self::AddVideoUnary(video) => {
                Self::insert(
                    player,
                    video.video_id.clone(),
                    if DATABASE
                        .read()
                        .unwrap()
                        .iter()
                        .any(|e| e.video_id == video.video_id)
                    {
                        MusicDownloadStatus::Downloaded
                    } else {
                        MusicDownloadStatus::NotDownloaded
                    },
                );
                if player.list.is_empty() {
                    player.list.push(video);
                } else {
                    player.list.insert(player.current + 1, video);
                }
            }
            Self::PlayNext(video) => {
                if player.current().map(|c| c.video_id == video.video_id).unwrap_or(false) {
                    return;
                }
                // Move a later copy instead of queueing the track twice.
                if let Some(position) = player
                    .list
                    .iter()
                    .position(|v| v.video_id == video.video_id)
                {
                    if position > player.current {
                        player.list.remove(position);
                    }
                }
                Self::AddVideoUnary(video).apply_sound_action(player);
            }
            Self::RemoveAt(index) => {
                if index >= player.list.len() {
                    return;
                }
                let video = player.list.remove(index);
                if !player.list.iter().any(|v| v.video_id == video.video_id) {
                    player.music_status.remove(&video.video_id);
                }
                match index.cmp(&player.current) {
                    std::cmp::Ordering::Less => player.current -= 1,
                    std::cmp::Ordering::Equal => {
                        // The playing track went away: play what took its place.
                        player.sink.stop();
                        player.rtcurrent = None;
                    }
                    std::cmp::Ordering::Greater => {}
                }
            }
            Self::ShuffleRest => {
                use rand::seq::SliceRandom;
                let start = (player.current + 1).min(player.list.len());
                player.list[start..].shuffle(&mut rand::thread_rng());
            }
            Self::CycleRepeat => player.repeat = player.repeat.next(),
            Self::RestoreQueue(videos, current) => {
                Self::Cleanup.apply_sound_action(player);
                Self::AddVideosToQueue(videos).apply_sound_action(player);
                player.current = current.min(player.list.len().saturating_sub(1));
                player.start_paused = true;
            }
            Self::PlayNow(video) => {
                Self::Cleanup.apply_sound_action(player);
                Self::AddVideoUnary(video).apply_sound_action(player);
                Self::Next(0).apply_sound_action(player);
            }
            Self::ReplaceQueue(videos) => {
                // Start a fresh queue from the first item.
                Self::Cleanup.apply_sound_action(player);
                Self::AddVideosToQueue(videos).apply_sound_action(player);
                Self::Next(0).apply_sound_action(player);
            }
        }
    }
}

pub fn download_manager_handler(sender: Sender<SoundAction>) -> MessageHandler {
    Arc::new(move |message| match message {
        DownloadManagerMessage::VideoStatusUpdate(video, status) => {
            sender
                .send(SoundAction::VideoStatusUpdate(video, status))
                .unwrap();
        }
    })
}
