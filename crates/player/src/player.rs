use flume::Sender;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::{PlayError, PlayerData, PlayerOptions, SEEK_STEP};

pub struct Player {
    sink: Sink,
    stream: OutputStream,
    data: PlayerData,
    error_sender: Sender<PlayError>,
    options: PlayerOptions,
    /// Position committed when playback was paused or seeked.
    pos_base: Duration,
    /// When the current playback run started, if it is playing.
    started: Option<Instant>,
}

impl Player {
    fn try_from_device(device: rodio::cpal::Device) -> Result<OutputStream, PlayError> {
        // WASAPI shared mode only accepts the device's own format, so ask the
        // device instead of relying on rodio's default (44100 Hz).
        let config = device
            .default_output_config()
            .map_err(|_| PlayError::StreamError(rodio::StreamError::NoDevice))?;

        OutputStreamBuilder::default()
            .with_device(device)
            .with_supported_config(&config)
            .open_stream()
            .map_err(PlayError::StreamError)
    }

    /// Try to create a stream from the default device, falling back to others
    fn try_default() -> Result<OutputStream, PlayError> {
        // Use rodio's internal cpal re-export
        let host = rodio::cpal::default_host();

        let default_device = host
            .default_output_device()
            .ok_or(PlayError::StreamError(rodio::StreamError::NoDevice))?;

        Self::try_from_device(default_device).or_else(|original_err| {
            let devices = host.output_devices().map_err(|_| original_err)?;

            for d in devices {
                if let Ok(res) = Self::try_from_device(d) {
                    return Ok(res);
                }
            }
            Err(PlayError::StreamError(rodio::StreamError::NoDevice))
        })
    }

    pub fn new(error_sender: Sender<PlayError>, options: PlayerOptions) -> Result<Self, PlayError> {
        let stream = Self::try_default()?;

        // sink::try_new requires a reference to the handle
        let sink = Sink::connect_new(stream.mixer());

        sink.set_volume(options.initial_volume_f32());

        Ok(Self {
            sink,
            stream,
            error_sender,
            data: PlayerData::new(options.initial_volume()),
            options,
            pos_base: Duration::ZERO,
            started: None,
        })
    }

