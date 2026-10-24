use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::Ordering,
};

use common_structs::MusicDownloadStatus;
use flume::{unbounded, Receiver, Sender};
use log::error;
use player::{PlayError, Player, PlayerOptions};

use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    consts::{CACHE_DIR, CONFIG},
    errors::{handle_error, handle_error_option},
    structures::{media::Media, sound_action::SoundAction},
    systems::DOWNLOAD_MANAGER,
    term::{list_selector::ListSelector, playlist::PLAYER_RUNNING, ManagerMessage, Screens},
    DATABASE,
};

pub struct PlayerState {
    pub goto: Screens,
    pub list: Vec<YoutubeMusicVideoRef>,
    pub current: usize,
    pub rtcurrent: Option<YoutubeMusicVideoRef>,
    pub music_status: HashMap<String, MusicDownloadStatus>,
    pub list_selector: ListSelector,
    pub controls: Media,
    pub sink: Player,
    pub updater: Sender<ManagerMessage>,
    pub soundaction_sender: Sender<SoundAction>,
    pub soundaction_receiver: Receiver<SoundAction>,
    pub stream_error_receiver: Receiver<PlayError>,
    last_download_list: Vec<String>,
    stream_tx: Sender<(String, Result<Vec<u8>, String>)>,
    stream_rx: Receiver<(String, Result<Vec<u8>, String>)>,
    pending_stream: Option<String>,
    autoplay_requested: Option<String>,
}

/// How many upcoming tracks to pre-download to disk.
/// `0` disables disk caching: every track is fetched into RAM on demand.
const PREFETCH_AHEAD: usize = 0;

impl PlayerState {
    fn new(
        soundaction_sender: Sender<SoundAction>,
        soundaction_receiver: Receiver<SoundAction>,
        updater: Sender<ManagerMessage>,
    ) -> Self {
        let (stream_error_sender, stream_error_receiver) = unbounded::<PlayError>();
        let (stream_tx, stream_rx) = unbounded::<(String, Result<Vec<u8>, String>)>();
        let sink = match handle_error_option(
            &updater,
            "player creation error",
            Player::new(
                stream_error_sender,
                PlayerOptions::new(CONFIG.player.initial_volume),
            ),
        ) {
            Some(sink) => sink,
            None => {
                eprintln!("No se pudo abrir un dispositivo de audio.");
                std::process::exit(1);
            }
        };
        Self {
            controls: Media::new(updater.clone(), soundaction_sender.clone()),
            soundaction_receiver,
            list_selector: ListSelector::default(),
            music_status: HashMap::new(),
            updater,
            stream_error_receiver,
            soundaction_sender,
            sink,
            goto: Screens::Playlist,
            list: Vec::new(),
            current: 0,
            rtcurrent: None,
            last_download_list: Vec::new(),
            stream_tx,
            stream_rx,
            pending_stream: None,
            autoplay_requested: None,
        }
    }

    pub fn current(&self) -> Option<&YoutubeMusicVideoRef> {
        self.relative_current(0)
    }

    /// Total length of the current track, from the decoder or the API metadata.
    pub fn total_seconds(&self) -> Option<u64> {
        self.sink
            .duration()
            .map(|d| d as u64)
            .filter(|d| *d > 0)
            .or_else(|| self.current().and_then(|v| parse_duration(&v.duration)))
    }

