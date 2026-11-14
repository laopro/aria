use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use flume::Sender;
use log::error;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    Frame,
};
use tokio::task::JoinHandle;
use ytpapi2::{SearchResults, YoutubeMusicPlaylistRef, YoutubeMusicVideoRef};

use crate::{
    run_service,
    structures::sound_action::SoundAction,
    utils::to_bidi_string,
    DATABASE,
};

use super::{
    format_track,
    item_list::{ListEvent, ListItem, ListItemAction},
    menu::MenuTarget,
    theme::THEME,
    widgets::{fit, hovered},
    EventResponse, Hint, ManagerMessage, Screen, Screens,
};

/// Result filter shown as a tab under the search box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tab {
    #[default]
    All,
    Songs,
    Videos,
    Playlists,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::All, Tab::Songs, Tab::Videos, Tab::Playlists];

    fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Songs => "Songs",
            Tab::Videos => "Videos",
            Tab::Playlists => "Playlists",
        }
    }

    /// The tab `step` places away, wrapping around.
    fn shift(self, step: isize) -> Tab {
        let index = Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as isize;
        Self::ALL[(index + step).rem_euclid(Self::ALL.len() as isize) as usize]
    }

    fn accepts(self, kind: Kind) -> bool {
        matches!(
            (self, kind),
            (Tab::All, _)
                | (Tab::Songs, Kind::Song)
                | (Tab::Videos, Kind::Video)
                | (Tab::Playlists, Kind::Playlist)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Song,
    Video,
    Playlist,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Song => "[S]",
            Kind::Video => "[V]",
            Kind::Playlist => "[P]",
        }
    }
}

#[derive(Clone)]
struct Entry {
    kind: Kind,
    /// Video id or playlist browse id, used to avoid listing the same item twice.
    key: String,
    label: String,
    status: Status,
    score: i64,
}

/// Everything found for the current query, kept between tab switches.
#[derive(Default)]
struct Results {
    query: String,
    tab: Tab,
    entries: Vec<Entry>,
}

impl Results {
    /// Adds a result unless the same track or playlist is already listed.
    fn add(
        &mut self,
        kind: Kind,
        status: Status,
        body: String,
        title: &str,
        author: &str,
        rank: usize,
    ) {
        let key = match &status {
            Status::Local(v) | Status::Unknown(v) => v.video_id.clone(),
            Status::PlayList(p, _) => p.browse_id.clone(),
        };
        if self.entries.iter().any(|e| e.key == key) {
            return;
        }
        let mut score = relevance(&self.query, title, author, rank);
        match kind {
            Kind::Song => score += 5,
            // A playlist named like the query should not bury the songs themselves.
            Kind::Playlist => score -= 15,
            Kind::Video => {}
        }
        self.entries.push(Entry {
            kind,
            key,
            label: format!("{} {}", kind.tag(), body.trim_start()),
            status,
            score,
        });
    }

    fn count(&self, tab: Tab) -> usize {
        self.entries.iter().filter(|e| tab.accepts(e.kind)).count()
    }

    /// The rows of the current tab, best match first.
    fn visible(&self) -> Vec<(String, Status)> {
        let mut shown: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| self.tab.accepts(e.kind))
            .collect();
        shown.sort_by(|a, b| b.score.cmp(&a.score));
        shown
            .into_iter()
            .map(|e| (e.label.clone(), e.status.clone()))
            .collect()
    }
}

/// Higher is a better match: exact or prefix title first, then matching words.
fn relevance(query: &str, title: &str, author: &str, rank: usize) -> i64 {
    let q = query.trim().to_lowercase();
    let t = title.to_lowercase();
    let a = author.to_lowercase();
    let base = if t == q {
        100
    } else if t.starts_with(&q) {
        80
    } else if t.contains(&q) {
        60
    } else if a.contains(&q) {
        40
    } else {
        0
    };
    let words = q
        .split_whitespace()
        .filter(|w| t.contains(w) || a.contains(w))
        .count() as i64;
    // YouTube's own top hits count for a lot even when the title does not match.
    let position = match rank {
        0 => 70,
        1 => 50,
        2 => 40,
        _ => 30 - rank.min(30) as i64,
    };
    base + words * 10 + position
}

fn tab_labels(results: &Results) -> Vec<(Tab, String)> {
    Tab::ALL
        .iter()
        .map(|t| (*t, format!(" {} {} ", t.label(), results.count(*t))))
        .collect()
}

