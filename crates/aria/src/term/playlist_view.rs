use crossterm::event::{KeyCode, KeyEvent};
use flume::Sender;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
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
    item_list::{ListEvent, ListItem, ListItemAction},
    menu::MenuTarget,
    rect_contains, split_y_start,
    theme::THEME,
    widgets::hovered,
    EventResponse, Hint, ManagerMessage, Screen, Screens,
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
    pub name: String,
    /// Browse id when the playlist comes from the account, else empty.
    pub browse_id: String,
}

impl PlaylistView {
    fn play_all(&mut self) -> EventResponse {
        if self.videos.is_empty() {
            return EventResponse::None;
        }
        self.sender
            .send(SoundAction::ReplaceQueue(self.videos.clone()))
            .unwrap();
        EventResponse::None
    }
}

/// Buttons of the action row: "Play all" and "Back".
fn action_buttons(row: Rect) -> [(Rect, &'static str); 2] {
    [
        (Rect { x: row.x, width: 12, ..row }, " ▶ Play all "),
        (Rect { x: row.x + 13, width: 8, ..row }, " ← Back "),
    ]
}

impl PlaylistView {
    fn play_from(&mut self, index: usize) {
        self.sender
            .send(SoundAction::ReplaceQueue(
                self.videos.iter().skip(index).cloned().collect(),
            ))
            .unwrap();
    }

    fn menu_for(&self, index: usize) -> EventResponse {
        // Only the user's own playlists offer "Remove from this playlist".
        let playlist = self
            .browse_id
            .starts_with("VLPL")
            .then(|| self.browse_id.clone());
        match self.videos.get(index) {
            Some(video) => {
                ManagerMessage::OpenMenu(MenuTarget::Video(video.clone(), playlist)).event()
            }
            None => EventResponse::None,
        }
    }

    /// Rebuilds the rows from `videos`, keeping the selection where it was.
    fn rebuild(&mut self, selected: usize) {
        let local: std::collections::HashSet<String> = DATABASE
            .read()
            .unwrap()
            .iter()
            .map(|x| x.video_id.clone())
            .collect();
        self.items.update(
            self.videos
                .iter()
                .enumerate()
                .map(|(i, m)| (format_track(m), PlayListAction(i, !local.contains(&m.video_id))))
                .collect(),
            selected,
        );
    }

    /// Mirrors a change made on the account when it touches the open playlist.
    pub fn apply_change(&mut self, change: &crate::account::LibraryChange) {
        use crate::account::LibraryChange;
        if self.browse_id.is_empty() {
            return;
        }
        let selected = self.items.current_position();
        match change {
            LibraryChange::Added { playlist, video } if playlist == &self.browse_id => {
                if self.videos.iter().any(|v| v.video_id == video.video_id) {
                    return;
                }
                if playlist == crate::account::LIKED_ID {
                    self.videos.insert(0, video.clone());
                } else {
                    self.videos.push(video.clone());
                }
            }
            LibraryChange::Removed { playlist, video_id } if playlist == &self.browse_id => {
                self.videos.retain(|v| &v.video_id != video_id);
            }
            LibraryChange::Refreshed { playlist, videos } if playlist == &self.browse_id => {
                if videos.iter().map(|v| &v.video_id).eq(self.videos.iter().map(|v| &v.video_id)) {
                    return;
                }
                self.videos = videos.clone();
            }
            LibraryChange::Deleted { browse_id } if browse_id == &self.browse_id => {
                self.videos.clear();
                self.browse_id.clear();
            }
            _ => return,
        }
        self.rebuild(selected);
    }
}

impl Screen for PlaylistView {
    fn on_mouse_press(&mut self, e: crossterm::event::MouseEvent, r: &Rect) -> EventResponse {
        let [actions, list] = split_y_start(*r, 1);
        if rect_contains(&actions, e.column, e.row, 0) {
            if matches!(
                e.kind,
                crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
            ) {
                let [play, back] = action_buttons(actions);
                if rect_contains(&play.0, e.column, e.row, 0) {
                    return self.play_all();
                }
                if rect_contains(&back.0, e.column, e.row, 0) {
                    return ManagerMessage::ChangeState(self.goto).event();
                }
            }
            return EventResponse::None;
        }
        match self.items.on_mouse_press(e, &list) {
            Some(ListEvent::Activate(PlayListAction(v, _))) => {
                self.play_from(v);
                EventResponse::None
            }
            Some(ListEvent::Menu(PlayListAction(v, _))) => self.menu_for(v),
            None => EventResponse::None,
        }
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        if let Some(PlayListAction(v, _)) = self.items.on_key_press(key).cloned() {
            self.play_from(v);
            return EventResponse::None;
        }
        match key.code {
            KeyCode::Esc => ManagerMessage::ChangeState(self.goto).event(),
            KeyCode::Char('p') => self.play_all(),
            KeyCode::Char('m') => self.menu_for(self.items.current_position()),
            KeyCode::Char('f') => ManagerMessage::SearchFrom(Screens::PlaylistViewer).event(),
            _ => EventResponse::None,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        vec![
            Hint::new("Enter", "play", KeyCode::Enter),
            Hint::new("p", "play all", KeyCode::Char('p')),
            Hint::new("m", "menu", KeyCode::Char('m')),
        ]
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [actions, list] = split_y_start(area, 1);
        let buf = frame.buffer_mut();
        for (i, (rect, label)) in action_buttons(actions).into_iter().enumerate() {
            let style = if hovered(rect, rect.y) {
                Style::default().fg(THEME.text).bg(THEME.hover)
            } else if i == 0 {
                Style::default()
                    .fg(THEME.text)
                    .bg(THEME.accent_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(THEME.text_dim).bg(THEME.bg)
            };
            buf.set_stringn(rect.x, rect.y, label, rect.width as usize, style);
        }
        self.items.placeholder = "Open a playlist from Home (F1) or Search (F2)".to_owned();
        self.items.render(list, frame.buffer_mut());
    }

    fn handle_global_message(&mut self, m: ManagerMessage) -> EventResponse {
        match m {
            ManagerMessage::Inspect(a, screen, m, browse_id) => {
                self.items
                    .set_title(format!(" {} ", to_bidi_string(&a)));
                self.goto = screen;
                self.name = a;
                self.browse_id = browse_id;
                self.videos = m;
                self.rebuild(0);

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
