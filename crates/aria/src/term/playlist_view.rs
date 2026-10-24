use crossterm::event::{KeyCode, KeyEvent};
use flume::Sender;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    consts::CONFIG,
    structures::sound_action::SoundAction,
    utils::{invert, to_bidi_string},
    DATABASE,
};

use super::{
    format_track,
    item_list::{ListItem, ListItemAction},
    rect_contains, split_y_start, EventResponse, ManagerMessage, Screen, Screens, ACCENT,
    TEXT_DIM,
};

#[derive(Clone)]
pub struct PlayListAction(usize, bool);

impl ListItemAction for PlayListAction {
    fn render_style(&self, _: &str, selected: bool) -> Style {
        if selected {
            if self.1 {
                invert(CONFIG.player.text_downloading_style)
            } else {
                invert(CONFIG.player.text_next_style)
            }
        } else if self.1 {
            CONFIG.player.text_downloading_style
        } else {
            CONFIG.player.text_next_style
        }
    }
}

// Audio device not connected!
pub struct PlaylistView {
    pub items: ListItem<PlayListAction>,
    pub videos: Vec<YoutubeMusicVideoRef>,
    pub goto: Screens,
    pub sender: Sender<SoundAction>,
}

impl PlaylistView {
    fn play_all(&mut self) -> EventResponse {
        if self.videos.is_empty() {
            return EventResponse::None;
        }
        self.sender
            .send(SoundAction::ReplaceQueue(self.videos.clone()))
            .unwrap();
        EventResponse::Message(vec![ManagerMessage::PlayerFrom(Screens::Playlist)])
    }
}

impl Screen for PlaylistView {
    fn on_mouse_press(&mut self, e: crossterm::event::MouseEvent, r: &Rect) -> EventResponse {
        let [actions, list] = split_y_start(*r, 1);
        if rect_contains(&actions, e.column, e.row, 0) {
            // "Play all" on the left, "Back" on the right.
            if e.column + 10 >= actions.x + actions.width {
                return ManagerMessage::ChangeState(self.goto).event();
            }
            return self.play_all();
        }
        if let Some(PlayListAction(v, _)) = self.items.on_mouse_press(e, &list) {
            self.sender
                .send(SoundAction::ReplaceQueue(
                    self.videos.iter().skip(v).cloned().collect(),
                ))
                .unwrap();
            EventResponse::Message(vec![ManagerMessage::PlayerFrom(Screens::Playlist)])
        } else {
            EventResponse::None
        }
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        if let Some(PlayListAction(v, _)) = self.items.on_key_press(key) {
            self.sender
                .send(SoundAction::ReplaceQueue(
                    self.videos.iter().skip(*v).cloned().collect(),
                ))
                .unwrap();
            return EventResponse::Message(vec![ManagerMessage::PlayerFrom(Screens::Playlist)]);
        }
        match key.code {
            KeyCode::Esc => ManagerMessage::ChangeState(self.goto).event(),
            KeyCode::Char('p') => self.play_all(),
            KeyCode::Char('f') => ManagerMessage::SearchFrom(Screens::PlaylistViewer).event(),
            _ => EventResponse::None,
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [actions, list] = split_y_start(area, 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " > Play all ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled("│", Style::default().fg(TEXT_DIM)),
                Span::styled(" ← Volver ", Style::default().fg(TEXT_DIM)),
            ])),
            actions,
        );
        frame.render_widget(&self.items, list);
    }

    fn handle_global_message(&mut self, m: ManagerMessage) -> EventResponse {
        match m {
            ManagerMessage::Inspect(a, screen, m) => {
                self.items
                    .set_title(format!(" {} ", to_bidi_string(&a)));
                self.goto = screen;
                let db = DATABASE.read().unwrap();
                self.items.update(
                    m.iter()
                        .enumerate()
                        .map(|(i, m)| {
                            (
                                format_track(m),
                                PlayListAction(i, !db.iter().any(|x| x.video_id == m.video_id)),
                            )
                        })
                        .collect(),
                    0,
                );
                self.videos = m;

                EventResponse::Message(vec![ManagerMessage::ChangeState(Screens::PlaylistViewer)])
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