    pub fn update(&self) -> Result<Self, PlayError> {
        let stream = Self::try_default()?;
        let sink = Sink::connect_new(stream.mixer());

        sink.set_volume(self.data.volume_f32());

        Ok(Self {
            sink,
            stream,
            error_sender: self.error_sender.clone(),
            data: self.data.clone(),
            options: self.options.clone(),
            pos_base: Duration::ZERO,
            started: None,
        })
    }
    pub fn change_volume(&mut self, positive: bool) {
        self.data.change_volume(positive);
        self.sink.set_volume(self.data.volume_f32());
    }

    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }

    pub fn play_at(&mut self, path: &Path, time: Duration) -> Result<(), PlayError> {
        log::info!("Playing file: {:?} at time: {:?}", path, time);
        self.play(path)?;
        if let Err(e) = self.sink.try_seek(time) {
            log::error!("Seek error: {}", e);
            let _ = self.error_sender.send(PlayError::SeekError(e));
        } else {
            self.start_tracking(time);
        }

        Ok(())
    }

    pub fn play(&mut self, path: &Path) -> Result<(), PlayError> {
        log::info!("Playing file: {:?}", path);
        self.data.set_current_file(Some(path.to_path_buf()));

        self.stop();

        let file = File::open(path).map_err(PlayError::Io)?;

        if file.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(PlayError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "File is empty",
            )));
        }

        let decoder = Decoder::new(file).map_err(PlayError::DecoderError)?;

        self.data.set_total_duration(decoder.total_duration());

        // Check if sink is detached or empty and recreate if necessary
        if self.sink.empty() {
            // Using try_new with the stored handle
            self.sink = Sink::connect_new(self.stream.mixer());
        }

        self.sink.set_volume(self.data.volume_f32());
        self.sink.append(decoder);
        self.start_tracking(Duration::ZERO);

        Ok(())
    }

    /// Plays audio already held in memory, without touching the disk.
    /// Seeking still works because the decoder owns a `Cursor` over the buffer.
    pub fn play_bytes(&mut self, bytes: Vec<u8>) -> Result<(), PlayError> {
        if bytes.is_empty() {
            return Err(PlayError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Audio buffer is empty",
            )));
        }

        self.stop();
        self.data.set_current_file(None);

        let decoder = Decoder::new(std::io::Cursor::new(bytes)).map_err(PlayError::DecoderError)?;

        self.data.set_total_duration(decoder.total_duration());

        if self.sink.empty() {
            self.sink = Sink::connect_new(self.stream.mixer());
        }

        self.sink.set_volume(self.data.volume_f32());
        self.sink.append(decoder);
        self.start_tracking(Duration::ZERO);

        Ok(())
    }

    pub fn stop(&mut self) {
        // rodio 0.21: To stop, you can clear the sink.
        if !self.sink.empty() {
            self.sink.clear();
        }
        self.pos_base = Duration::ZERO;
        self.started = None;
    }

    /// Restarts the position clock at `base`.
    fn start_tracking(&mut self, base: Duration) {
        self.pos_base = base;
        self.started = Some(Instant::now());
    }

    pub fn elapsed(&self) -> Duration {
        let live = self.started.map(|start| start.elapsed()).unwrap_or_default();
        let position = self.pos_base + live;
        match self.duration() {
            Some(total) => position.min(Duration::from_secs_f64(total)),
            None => position,
        }
    }

    pub fn duration(&self) -> Option<f64> {
        self.data
            .total_duration()
            .map(|duration| duration.as_secs_f64())
    }

    pub fn toggle_playback(&mut self) {
        if self.sink.is_paused() {
            self.sink.play();
            self.started = Some(Instant::now());
        } else {
            self.sink.pause();
            self.pos_base = self.elapsed();
            self.started = None;
        }
    }

    pub fn seek_fw(&mut self) {
        let current_elapsed = self.elapsed();
        let new_pos = current_elapsed + SEEK_STEP;

        self.seek_to(new_pos);
    }

    pub fn seek_bw(&mut self) {
        let current_elapsed = self.elapsed();
        let new_pos = current_elapsed.saturating_sub(SEEK_STEP);
        self.seek_to(new_pos);
    }

    pub fn seek_to(&mut self, time: Duration) {
        log::info!("Seek to: {:?}", time);
        if self.is_finished() {
            return;
        }
        let file = self.data.current_file();

        if let Err(e) = self.sink.try_seek(time) {
            log::error!("Seek error: {}", e);
            let _ = self.error_sender.send(PlayError::SeekError(e));
        } else {
            self.start_tracking(time);
            // If the sink is finished, we need to reset the music
            // This happens when the user seeks to the start of the song before the buffer.
            // In-memory playback keeps its own buffer, so there is nothing to reload.
            if self.is_finished() {
                if let Some(file) = file {
                    log::info!("Sink is finished while seeking, resetting the music");
                    if let Err(e) = self.play_at(&file, time) {
                        log::error!("Error playing file: {:?}", e);
                        let _ = self.error_sender.send(e);
                    }
                }
            }
        }
    }

    pub fn percentage(&self) -> f64 {
        self.duration().map_or(0.0, |duration| {
            let elapsed = self.elapsed().as_secs_f64();
            elapsed / duration
        })
    }

    pub fn volume_percent(&self) -> u8 {
        self.data.volume()
    }

    pub fn volume(&self) -> i32 {
        self.data.volume().into()
    }

    pub fn volume_up(&mut self) {
        let volume = self.volume() + 5;
        self.set_volume(volume);
    }

    pub fn volume_down(&mut self) {
        let volume = self.volume() - 5;
        self.set_volume(volume);
    }

    pub fn set_volume(&mut self, mut volume: i32) {
        volume = volume.clamp(0, 100);
        self.data.set_volume(volume as u8);
        self.sink.set_volume((volume as f32) / 100.0);
    }

    pub fn pause(&mut self) {
        if !self.sink.is_paused() {
            self.toggle_playback();
        }
    }

    pub fn resume(&mut self) {
        if self.sink.is_paused() {
            self.toggle_playback();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    pub fn seek(&mut self, secs: i64) {
        if secs.is_positive() {
            self.seek_fw();
        } else {
            self.seek_bw();
        }
    }

    pub fn get_progress(&self) -> (f64, u32, u32) {
        let position = self.elapsed();
        let duration = self.duration().unwrap_or(99.0) as u32;
        let percent = self.percentage() * 100.0;
        (percent.min(100.0), position.as_secs() as u32, duration)
    }
}