/// Which tab sits under column `x` of the tab row.
fn tab_at(results: &Results, x: u16) -> Option<Tab> {
    let mut start = 0u16;
    for (tab, label) in tab_labels(results) {
        let end = start + label.chars().count() as u16;
        if x >= start && x < end {
            return Some(tab);
        }
        start = end + 2;
    }
    None
}

/// Rows: search box, tab row, results.
fn layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(area)
}

/// Resolves the selected track's stream ahead of time, so Enter plays it sooner.
fn prewarm_selected(list: &ListItem<Status>) {
    if let Some(Status::Unknown(video)) = list.select() {
        download_manager::prewarm(&video.video_id);
    }
}

/// Rebuilds the visible list from the stored results.
fn refresh(results: &RwLock<Results>, list: &RwLock<ListItem<Status>>, reset_cursor: bool) {
    let shown = results.read().unwrap().visible();
    let mut list = list.write().unwrap();
    if reset_cursor {
        list.update(shown, 0);
    } else {
        list.update_contents(shown);
    }
    prewarm_selected(&list);
}

pub struct Search {
    pub text: String,
    pub goto: Screens,
    pub list: Arc<RwLock<ListItem<Status>>>,
    results: Arc<RwLock<Results>>,
    pub search_handle: Option<JoinHandle<()>>,
    pub action_sender: Sender<SoundAction>,
    updater: Sender<ManagerMessage>,
    /// Keys go to the search box (true) or to the results list (false).
    typing: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Local(YoutubeMusicVideoRef),
    Unknown(YoutubeMusicVideoRef),
    PlayList(YoutubeMusicPlaylistRef, Vec<YoutubeMusicVideoRef>),
}

impl ListItemAction for Status {
    fn render_style(&self, _: &str, _selected: bool) -> Style {
        Style::default().fg(THEME.text).bg(THEME.bg)
    }

    fn is_playing(&self) -> bool {
        match self {
            Self::Local(video) | Self::Unknown(video) => {
                super::widgets::is_now_playing(&video.video_id)
            }
            Self::PlayList(..) => false,
        }
    }
}

impl Screen for Search {
    fn on_mouse_press(&mut self, mouse: MouseEvent, frame_data: &Rect) -> EventResponse {
        let rows = layout(*frame_data);
        let left_down = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
        if mouse.row == rows[0].y && left_down {
            self.typing = true;
            return EventResponse::None;
        }
        if mouse.row == rows[1].y && left_down {
            let x = mouse.column.saturating_sub(rows[1].x + 1);
            let hit = tab_at(&self.results.read().unwrap(), x);
            if let Some(tab) = hit {
                self.set_tab(tab);
            }
            return EventResponse::None;
        }
        let event = self.list.write().unwrap().on_mouse_press(mouse, &rows[2]);
        if left_down && mouse.row > rows[2].y {
            self.typing = false;
        }
        match event {
            Some(ListEvent::Activate(status)) => self.execute_status(status, mouse.modifiers),
            Some(ListEvent::Menu(status)) => self.menu_for(status),
            // A playlist opens on a single click; tracks wait for a double click.
            Some(ListEvent::Select(status @ Status::PlayList(..))) => {
                self.execute_status(status, mouse.modifiers)
            }
            Some(ListEvent::Select(_)) => EventResponse::None,
            None => EventResponse::None,
        }
    }

