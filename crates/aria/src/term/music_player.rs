use common_structs::MusicDownloadStatus;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
};

use crate::{
    structures::{app_status::MusicDownloadStatusExt, sound_action::SoundAction},
    systems::{player::PlayerState, DOWNLOAD_MANAGER},
};

use super::{
    format_track, menu::MenuTarget, theme::THEME, view::ListMouse, widgets::equalizer,
    EventResponse, ManagerMessage, Screen, Screens,
};

impl PlayerState {
    /// Plays the queue entry at `index`; the playing one toggles pause.
    pub fn activate(&mut self, index: usize) {
        match index.cmp(&self.current) {
            std::cmp::Ordering::Less => {
                SoundAction::Previous(self.current - index).apply_sound_action(self);
            }
            std::cmp::Ordering::Equal => {
                SoundAction::PlayPause.apply_sound_action(self);
            }
            std::cmp::Ordering::Greater => {
                SoundAction::Next(index - self.current).apply_sound_action(self)
            }
        }
    }
}

impl Screen for PlayerState {
    fn on_mouse_press(
        &mut self,
        mouse_event: crossterm::event::MouseEvent,
        frame_data: &Rect,
    ) -> EventResponse {
        match self.list_selector.mouse(mouse_event, *frame_data) {
            ListMouse::Activate(index) => self.activate(index),
            ListMouse::Menu(index) => {
                return ManagerMessage::OpenMenu(MenuTarget::Queue(index)).event()
            }
            ListMouse::Select(_) | ListMouse::None => {}
        }
        EventResponse::None
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        match key.code {
            KeyCode::Esc => return ManagerMessage::ChangeState(self.goto).event(),
            KeyCode::F(5) => {
                // Retry the tracks that failed to download.
                let mut musics = Vec::new();
                self.music_status
                    .iter_mut()
                    .for_each(|(key, music_status)| {
                        if MusicDownloadStatus::DownloadFailed != *music_status {
                            return;
                        }
                        if let Some(e) = self.list.iter().find(|x| &x.video_id == key) {
                            musics.push(e.clone());
                            *music_status = MusicDownloadStatus::NotDownloaded;
                        }
                    });
                DOWNLOAD_MANAGER.add_to_download_list(musics);
            }
            KeyCode::Char('f') => return ManagerMessage::SearchFrom(Screens::MusicPlayer).event(),
            KeyCode::Char('C') => SoundAction::Cleanup.apply_sound_action(self),
            KeyCode::Up | KeyCode::Char('k') => self.list_selector.scroll_up(),
            KeyCode::Down | KeyCode::Char('j') => self.list_selector.scroll_down(),
            KeyCode::PageUp => {
                let to = self.list_selector.cursor().saturating_sub(10);
                self.list_selector.scroll_to(to);
            }
            KeyCode::PageDown => {
                let to = self.list_selector.cursor() + 10;
                self.list_selector.scroll_to(to);
            }
            KeyCode::Home => self.list_selector.scroll_to(0),
            KeyCode::End => self.list_selector.scroll_to(usize::MAX),
            KeyCode::Enter => {
                if let Some(e) = self.list_selector.play() {
                    self.activate(e);
                }
            }
            KeyCode::Delete => {
                SoundAction::RemoveAt(self.list_selector.cursor()).apply_sound_action(self)
            }
            KeyCode::Char('m') => {
                return ManagerMessage::OpenMenu(MenuTarget::Queue(self.list_selector.cursor()))
                    .event()
            }
            _ => {}
        }
        EventResponse::None
    }

    fn render(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let playing = !self.sink.is_paused();
        self.list_selector.update(self.list.len(), self.current);
        let current = self.current;
        let list = &self.list;
        let status = &self.music_status;
        self.list_selector.render(
            area,
            f.buffer_mut(),
            |index| {
                let state = list
                    .get(index)
                    .and_then(|x| status.get(&x.video_id))
                    .copied()
                    .unwrap_or(MusicDownloadStatus::Downloaded);
                let label = list.get(index).map(format_track).unwrap_or_default();
                if index == current {
                    (
                        Style::default()
                            .fg(THEME.text)
                            .bg(THEME.accent_bg)
                            .add_modifier(Modifier::BOLD),
                        format!(" {}{label}", equalizer(playing)),
                    )
                } else {
                    let marker = match state {
                        MusicDownloadStatus::DownloadFailed => "!",
                        MusicDownloadStatus::Downloading(_) => "↓",
                        _ => " ",
                    };
                    (state.style(None), format!(" {marker}  {label}"))
                }
            },
            " Queue ",
        )
    }

    fn handle_global_message(&mut self, message: ManagerMessage) -> EventResponse {
        match message {
            ManagerMessage::RestartPlayer => {
                SoundAction::RestartPlayer.apply_sound_action(self);
                ManagerMessage::ChangeState(Screens::MusicPlayer).event()
            }
            _ => EventResponse::None,
        }
    }

    fn close(&mut self, _: Screens) -> EventResponse {
        EventResponse::None
    }

    fn open(&mut self) -> EventResponse {
        EventResponse::None
    }
}
