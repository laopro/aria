use common_structs::MusicDownloadStatus;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};

use rand::seq::SliceRandom;
use ratatui::{layout::Rect, style::{Modifier, Style}};

use crate::{
    structures::{app_status::MusicDownloadStatusExt, sound_action::SoundAction},
    systems::{player::PlayerState, DOWNLOAD_MANAGER},
    utils::invert,
};

use super::{
    format_track, rect_contains, relative_pos, EventResponse, ManagerMessage, Screen, Screens,
    ACCENT, TEXT,
};

impl PlayerState {
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
        frame_data: &ratatui::layout::Rect,
    ) -> EventResponse {
        let x = mouse_event.column;
        let y = mouse_event.row;
        let list_rect = *frame_data;
        if let MouseEventKind::Down(_) = &mouse_event.kind {
            if rect_contains(&list_rect, x, y, 1) {
                let (_, y) = relative_pos(&list_rect, x, y, 1);
                if let Some(e) = self
                    .list_selector
                    .click_on(y as usize, list_rect.height as usize)
                {
                    self.activate(e);
                }
            }
        } else if let MouseEventKind::ScrollUp = &mouse_event.kind {
            self.list_selector.scroll_up();
        } else if let MouseEventKind::ScrollDown = &mouse_event.kind {
            self.list_selector.scroll_down();
        }
        EventResponse::None
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &ratatui::layout::Rect) -> EventResponse {
        match key.code {
            KeyCode::Esc => ManagerMessage::ChangeState(self.goto).event(),
            KeyCode::F(5) => {
                // Get all musics that have failled to download
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
                // Download them
                DOWNLOAD_MANAGER.add_to_download_list(musics);
                EventResponse::None
            }
            KeyCode::Char('f') => ManagerMessage::SearchFrom(Screens::MusicPlayer).event(),
            KeyCode::Char('s') => {
                self.list.shuffle(&mut rand::thread_rng());
                self.current = 0;
                self.sink.stop();
                EventResponse::None
            }
            KeyCode::Char('C') => {
                SoundAction::Cleanup.apply_sound_action(self);
                EventResponse::None
            }
            KeyCode::Char(' ') => {
                SoundAction::PlayPause.apply_sound_action(self);
                EventResponse::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.list_selector.scroll_up();
                EventResponse::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.list_selector.scroll_down();
                EventResponse::None
            }
            KeyCode::Enter => {
                if let Some(e) = self.list_selector.play() {
                    self.activate(e);
                }
                EventResponse::None
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                SoundAction::Plus.apply_sound_action(self);
                EventResponse::None
            }
            KeyCode::Char('-') => {
                SoundAction::Minus.apply_sound_action(self);
                EventResponse::None
            }
            KeyCode::Char('<') | KeyCode::Left | KeyCode::Char('h') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    SoundAction::Previous(1).apply_sound_action(self);
                } else {
                    SoundAction::Backward.apply_sound_action(self);
                }
                EventResponse::None
            }
            KeyCode::Char('>') | KeyCode::Right | KeyCode::Char('l') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    SoundAction::Next(1).apply_sound_action(self);
                } else {
                    SoundAction::Forward.apply_sound_action(self);
                }
                EventResponse::None
            }
            KeyCode::Char('r') => {
                SoundAction::DeleteVideoUnary.apply_sound_action(self);
                EventResponse::None
            }
            _ => EventResponse::None,
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame, area: Rect) {
        let playing = !self.sink.is_paused();
        self.list_selector.update(self.list.len(), self.current);
        self.list_selector.render(
            area,
            f.buffer_mut(),
            |index, select, scroll| {
                let music_state = self
                    .list
                    .get(index)
                    .and_then(|x| self.music_status.get(&x.video_id))
                    .copied()
                    .unwrap_or(MusicDownloadStatus::Downloaded);
                let music_state_c = music_state.character(Some(playing));
                (
                    if select {
                        Style::default()
                            .bg(ACCENT)
                            .fg(TEXT)
                            .add_modifier(Modifier::BOLD)
                    } else if scroll {
                        invert(music_state.style(None))
                    } else {
                        music_state.style(None)
                    },
                    if let Some(e) = self.list.get(index) {
                        format!(" {music_state_c}{}", format_track(e))
                    } else {
                        String::new()
                    },
                )
            },
            " Now playing ",
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
