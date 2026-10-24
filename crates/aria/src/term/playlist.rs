use std::sync::atomic::AtomicBool;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use flume::{Receiver, Sender};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    consts::CONFIG,
    run_service,
    utils::{invert, to_bidi_string},
    DATABASE,
};

use super::{
    item_list::{ListItem, ListItemAction},
    rect_contains, split_y_start, EventResponse, ManagerMessage, Screen, Screens, ACCENT, TEXT,
    TEXT_DIM,
};

#[derive(Clone)]
pub enum ChooserAction {
    Play(PlayListEntry),
}

impl ListItemAction for ChooserAction {
    fn render_style(&self, _: &str, selected: bool) -> Style {
        if selected {
            invert(CONFIG.player.text_next_style)
        } else {
            CONFIG.player.text_next_style
        }
    }
}

pub struct Chooser {
    pub item_list: ListItem<ChooserAction>,
    pub goto: Screens,
    input: String,
    input_active: bool,
    status: String,
    import_tx: Sender<(String, Vec<YoutubeMusicVideoRef>)>,
    import_rx: Receiver<(String, Vec<YoutubeMusicVideoRef>)>,
}

#[derive(Clone)]
pub struct PlayListEntry {
    pub name: String,
    pub videos: Vec<YoutubeMusicVideoRef>,
    pub text_to_show: String,
}

impl PlayListEntry {
    pub fn new(name: String, videos: Vec<YoutubeMusicVideoRef>) -> Self {
        Self {
            text_to_show: format_playlist(&name, &videos),
            name,
            videos,
        }
    }
}

pub fn format_playlist(name: &str, videos: &[YoutubeMusicVideoRef]) -> String {
    let db = DATABASE.read().unwrap();
    let local_videos = videos
        .iter()
        .filter(|x| db.iter().any(|y| x.video_id == y.video_id))
        .count();
    format!(
        " {}  ·  {}/{} local",
        to_bidi_string(name),
        local_videos,
        videos.len()
    )
}

/// How many rows the link input and the status line take.
const FORM_HEIGHT: u16 = 2;

impl Chooser {
    pub fn new(goto: Screens) -> Self {
        let (import_tx, import_rx) = flume::unbounded();
        Self {
            item_list: ListItem::new(" Choose a playlist ".to_owned()),
            goto,
            input: String::new(),
            input_active: false,
            status: if crate::is_guest() {
                " Guest mode: run `aria --login` to sync your library".to_owned()
            } else {
                String::new()
            },
            import_tx,
            import_rx,
        }
    }

    /// Opens the playlist view instead of starting playback.
    fn inspect(&self, entry: PlayListEntry) -> EventResponse {
        EventResponse::Message(vec![ManagerMessage::Inspect(
            entry.name,
            Screens::Playlist,
            entry.videos,
        )
        .pass_to(Screens::PlaylistViewer)])
    }

    /// Extracts the `list=` parameter from a YouTube Music URL.
    fn playlist_id(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if let Some(pos) = trimmed.find("list=") {
            let id: String = trimmed[pos + 5..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !id.is_empty() {
                return Some(id);
            }
        }
        let raw: String = trimmed
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        (raw.len() > 10 && raw.len() == trimmed.len()).then_some(raw)
    }

    fn load_link(&mut self) {
        let Some(id) = Self::playlist_id(&self.input) else {
            self.status = " Invalid link".to_owned();
            return;
        };
        let browse_id = if id.starts_with("VL") {
            id
        } else {
            format!("VL{id}")
        };
        self.status = " Loading playlist...".to_owned();
        self.input.clear();
        self.input_active = false;

        let sender = self.import_tx.clone();
        run_service(async move {
            let Some(api) = crate::api_instance().await else {
                let _ = sender.send((" No YouTube Music session".to_owned(), Vec::new()));
                return;
            };
            match api.get_playlist_named(&browse_id, 5).await {
                Ok((name, videos)) if !videos.is_empty() => {
                    let name = if name.trim().is_empty() {
                        "Imported playlist".to_owned()
                    } else {
                        name
                    };
                    let _ = sender.send((name, videos));
                }
                Ok(_) => {
                    let _ = sender.send((" The playlist is empty".to_owned(), Vec::new()));
                }
                Err(_) => {
                    let _ = sender.send((" Could not load the playlist".to_owned(), Vec::new()));
                }
            }
        });
    }

    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    fn add_element(&mut self, element: (String, Vec<YoutubeMusicVideoRef>)) {
        let entry = PlayListEntry::new(element.0, element.1);
        self.item_list
            .add_element((entry.text_to_show.clone(), ChooserAction::Play(entry)));
    }

    fn drain_imports(&mut self) {
        while let Ok((name, videos)) = self.import_rx.try_recv() {
            if videos.is_empty() {
                self.status = format!(" {name}");
            } else {
                self.status = " Playlist added".to_owned();
                self.add_element((name, videos));
            }
        }
    }
}

impl Screen for Chooser {
    fn on_mouse_press(&mut self, mouse_event: MouseEvent, frame_data: &Rect) -> EventResponse {
        let [form_area, list_area] = split_y_start(*frame_data, FORM_HEIGHT);
        if rect_contains(&form_area, mouse_event.column, mouse_event.row, 0) {
            self.input_active = true;
            return EventResponse::None;
        }
        self.input_active = false;
        if let Some(ChooserAction::Play(a)) = self.item_list.on_mouse_press(mouse_event, &list_area)
        {
            return self.inspect(a);
        }
        EventResponse::None
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        if self.input_active {
            match key.code {
                KeyCode::Esc => {
                    self.input_active = false;
                    self.input.clear();
                }
                KeyCode::Enter => self.load_link(),
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(c) => self.input.push(c),
                _ => {}
            }
            return EventResponse::None;
        }

        if let Some(ChooserAction::Play(a)) = self.item_list.on_key_press(key).cloned() {
            return self.inspect(a);
        }
        match key.code {
            KeyCode::Esc => return ManagerMessage::ChangeState(Screens::MusicPlayer).event(),
            KeyCode::Char('f') => return ManagerMessage::SearchFrom(Screens::Playlist).event(),
            KeyCode::Char('/') | KeyCode::Char('a') => self.input_active = true,
            _ => {}
        }
        EventResponse::None
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.drain_imports();

        let [form_area, list_area] = split_y_start(area, FORM_HEIGHT);
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(form_area);

        let placeholder = "Paste a playlist link and press Enter";
        let text_style = if self.input_active {
            Style::default().fg(TEXT)
        } else {
            Style::default().fg(TEXT_DIM)
        };
        let text = if self.input.is_empty() {
            placeholder.to_owned()
        } else {
            self.input.clone()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" + ", Style::default().fg(ACCENT)),
                Span::styled(text, text_style),
                Span::styled(
                    if self.input_active { "|" } else { "" },
                    Style::default().fg(ACCENT),
                ),
            ])),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(self.status.clone(), Style::default().fg(TEXT_DIM))),
            rows[1],
        );

        frame.render_widget(&self.item_list, list_area);
    }

    fn handle_global_message(&mut self, message: super::ManagerMessage) -> EventResponse {
        if let ManagerMessage::AddElementToChooser(a) = message {
            self.add_element(a);
        }
        EventResponse::None
    }

    fn close(&mut self, _: Screens) -> EventResponse {
        EventResponse::None
    }

    fn open(&mut self) -> EventResponse {
        EventResponse::None
    }
}

pub static PLAYER_RUNNING: AtomicBool = AtomicBool::new(false);
