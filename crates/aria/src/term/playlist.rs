use std::{sync::atomic::AtomicBool, time::Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use flume::{Receiver, Sender};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    Frame,
};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    account::{self, LibraryChange},
    consts::CONFIG,
    run_service,
    utils::{invert, to_bidi_string},
};

use super::{
    item_list::{ListEvent, ListItem, ListItemAction},
    rect_contains, split_y_start,
    theme::THEME,
    widgets::fit,
    EventResponse, Hint, ManagerMessage, Screen, Screens,
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

/// What the text box at the top of Home is for.
#[derive(Clone, Copy, PartialEq)]
enum Input {
    /// A playlist link to import.
    Link,
    /// The name of a playlist to create on the account.
    NewPlaylist,
}

pub struct Chooser {
    pub item_list: ListItem<ChooserAction>,
    pub goto: Screens,
    input: String,
    input_active: bool,
    input_kind: Input,
    status: String,
    /// Playlist a first Del press asked to delete; a second press confirms.
    pending_delete: Option<(String, Instant)>,
    updater: Sender<ManagerMessage>,
    import_tx: Sender<(String, Vec<YoutubeMusicVideoRef>)>,
    import_rx: Receiver<(String, Vec<YoutubeMusicVideoRef>)>,
}

#[derive(Clone)]
pub struct PlayListEntry {
    pub name: String,
    /// Browse id on the account; empty for local or imported lists.
    pub browse_id: String,
    pub videos: Vec<YoutubeMusicVideoRef>,
}

impl PlayListEntry {
    fn label(&self) -> String {
        format!(
            "{}\t{} tracks",
            to_bidi_string(self.name.trim()),
            self.videos.len()
        )
    }
}

/// How many rows the link input and the status line take.
const FORM_HEIGHT: u16 = 2;

impl Chooser {
    pub fn new(goto: Screens, updater: Sender<ManagerMessage>) -> Self {
        let (import_tx, import_rx) = flume::unbounded();
        Self {
            item_list: ListItem::new(" Your library ".to_owned()),
            goto,
            input: String::new(),
            input_active: false,
            input_kind: Input::Link,
            status: if crate::is_guest() {
                " Guest mode: press F8 to sign in and load your library".to_owned()
            } else {
                String::new()
            },
            pending_delete: None,
            updater,
            import_tx,
            import_rx,
        }
    }

    /// Opens the playlist view instead of starting playback.
    fn inspect(&self, entry: PlayListEntry) -> EventResponse {
        // Show what is cached right away, and fetch the current version behind it.
        if entry.browse_id.starts_with("VL") {
            account::refresh_playlist(self.updater.clone(), entry.browse_id.clone());
        }
        EventResponse::Message(vec![ManagerMessage::Inspect(
            entry.name,
            Screens::Playlist,
            entry.videos,
            entry.browse_id,
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
            match api.get_playlist_named(&browse_id, 50).await {
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

    /// Sends the typed text: a link to import, or the name of a new playlist.
    fn submit(&mut self) {
        match self.input_kind {
            Input::Link => self.load_link(),
            Input::NewPlaylist => {
                let name = self.input.trim().to_owned();
                self.input.clear();
                self.input_active = false;
                self.input_kind = Input::Link;
                if !name.is_empty() {
                    account::create_playlist(self.updater.clone(), name);
                }
            }
        }
    }

    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    /// Adds a list, or replaces the one with the same account id.
    fn add_playlist(&mut self, name: String, browse_id: String, videos: Vec<YoutubeMusicVideoRef>) {
        let entry = PlayListEntry {
            name,
            browse_id,
            videos,
        };
        let row = (entry.label(), ChooserAction::Play(entry.clone()));
        let rows = self.item_list.items_mut();
        let existing = rows.iter().position(|(_, ChooserAction::Play(other))| {
            !entry.browse_id.is_empty() && other.browse_id == entry.browse_id
        });
        match existing {
            Some(index) => rows[index] = row,
            None => rows.push(row),
        }
        self.item_list.refresh();
    }

    /// Mirrors a change made on the account.
    pub fn apply_change(&mut self, change: &LibraryChange) {
        let rows = self.item_list.items_mut();
        match change {
            LibraryChange::Added { playlist, video } => {
                for (label, ChooserAction::Play(entry)) in rows.iter_mut() {
                    if &entry.browse_id == playlist
                        && !entry.videos.iter().any(|v| v.video_id == video.video_id)
                    {
                        // Liked Music lists the newest like first.
                        if playlist == account::LIKED_ID {
                            entry.videos.insert(0, video.clone());
                        } else {
                            entry.videos.push(video.clone());
                        }
                        *label = entry.label();
                    }
                }
            }
            LibraryChange::Removed { playlist, video_id } => {
                for (label, ChooserAction::Play(entry)) in rows.iter_mut() {
                    if &entry.browse_id == playlist {
                        entry.videos.retain(|v| &v.video_id != video_id);
                        *label = entry.label();
                    }
                }
            }
            LibraryChange::Created { name, browse_id } => {
                self.add_playlist(name.clone(), browse_id.clone(), Vec::new());
                return;
            }
            LibraryChange::Deleted { browse_id } => {
                rows.retain(|(_, ChooserAction::Play(entry))| &entry.browse_id != browse_id);
            }
            LibraryChange::Reloading => {
                rows.retain(|(_, ChooserAction::Play(entry))| entry.browse_id.is_empty());
            }
            LibraryChange::Refreshed { playlist, videos } => {
                for (label, ChooserAction::Play(entry)) in rows.iter_mut() {
                    if &entry.browse_id == playlist {
                        entry.videos = videos.clone();
                        *label = entry.label();
                    }
                }
            }
        }
        self.item_list.refresh();
    }

    fn drain_imports(&mut self) {
        while let Ok((name, videos)) = self.import_rx.try_recv() {
            if videos.is_empty() {
                self.status = format!(" {name}");
            } else {
                self.status = " Playlist added".to_owned();
                self.add_playlist(name, String::new(), videos);
            }
        }
    }

    /// Del on a playlist: the first press asks, the second one deletes it.
    fn delete_selected(&mut self) {
        let Some(ChooserAction::Play(entry)) = self.item_list.select().cloned() else {
            return;
        };
        if !entry.browse_id.starts_with("VLPL") {
            self.status = " Only your own playlists can be deleted".to_owned();
            return;
        }
        match &self.pending_delete {
            Some((id, at)) if id == &entry.browse_id && at.elapsed().as_secs() < 5 => {
                self.pending_delete = None;
                self.status = format!(" Deleting \"{}\"…", entry.name);
                account::delete_playlist(self.updater.clone(), entry.browse_id);
            }
            _ => {
                self.status = format!(
                    " Press Del again to delete \"{}\" from your account",
                    entry.name
                );
                self.pending_delete = Some((entry.browse_id, Instant::now()));
            }
        }
    }
}

impl Screen for Chooser {
    fn on_mouse_press(&mut self, mouse_event: MouseEvent, frame_data: &Rect) -> EventResponse {
        let [form_area, list_area] = split_y_start(*frame_data, FORM_HEIGHT);
        let left_down = matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left));
        if rect_contains(&form_area, mouse_event.column, mouse_event.row, 0) {
            if left_down {
                self.input_active = true;
            }
            return EventResponse::None;
        }
        if left_down {
            self.input_active = false;
        }
        match self.item_list.on_mouse_press(mouse_event, &list_area) {
            Some(ListEvent::Activate(ChooserAction::Play(a)))
            | Some(ListEvent::Menu(ChooserAction::Play(a))) => self.inspect(a),
            None => EventResponse::None,
        }
    }

    fn wants_text(&self) -> bool {
        self.input_active
    }

    fn hints(&self) -> Vec<Hint> {
        if self.input_active {
            return vec![Hint::new("Enter", "confirm", KeyCode::Enter)];
        }
        vec![
            Hint::new("Enter", "open", KeyCode::Enter),
            Hint::new("n", "new playlist", KeyCode::Char('n')),
            Hint::new("F5", "sync", KeyCode::F(5)),
        ]
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        if self.input_active {
            match key.code {
                KeyCode::Esc => {
                    self.input_active = false;
                    self.input_kind = Input::Link;
                    self.input.clear();
                }
                KeyCode::Enter => self.submit(),
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
            KeyCode::Char('/') | KeyCode::Char('a') => {
                self.input_kind = Input::Link;
                self.input_active = true;
            }
            KeyCode::Char('n') => {
                if crate::is_guest() {
                    self.status = " Sign in first (F8) to create playlists".to_owned();
                } else {
                    self.input_kind = Input::NewPlaylist;
                    self.input_active = true;
                }
            }
            KeyCode::Delete => self.delete_selected(),
            KeyCode::F(5) => crate::tasks::api::resync(self.updater.clone()),
            _ => {}
        }
        EventResponse::None
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.drain_imports();

        let [form_area, list_area] = split_y_start(area, FORM_HEIGHT);
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(form_area);

        let base = Style::default().bg(THEME.bg);
        let text_style = if self.input_active {
            base.fg(THEME.text).bg(THEME.hover)
        } else {
            base.fg(THEME.text_dim)
        };
        let prompt = match self.input_kind {
            Input::NewPlaylist if self.input_active => "New playlist name: ",
            _ => "",
        };
        let text = if self.input.is_empty() && !self.input_active {
            "Paste a playlist link here (click or press /) · n creates a playlist".to_owned()
        } else {
            format!(
                "{prompt}{}{}",
                self.input,
                if self.input_active { "▏" } else { "" }
            )
        };
        let buf = frame.buffer_mut();
        buf.set_string(rows[0].x, rows[0].y, " + ", base.fg(THEME.accent));
        buf.set_string(
            rows[0].x + 3,
            rows[0].y,
            fit(&text, rows[0].width.saturating_sub(3) as usize),
            text_style,
        );
        buf.set_stringn(
            rows[1].x,
            rows[1].y,
            &self.status,
            rows[1].width as usize,
            base.fg(THEME.text_dim),
        );

        self.item_list.loading =
            crate::tasks::api::SYNCING.load(std::sync::atomic::Ordering::Relaxed) > 0;
        self.item_list.placeholder = if self.item_list.loading {
            "Loading your library…".to_owned()
        } else {
            "No playlists yet: search with F2 or paste a link above".to_owned()
        };
        self.item_list.render(list_area, frame.buffer_mut());
    }

    fn handle_global_message(&mut self, message: super::ManagerMessage) -> EventResponse {
        match message {
            ManagerMessage::AddElementToChooser((name, videos)) => {
                // Local lists reload under the same name: keep a single entry.
                self.item_list
                    .items_mut()
                    .retain(|(_, ChooserAction::Play(entry))| {
                        !(entry.browse_id.is_empty() && entry.name == name)
                    });
                self.add_playlist(name, String::new(), videos);
            }
            ManagerMessage::AddPlaylist(name, browse_id, videos) => {
                self.add_playlist(name, browse_id, videos)
            }
            _ => {}
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