    /// Snapshot used by the global player bar.
    pub fn now_playing(&self) -> Option<crate::term::NowPlaying> {
        let video = self.current()?;
        let elapsed = self.sink.elapsed().as_secs();
        let total = self.total_seconds().unwrap_or(0);
        let ratio = if total > 0 {
            (elapsed as f64 / total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Some(crate::term::NowPlaying {
            title: video.title.clone(),
            artist: video.author.clone(),
            elapsed,
            total,
            ratio,
            paused: self.sink.is_paused(),
        })
    }

    pub fn relative_current(&self, n: isize) -> Option<&YoutubeMusicVideoRef> {
        self.list.get(self.current.saturating_add_signed(n))
    }

    pub fn set_relative_current(&mut self, n: isize) {
        self.current = self.current.saturating_add_signed(n);
    }

    pub fn is_current_download_failed(&self) -> bool {
        self.current()
            .as_ref()
            .map(|x| {
                self.music_status.get(&x.video_id) == Some(&MusicDownloadStatus::DownloadFailed)
            })
            .unwrap_or(false)
    }

    pub fn is_current_downloaded(&self) -> bool {
        self.current()
            .as_ref()
            .map(|x| self.music_status.get(&x.video_id) == Some(&MusicDownloadStatus::Downloaded))
            .unwrap_or(false)
    }

    /// Requests an autoplay radio once the queue reaches its last track.
    fn maybe_request_autoplay(&mut self) {
        let Some(current) = self.current().cloned() else {
            return;
        };
        if self.current + 1 < self.list.len() {
            return;
        }
        if self.autoplay_requested.as_deref() == Some(current.video_id.as_str()) {
            return;
        }
        self.autoplay_requested = Some(current.video_id.clone());
        let _ = self
            .updater
            .try_send(ManagerMessage::Autoplay(current.video_id));
    }

    pub fn update(&mut self) {
        PLAYER_RUNNING.store(self.current().is_some(), Ordering::SeqCst);
        self.update_controls();
        self.handle_stream_errors();
        self.handle_stream_fetch();
        if self.current > self.list.len() {
            self.current = self.list.len();
        }
        while let Ok(e) = self.soundaction_receiver.try_recv() {
            e.apply_sound_action(self);
        }
        self.maybe_request_autoplay();
        if self.is_current_download_failed() {
            SoundAction::Next(1).apply_sound_action(self);
        }
        if self.sink.is_finished() {
            if self.pending_stream.is_none() && self.rtcurrent.as_ref() == self.current() {
                self.set_relative_current(1);
                self.rtcurrent = None;
            }
            self.handle_stream_errors();
            self.update_controls();
            // If the current song is finished, we play the next one but if the next one has failed to download, we skip it
            // TODO(optimize this)
            while self
                .current()
                .map(|x| {
                    self.music_status.get(&x.video_id) == Some(&MusicDownloadStatus::DownloadFailed)
                })
                .unwrap_or(false)
            {
                self.set_relative_current(1);
            }

            if self.is_current_downloaded() {
                if let Some(video) = self.current().cloned() {
                    let k = CACHE_DIR.join(format!("downloads/{}.mp4", &video.video_id));
                    if let Err(e) = self.sink.play(k.as_path()) {
                        if matches!(e, PlayError::DecoderError(_)) {
                            // Cleaning the file

                            DATABASE.remove_video(&video);
                            handle_error(
                                &self.updater,
                                "invalid cleaning MP4",
                                std::fs::remove_file(k),
                            );
                            handle_error(
                                &self.updater,
                                "invalid cleaning JSON",
                                std::fs::remove_file(
                                    CACHE_DIR.join(format!("downloads/{}.json", &video.video_id)),
                                ),
                            );
                            self.current = 0;
                            DATABASE.write();
                        } else {
                            self.updater
                                .send(ManagerMessage::PassTo(
                                    Screens::DeviceLost,
                                    Box::new(ManagerMessage::Error(
                                        format!("{e:?}"),
                                        Box::new(None),
                                    )),
                                ))
                                .unwrap();
                        }
                    }
                }
            }
        } else {
            self.rtcurrent = self.current().cloned();
        }
        self.maybe_fetch_current();

        let to_download = self
            .list
            .iter()
            .skip(self.current)
            .chain(self.list.iter().take(self.current).rev())
            .filter(|x| {
                self.music_status.get(&x.video_id) == Some(&MusicDownloadStatus::NotDownloaded)
            })
            .take(PREFETCH_AHEAD)
            .cloned()
            .collect::<VecDeque<_>>();
        let new_ids: Vec<String> = to_download.iter().map(|v| v.video_id.clone()).collect();
        if new_ids != self.last_download_list {
            self.last_download_list = new_ids;
            DOWNLOAD_MANAGER.set_download_list(to_download);
        }
    }

    /// Picks up audio fetched in the background and starts playing it.
    fn handle_stream_fetch(&mut self) {
        while let Ok((id, result)) = self.stream_rx.try_recv() {
            if self.pending_stream.as_deref() == Some(id.as_str()) {
                self.pending_stream = None;
            }
            if self.current().map(|v| v.video_id == id).unwrap_or(false) {
                match result {
                    Ok(bytes) => {
                        if let Err(e) = self.sink.play_bytes(bytes) {
                            error!("could not play streamed audio for {id}: {e:?}");
                        }
                    }
                    Err(e) => {
                        error!("stream fetch failed for {id}: {e}");
                        self.music_status
                            .insert(id, MusicDownloadStatus::DownloadFailed);
                    }
                }
            }
        }
    }

    /// Starts fetching the current track into RAM when it is not already on disk.
    fn maybe_fetch_current(&mut self) {
        if self.pending_stream.is_some()
            || self.is_current_downloaded()
            || self.rtcurrent.as_ref() == self.current()
        {
            return;
        }
        let Some(video) = self.current().cloned() else {
            return;
        };
        self.pending_stream = Some(video.video_id.clone());
        let tx = self.stream_tx.clone();
        std::thread::spawn(move || {
            let result = download_manager::stream_bytes(&video.video_id);
            let _ = tx.send((video.video_id, result));
        });
    }

    fn handle_stream_errors(&self) {
        while let Ok(e) = self.stream_error_receiver.try_recv() {
            error!("Stream error: {:?}", e);
            handle_error(&self.updater, "audio device stream error", Err(e));
        }
    }
    fn update_controls(&mut self) {
        let current = self.current().cloned();
        let result = self
            .controls
            .update(current, &self.sink)
            .map_err(|x| format!("{x:?}"));
        handle_error::<String>(&self.updater, "Can't update finished media control", result);
    }
}

pub fn player_system(updater: Sender<ManagerMessage>) -> (Sender<SoundAction>, PlayerState) {
    let (tx, rx) = flume::unbounded::<SoundAction>();
    (tx.clone(), PlayerState::new(tx, rx, updater))
}

/// Parses "3:34" or "1:02:03" into seconds.
fn parse_duration(text: &str) -> Option<u64> {
    let mut seconds = 0u64;
    let mut parts = 0;
    for part in text.split(':') {
        let value: u64 = part.trim().parse().ok()?;
        seconds = seconds * 60 + value;
        parts += 1;
    }
    (parts >= 2).then_some(seconds)
}