    fn on_key_press(&mut self, key: KeyEvent, _: &Rect) -> EventResponse {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Keys that work the same in the box and in the list.
        match key.code {
            KeyCode::Esc => return ManagerMessage::ChangeState(self.goto).event(),
            KeyCode::Tab => {
                let next = self.current_tab().shift(1);
                self.set_tab(next);
                return EventResponse::None;
            }
            KeyCode::BackTab => {
                let prev = self.current_tab().shift(-1);
                self.set_tab(prev);
                return EventResponse::None;
            }
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::PageDown => self.move_cursor(10),
            _ => {}
        }
        if self.typing {
            match key.code {
                KeyCode::Down | KeyCode::Enter => {
                    if self.list.read().unwrap().len() > 0 {
                        self.typing = false;
                    }
                }
                KeyCode::Backspace => {
                    self.text.pop();
                    self.search();
                }
                KeyCode::Char('u') if ctrl => {
                    self.text.clear();
                    self.search();
                }
                KeyCode::Char(c) if !ctrl => {
                    self.text.push(c);
                    self.search();
                }
                _ => {}
            }
            return EventResponse::None;
        }
        match key.code {
            KeyCode::Up => {
                if self.list.read().unwrap().current_position() == 0 {
                    self.typing = true;
                } else {
                    self.move_cursor(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Home => self.list.write().unwrap().select_to(0),
            KeyCode::End => self.list.write().unwrap().select_to(usize::MAX),
            KeyCode::Char('/') => self.typing = true,
            KeyCode::Backspace => {
                self.typing = true;
                self.text.pop();
                self.search();
            }
            KeyCode::Enter => {
                let picked = self.list.read().unwrap().select().cloned();
                if let Some(status) = picked {
                    return self.execute_status(status, key.modifiers);
                }
            }
            KeyCode::Char('m') => {
                let picked = self.list.read().unwrap().select().cloned();
                if let Some(status) = picked {
                    return self.menu_for(status);
                }
            }
            _ => {}
        }
        EventResponse::None
    }

    fn wants_text(&self) -> bool {
        self.typing
    }

    fn hints(&self) -> Vec<Hint> {
        if self.typing {
            vec![
                Hint::new("↓", "results", KeyCode::Down),
                Hint::new("Tab", "filter", KeyCode::Tab),
            ]
        } else {
            vec![
                Hint::new("Enter", "play", KeyCode::Enter),
                Hint::new("m", "menu", KeyCode::Char('m')),
                Hint::new("/", "type", KeyCode::Char('/')),
                Hint::new("Tab", "filter", KeyCode::Tab),
            ]
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let rows = layout(area);
        let buf = frame.buffer_mut();
        let base = Style::default().bg(THEME.bg);
        let box_style = if self.typing {
            base.fg(THEME.text).bg(THEME.hover)
        } else {
            base.fg(THEME.text_dim)
        };
        let width = rows[0].width as usize;
        let shown = if self.text.is_empty() && !self.typing {
            "Click here or press / to search".to_owned()
        } else {
            format!("{}{}", self.text, if self.typing { "▏" } else { "" })
        };
        buf.set_string(rows[0].x, rows[0].y, " ⌕ ", base.fg(THEME.accent).bg(box_style.bg.unwrap_or(THEME.bg)));
        buf.set_string(rows[0].x + 3, rows[0].y, fit(&shown, width.saturating_sub(3)), box_style);

        let results = self.results.read().unwrap();
        let mut x = rows[1].x + 1;
        for (tab, label) in tab_labels(&results) {
            let w = label.chars().count() as u16;
            let rect = Rect { x, y: rows[1].y, width: w, height: 1 };
            let style = if tab == results.tab {
                base.fg(THEME.text).bg(THEME.accent_bg).add_modifier(Modifier::BOLD)
            } else if hovered(rect, rect.y) {
                base.fg(THEME.text).bg(THEME.hover)
            } else {
                base.fg(THEME.text_dim)
            };
            buf.set_stringn(x, rows[1].y, &label, (rows[1].x + rows[1].width).saturating_sub(x) as usize, style);
            x += w + 2;
        }
        drop(results);

        let mut items = self.list.write().unwrap();
        items.placeholder = if self.text.trim().is_empty() {
            "Type to search songs, videos and playlists".to_owned()
        } else if self.search_handle.as_ref().map(|h| !h.is_finished()).unwrap_or(false) {
            "Searching…".to_owned()
        } else {
            "No results".to_owned()
        };
        items.render(rows[2], frame.buffer_mut());
    }

    fn handle_global_message(&mut self, _: super::ManagerMessage) -> EventResponse {
        EventResponse::None
    }

    fn close(&mut self, _: Screens) -> EventResponse {
        EventResponse::None
    }

    fn open(&mut self) -> EventResponse {
        self.typing = true;
        EventResponse::None
    }
}

impl Search {
    pub async fn new(action_sender: Sender<SoundAction>, updater: Sender<ManagerMessage>) -> Self {
        Self {
            text: String::new(),
            list: Arc::new(RwLock::new(ListItem::new(" Results ".to_string()))),
            results: Arc::new(RwLock::new(Results::default())),
            goto: Screens::MusicPlayer,
            search_handle: None,
            action_sender,
            updater,
            typing: true,
        }
    }

    /// Context menu for a result: tracks get one, playlists open instead.
    fn menu_for(&self, status: Status) -> EventResponse {
        match status {
            Status::Local(video) | Status::Unknown(video) => {
                ManagerMessage::OpenMenu(MenuTarget::Video(video, None)).event()
            }
            playlist => self.execute_status(playlist, KeyModifiers::NONE),
        }
    }

    fn current_tab(&self) -> Tab {
        self.results.read().unwrap().tab
    }

    fn set_tab(&mut self, tab: Tab) {
        self.results.write().unwrap().tab = tab;
        refresh(&self.results, &self.list, true);
    }

    fn move_cursor(&self, delta: isize) {
        let mut list = self.list.write().unwrap();
        let position = list.current_position() as isize + delta;
        list.select_to(position.max(0) as usize);
        prewarm_selected(&list);
    }

    /// Shows local matches at once, then merges the online results as they arrive.
    fn search(&mut self) {
        let query = self.text.trim().to_owned();
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }
        {
            let needle = query.to_lowercase();
            let db = DATABASE.read().unwrap();
            let mut results = self.results.write().unwrap();
            results.query = query.clone();
            results.entries.clear();
            for (rank, video) in db
                .iter()
                .filter(|x| {
                    x.title.to_lowercase().contains(&needle)
                        || x.author.to_lowercase().contains(&needle)
                })
                .take(100)
                .enumerate()
            {
                results.add(
                    Kind::Song,
                    Status::Local(video.clone()),
                    format_track(video),
                    &video.title,
                    &video.author,
                    rank,
                );
            }
        }
        refresh(&self.results, &self.list, true);

        if query.is_empty() {
            return;
        }
        let results = self.results.clone();
        let list = self.list.clone();
        self.search_handle = Some(run_service(async move {
            // The shared session: signed in when there is one, so results match the account.
            let Some(api) = crate::api_instance().await else {
                return;
            };
            // Wait a little so fast typing does not hit the API for every key.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let escaped = query.replace('\\', "\\\\").replace('"', "\\\"");
            // The songs shelf and the unfiltered page are fetched at the same time.
            let songs_task = {
                let api = api.clone();
                let escaped = escaped.clone();
                tokio::spawn(async move { api.search_songs(&escaped).await })
            };
            let all = match api.search_unfiltered(&escaped).await {
                Ok(all) => all,
                Err(e) => {
                    error!("{e:?}");
                    SearchResults {
                        videos: Vec::new(),
                        playlists: Vec::new(),
                    }
                }
            };

            let songs = match songs_task.await {
                Ok(Ok(songs)) => songs,
                other => {
                    error!("song search failed: {other:?}");
                    Vec::new()
                }
            };

            let local: HashSet<String> = DATABASE
                .read()
                .unwrap()
                .iter()
                .map(|x| x.video_id.clone())
                .collect();
            let video_status = |v: &YoutubeMusicVideoRef| {
                if local.contains(&v.video_id) {
                    Status::Local(v.clone())
                } else {
                    Status::Unknown(v.clone())
                }
            };
            {
                let mut r = results.write().unwrap();
                if r.query != query {
                    return;
                }
                for (rank, v) in songs.iter().enumerate() {
                    r.add(
                        Kind::Song,
                        video_status(v),
                        format_track(v),
                        &v.title,
                        &v.author,
                        rank,
                    );
                }
                // The unfiltered search also holds the songs; `add` skips those.
                for (rank, v) in all.videos.iter().enumerate() {
                    r.add(
                        Kind::Video,
                        video_status(v),
                        format_track(v),
                        &v.title,
                        &v.author,
                        rank,
                    );
                }
                // Playlist tracks are fetched only when the playlist is opened.
                for (rank, playlist) in all.playlists.iter().enumerate() {
                    r.add(
                        Kind::Playlist,
                        Status::PlayList(playlist.clone(), Vec::new()),
                        format!(
                            "{}\t{}",
                            to_bidi_string(&playlist.name),
                            to_bidi_string(&playlist.subtitle)
                        ),
                        &playlist.name,
                        &playlist.subtitle,
                        rank,
                    );
                }
            }
            refresh(&results, &list, false);
        }));
    }

    pub fn execute_status(&self, e: Status, _modifiers: KeyModifiers) -> EventResponse {
        match e {
            Status::Local(e) | Status::Unknown(e) => {
                // The player streams it straight into RAM; no second download to disk.
                self.action_sender.send(SoundAction::PlayNow(e)).unwrap();
                EventResponse::None
            }
            Status::PlayList(playlist, _) => {
                let updater = self.updater.clone();
                let _ = updater.send(ManagerMessage::Toast("Opening playlist…".to_owned()));
                run_service(async move {
                    let Some(api) = crate::api_instance().await else {
                        return;
                    };
                    match api.get_playlist(&playlist, 10).await {
                        Ok(videos) => {
                            let _ = updater.send(
                                ManagerMessage::Inspect(
                                    playlist.name,
                                    Screens::Search,
                                    videos,
                                    String::new(),
                                )
                                .pass_to(Screens::PlaylistViewer),
                            );
                        }
                        Err(e) => error!("{e:?}"),
                    }
                });
                EventResponse::None
            }
        }
    }
}
