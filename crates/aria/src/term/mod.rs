pub mod device_lost;
pub mod item_list;
pub mod list_selector;
pub mod menu;
pub mod music_player;
pub mod playlist;
pub mod playlist_view;
pub mod queue_panel;
pub mod search;
pub mod theme;
pub mod view;
pub mod widgets;

use std::{
    hash::{Hash, Hasher},
    io::{self, Write},
    time::{Duration, Instant},
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use flume::{Receiver, Sender};
use once_cell::sync::Lazy;
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear},
    Frame, Terminal,
};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    is_shutdown_sent, shutdown, state, structures::sound_action::SoundAction,
    systems::player::PlayerState,
};

use self::{
    device_lost::DeviceLost,
    item_list::ListItem,
    menu::{Menu, MenuAction, MenuTarget},
    playlist::Chooser,
    playlist_view::PlaylistView,
    queue_panel::{Panel, PanelMouse, SidePanel},
    search::Search,
    theme::THEME,
    widgets::{fit, fit_right, hovered},
};

/// A keyboard hint of the bottom bar. Clicking its left half sends `left`,
/// its right half sends `right` (the same key for single hints).
#[derive(Clone, Copy)]
pub struct Hint {
    pub key: &'static str,
    pub desc: &'static str,
    pub left: KeyEvent,
    pub right: KeyEvent,
}

impl Hint {
    pub fn new(key: &'static str, desc: &'static str, code: KeyCode) -> Self {
        let event = KeyEvent::new(code, KeyModifiers::NONE);
        Self {
            key,
            desc,
            left: event,
            right: event,
        }
    }

    pub fn split(key: &'static str, desc: &'static str, left: KeyEvent, right: KeyEvent) -> Self {
        Self {
            key,
            desc,
            left,
            right,
        }
    }

    fn width(&self) -> u16 {
        (self.key.chars().count() + 1 + self.desc.chars().count()) as u16
    }
}

// A trait to handle the different screens
pub trait Screen {
    fn on_mouse_press(&mut self, mouse_event: MouseEvent, frame_data: &Rect) -> EventResponse;
    fn on_key_press(&mut self, key: KeyEvent, frame_data: &Rect) -> EventResponse;
    fn render(&mut self, frame: &mut Frame, area: Rect);
    fn handle_global_message(&mut self, message: ManagerMessage) -> EventResponse;
    fn close(&mut self, new_screen: Screens) -> EventResponse;
    fn open(&mut self) -> EventResponse;
    /// True while the screen takes typed text, so letters and Space go to it.
    fn wants_text(&self) -> bool {
        false
    }
    /// Screen-specific keys shown in the bottom bar.
    fn hints(&self) -> Vec<Hint> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub enum EventResponse {
    Message(Vec<ManagerMessage>),
    None,
}

// A message that can be sent to the manager
#[derive(Debug, Clone)]
pub enum ManagerMessage {
    Error(String, Box<Option<ManagerMessage>>),
    PassTo(Screens, Box<ManagerMessage>),
    /// Opens a playlist: name, screen to go back to, tracks, account browse id (or "").
    Inspect(String, Screens, Vec<YoutubeMusicVideoRef>, String),
    ChangeState(Screens),
    SearchFrom(Screens),
    Autoplay(String),
    Status(String),
    #[allow(dead_code)]
    PlaylistFrom(Screens),
    RestartPlayer,
    Quit,
    AddElementToChooser((String, Vec<YoutubeMusicVideoRef>)),
    /// A short notice shown for a few seconds.
    Toast(String),
    OpenMenu(MenuTarget),
    /// Lyrics fetched for a track (`None` when it has none).
    Lyrics(String, Option<String>),
    /// A playlist loaded from the account: name, browse id, tracks.
    AddPlaylist(String, String, Vec<YoutubeMusicVideoRef>),
    /// A change made on the account, to mirror in the lists.
    Library(crate::account::LibraryChange),
}

impl ManagerMessage {
    pub fn pass_to(self, screen: Screens) -> Self {
        Self::PassTo(screen, Box::new(self))
    }
    pub fn event(self) -> EventResponse {
        EventResponse::Message(vec![self])
    }
}

// The different screens
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Screens {
    MusicPlayer = 0x0,
    Playlist = 0x1,
    Search = 0x2,
    DeviceLost = 0x3,
    PlaylistViewer = 0x4,
}

const SIDEBAR_WIDTH: u16 = 24;
/// Below this size the layout does not fit and a notice is shown instead.
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 12;
/// Below this terminal width the sidebar hides and the top bar shows the sections.
const NARROW_WIDTH: u16 = 80;
const TOP_BAR_HEIGHT: u16 = 1;
/// Border, four rows (title, buttons, progress, hints), border.
const PLAYER_BAR_HEIGHT: u16 = 6;
const PANEL_WIDTH: u16 = 44;
const PANEL_BUTTON_WIDTH: u16 = 8;
/// Width of the "Ctrl+C  Quit" group at the right end of the top bar.
const QUIT_GROUP_WIDTH: u16 = 13;
const TOAST_TIME: Duration = Duration::from_secs(3);
/// Cover size in cells: 8 x 4 cells show 8 x 8 pixels with half blocks.
const COVER_WIDTH: u16 = 8;

/// Snapshot of what the player bar needs, taken before rendering so the
/// manager does not borrow the player state while rendering screens.
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub elapsed: u64,
    pub total: u64,
    pub ratio: f64,
    pub paused: bool,
}

const SIDEBAR_SECTIONS: [(Screens, &str); 4] = [
    (Screens::Playlist, " F1 Home"),
    (Screens::Search, " F2 Search"),
    (Screens::MusicPlayer, " F3 Now playing"),
    (Screens::PlaylistViewer, " F4 Playlist"),
];

fn section_name(screen: Screens) -> &'static str {
    match screen {
        Screens::Playlist => "Home",
        Screens::Search => "Search",
        Screens::MusicPlayer => "Now playing",
        Screens::PlaylistViewer => "Playlist",
        Screens::DeviceLost => "Error",
    }
}

/// Covers are drawn only on terminals with 24-bit color.
static TRUECOLOR: Lazy<bool> = Lazy::new(|| {
    std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
});

/// The area inside a box drawn on `outer`.
fn shrink(outer: Rect) -> Rect {
    Rect {
        x: outer.x + 1,
        y: outer.y + 1,
        width: outer.width.saturating_sub(2),
        height: outer.height.saturating_sub(2),
    }
}

/// The box that surrounds `inner`.
fn grow(inner: Rect) -> Rect {
    Rect {
        x: inner.x.saturating_sub(1),
        y: inner.y.saturating_sub(1),
        width: inner.width + 2,
        height: inner.height + 2,
    }
}

/// Draws a rounded box on `outer` with `title` on its top border.
fn draw_box(f: &mut Frame, outer: Rect, title: &str) {
    if outer.width < 2 || outer.height < 2 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME.border))
        .title(ratatui::text::Span::styled(
            title.to_string(),
            Style::default().fg(THEME.text).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(THEME.bg));
    f.render_widget(block, outer);
}

/// Where everything goes on screen. All areas but the top bar are box insides.
#[derive(Clone, Copy, Default)]
struct Shell {
    top: Rect,
    /// Zero-sized when the terminal is narrow.
    side: Rect,
    content: Rect,
    /// Zero-sized while the side panel is closed.
    panel: Rect,
    /// The player box; its inner rows are `bar.y + 1 ..= bar.y + 4`.
    bar: Rect,
}

impl Shell {
    fn new(size: Rect, panel_open: bool) -> Self {
        let [top, rest] = split_y_start(size, TOP_BAR_HEIGHT);
        let [rest, bar] = split_y(rest, PLAYER_BAR_HEIGHT);
        let side_width = if size.width < NARROW_WIDTH {
            0
        } else {
            SIDEBAR_WIDTH
        };
        let [side, mut content] = split_x_start(rest, side_width);
        let mut panel = Rect::default();
        if panel_open && content.width >= PANEL_WIDTH + 40 {
            panel = shrink(Rect {
                x: content.x + content.width - PANEL_WIDTH,
                width: PANEL_WIDTH,
                ..content
            });
            content.width -= PANEL_WIDTH;
        }
        Self {
            top,
            side: if side_width == 0 {
                Rect::default()
            } else {
                shrink(side)
            },
            content: shrink(content),
            panel,
            bar,
        }
    }

    fn narrow(&self) -> bool {
        self.side.width == 0
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Zone {
    Top,
    Side,
    Content,
    Panel,
    Bar,
}

#[derive(Clone, Copy, PartialEq)]
enum BarControl {
    Progress,
    Volume,
}

#[derive(Clone, Copy, PartialEq)]
enum PlayerButton {
    Previous,
    PlayPause,
    Next,
    Shuffle,
    Repeat,
}

/// Hit areas of the player box.
struct PlayerLayout {
    cover: Option<Rect>,
    title: Rect,
    buttons: Vec<(Rect, PlayerButton)>,
    volume: Rect,
    progress: Rect,
    hints: Rect,
}

const BUTTONS: [(PlayerButton, u16); 5] = [
    (PlayerButton::Previous, 4),
    (PlayerButton::PlayPause, 5),
    (PlayerButton::Next, 4),
    (PlayerButton::Shuffle, 11),
    (PlayerButton::Repeat, 15),
];
const VOLUME_WIDTH: u16 = 15;
/// Cells of the volume gauge, after the "vol " label.
const VOLUME_CELLS: u16 = 7;

fn player_layout(bar: Rect) -> PlayerLayout {
    let row = |n: u16| bar.y + n;
    let show_cover = *TRUECOLOR && bar.width > 60;
    let left = if show_cover {
        bar.x + 2 + COVER_WIDTH + 1
    } else {
        bar.x + 2
    };
    let right = bar.x + bar.width.saturating_sub(2);
    let width = right.saturating_sub(left);
    let line = |n: u16| Rect {
        x: left,
        y: row(n),
        width,
        height: 1,
    };
    let mut x = left;
    let buttons = BUTTONS
        .iter()
        .map(|(button, w)| {
            let rect = Rect {
                x,
                y: row(2),
                width: *w,
                height: 1,
            };
            x += w + if *button == PlayerButton::Next { 2 } else { 1 };
            (rect, *button)
        })
        .collect();
    PlayerLayout {
        cover: show_cover.then_some(Rect {
            x: bar.x + 2,
            y: row(1),
            width: COVER_WIDTH,
            height: 4,
        }),
        title: line(1),
        buttons,
        volume: Rect {
            x: right.saturating_sub(VOLUME_WIDTH),
            y: row(2),
            width: VOLUME_WIDTH,
            height: 1,
        },
        progress: line(3),
        hints: line(4),
    }
}

/// One-line label of a track, as tab-separated fields: `title\tartist\tduration`.
pub fn format_track(video: &ytpapi2::YoutubeMusicVideoRef) -> String {
    format!(
        "{}\t{}\t{}",
        crate::utils::to_bidi_string(&video.title),
        crate::utils::to_bidi_string(&video.author),
        video.duration
    )
}

/// Which sidebar entry is under `y`, if any.
fn sidebar_target(y: u16, area: Rect) -> Option<Screens> {
    // Row 0 is a blank line.
    let row = y.saturating_sub(area.y) as usize;
    row.checked_sub(1)
        .and_then(|index| SIDEBAR_SECTIONS.get(index))
        .map(|(screen, _)| *screen)
}

/// The section tabs of the top bar, shown when the sidebar is hidden.
fn top_tabs(top: Rect) -> Vec<(Rect, Screens)> {
    let mut x = top.x + 7;
    SIDEBAR_SECTIONS
        .iter()
        .map(|(screen, _)| {
            let width = section_name(*screen).chars().count() as u16 + 2;
            let rect = Rect {
                x,
                y: top.y,
                width,
                height: 1,
            };
            x += width;
            (rect, *screen)
        })
        .collect()
}

/// The panel toggle button, just left of "Quit" in the top bar.
fn panel_button(top: Rect) -> Rect {
    Rect {
        x: top.x + top.width.saturating_sub(QUIT_GROUP_WIDTH + PANEL_BUTTON_WIDTH),
        width: PANEL_BUTTON_WIDTH,
        ..top
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

/// Keys every screen shares, shown after the screen's own hints.
fn global_hints() -> Vec<Hint> {
    vec![
        Hint::new("Space", "play/pause", KeyCode::Char(' ')),
        Hint::split(
            "←/→",
            "seek",
            key(KeyCode::Left, KeyModifiers::NONE),
            key(KeyCode::Right, KeyModifiers::NONE),
        ),
        Hint::split(
            "Ctrl+←/→",
            "track",
            key(KeyCode::Left, KeyModifiers::CONTROL),
            key(KeyCode::Right, KeyModifiers::CONTROL),
        ),
        Hint::split(
            "-/+",
            "volume",
            key(KeyCode::Char('-'), KeyModifiers::NONE),
            key(KeyCode::Char('+'), KeyModifiers::NONE),
        ),
        Hint::new("?", "help", KeyCode::Char('?')),
    ]
}

const HELP: [(&str, &str); 8] = [
    ("Playback", "Space play/pause · ←/→ seek · Ctrl+←/→ track · -/+ volume · s shuffle · r repeat"),
    ("Sections", "F1 Home · F2 Search · F3 Now playing · F4 Playlist · Esc back"),
    ("Panel", "F6 show/hide the right panel · F7 lyrics · F8 sign in"),
    ("Lists", "↑/↓ PgUp/PgDn move · Enter or double click play · m or right click menu"),
    ("Queue", "Del removes the selected track (on Now playing)"),
    ("Account", "m: like, add to or remove from a playlist · Home: n new playlist · Del delete · F5 sync"),
    ("Search", "type to search · ↓ to the results · / back to the box · Tab filter"),
    ("Mouse", "wheel scrolls · drag the scrollbar or the progress bar · click a hint"),
];

/// Copies `text` with the system clipboard tool, or through the terminal (OSC 52).
fn copy_to_clipboard(text: &str) -> bool {
    use std::process::{Command, Stdio};
    let tools: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (tool, args) in tools {
        let child = Command::new(tool)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
    }
    let mut out = io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).is_ok() && out.flush().is_ok()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// The screen manager that handles the different screens
pub struct Manager {
    music_player: PlayerState,
    chooser: Chooser,
    search: Search,
    device_lost: DeviceLost,
    current_screen: Screens,
    playlist_viewer: PlaylistView,
    mouse_pos: Option<(u16, u16)>,
    panel_open: bool,
    panel: SidePanel,
    signed_in: bool,
    menu: Option<Menu>,
    help_open: bool,
    toast: Option<(String, Instant)>,
    /// Track whose cover is shown, its pixels, and when the disk was last checked.
    cover: Option<(String, Option<Vec<u8>>, Instant)>,
    /// Zone where the left button went down; drags go there until it is released.
    drag: Option<Zone>,
    /// The player control held with the mouse: a drag only ever moves that one.
    bar_drag: Option<BarControl>,
    /// Where the progress knob is being dragged to; the seek happens on release.
    seek_preview: Option<f64>,
    /// Whether the last input came from the mouse (menus open at the pointer).
    mouse_input: bool,
    shell: Shell,
    /// Where the open menu was placed, so its submenu opens at the same spot.
    menu_position: (u16, u16),
    saved_signature: u64,
    window_title: String,
}

impl Manager {
    pub async fn new(action_sender: Sender<SoundAction>, mut music_player: PlayerState) -> Self {
        let updater = music_player.updater.clone();
        let saved = state::load();
        if let Some(volume) = saved.volume {
            music_player.sink.set_volume(volume);
        }
        music_player.repeat = saved.repeat;
        if !saved.queue.is_empty() {
            SoundAction::RestoreQueue(saved.queue, saved.current)
                .apply_sound_action(&mut music_player);
            // Pick the track up where it was left, once the user presses play.
            music_player.resume_at =
                (saved.position > 0).then(|| Duration::from_secs(saved.position));
        }
        let mut manager = Self {
            music_player,
            chooser: Chooser::new(Screens::MusicPlayer, updater.clone()),
            playlist_viewer: PlaylistView {
                sender: action_sender.clone(),
                items: ListItem::new(" Playlist ".to_owned()),
                goto: Screens::Playlist,
                videos: Vec::new(),
                name: String::new(),
                browse_id: String::new(),
            },
            search: Search::new(action_sender, updater).await,
            current_screen: Screens::Playlist,
            device_lost: DeviceLost(Vec::new(), None),
            mouse_pos: None,
            panel_open: saved.panel_open,
            panel: SidePanel::new(if saved.lyrics_tab {
                Panel::Lyrics
            } else {
                Panel::Queue
            }),
            signed_in: !crate::is_guest(),
            menu: None,
            help_open: false,
            toast: None,
            cover: None,
            drag: None,
            bar_drag: None,
            seek_preview: None,
            mouse_input: false,
            shell: Shell::default(),
            menu_position: (0, 0),
            saved_signature: 0,
            window_title: String::new(),
        };
        // Reopen the playlist and the section the last session ended on.
        if let Some(view) = saved.viewed {
            // The saved copy shows at once; the account's current one replaces it.
            crate::account::refresh_playlist(
                manager.music_player.updater.clone(),
                view.browse_id.clone(),
            );
            manager.playlist_viewer.handle_global_message(ManagerMessage::Inspect(
                view.name,
                Screens::Playlist,
                view.videos,
                view.browse_id,
            ));
        }
        manager.current_screen = match saved.screen {
            0 => Screens::MusicPlayer,
            2 => Screens::Search,
            4 if !manager.playlist_viewer.videos.is_empty() => Screens::PlaylistViewer,
            _ => Screens::Playlist,
        };
        manager.saved_signature = manager.state_signature();
        manager
    }

    pub fn current_screen(&mut self) -> &mut dyn Screen {
        self.get_screen(self.current_screen)
    }

    pub fn get_screen(&mut self, screen: Screens) -> &mut dyn Screen {
        match screen {
            Screens::MusicPlayer => &mut self.music_player,
            Screens::Playlist => &mut self.chooser,
            Screens::Search => &mut self.search,
            Screens::DeviceLost => &mut self.device_lost,
            Screens::PlaylistViewer => &mut self.playlist_viewer,
        }
    }

    fn screen_ref(&self) -> &dyn Screen {
        match self.current_screen {
            Screens::MusicPlayer => &self.music_player,
            Screens::Playlist => &self.chooser,
            Screens::Search => &self.search,
            Screens::DeviceLost => &self.device_lost,
            Screens::PlaylistViewer => &self.playlist_viewer,
        }
    }

    pub fn set_current_screen(&mut self, screen: Screens) {
        self.current_screen = screen;
        let k = self.current_screen().open();
        self.handle_event(k);
    }

    pub fn handle_event(&mut self, event: EventResponse) -> bool {
        match event {
            EventResponse::Message(messages) => {
                for message in messages {
                    if self.handle_manager_message(message) {
                        return true;
                    }
                }
            }
            EventResponse::None => {}
        }
        false
    }

    fn toast(&mut self, text: &str) {
        self.toast = Some((text.to_owned(), Instant::now()));
    }

    pub fn handle_manager_message(&mut self, e: ManagerMessage) -> bool {
        match e {
            ManagerMessage::PassTo(e, a) => {
                let rs = self.get_screen(e).handle_global_message(*a);
                self.handle_event(rs);
            }
            ManagerMessage::Quit => {
                let c = self.current_screen;
                self.current_screen().close(c);
                return true;
            }
            ManagerMessage::ChangeState(e) => {
                self.current_screen().close(e);
                self.set_current_screen(e);
            }
            ManagerMessage::SearchFrom(e) => {
                self.current_screen().close(Screens::Search);
                self.search.goto = e;
                self.set_current_screen(Screens::Search);
            }
            ManagerMessage::Status(text) => {
                self.signed_in = !crate::is_guest();
                self.chooser.set_status(text)
            }
            ManagerMessage::Toast(text) => self.toast(&text),
            ManagerMessage::OpenMenu(target) => self.open_menu(target),
            ManagerMessage::Lyrics(id, text) => self.panel.set_lyrics(id, text),
            ManagerMessage::Library(change) => {
                self.chooser.apply_change(&change);
                self.playlist_viewer.apply_change(&change);
            }
            ManagerMessage::Autoplay(video_id) => {
                let sender = self.music_player.soundaction_sender.clone();
                crate::run_service(async move {
                    let Some(api) = crate::api_instance().await else {
                        return;
                    };
                    match api.get_radio(&video_id).await {
                        Ok(videos) if !videos.is_empty() => {
                            let videos = videos
                                .into_iter()
                                .filter(|v| v.video_id != video_id)
                                .collect();
                            let _ = sender.send(SoundAction::AddVideosToQueue(videos));
                        }
                        Ok(_) => log::info!("autoplay: radio returned no tracks"),
                        Err(e) => log::error!("autoplay error: {e:?}"),
                    }
                });
            }
            ManagerMessage::PlaylistFrom(e) => {
                self.current_screen().close(Screens::Playlist);
                self.chooser.goto = e;
                self.set_current_screen(Screens::Playlist);
            }
            e => {
                return self.handle_manager_message(ManagerMessage::PassTo(
                    Screens::DeviceLost,
                    Box::new(ManagerMessage::Error(
                        format!(
                        "Invalid manager message (Forward the message to a screen maybe):\n{e:?}"
                    ),
                        Box::new(None),
                    )),
                ));
            }
        }
        false
    }

    fn open_menu(&mut self, target: MenuTarget) {
        let (x, y) = match self.mouse_pos {
            Some(position) if self.mouse_input => position,
            _ => (self.shell.content.x + 4, self.shell.content.y + 2),
        };
        let (video, queue_index, playlist) = match target {
            MenuTarget::Queue(index) => match self.music_player.list.get(index) {
                Some(video) => (video.clone(), Some(index), None),
                None => return,
            },
            MenuTarget::Video(video, playlist) => (video, None, playlist),
        };
        let liked = crate::account::is_liked(&video.video_id);
        self.menu_position = (x, y);
        self.menu = Some(Menu::for_track(video, queue_index, playlist, liked, x, y));
    }

    fn run_menu_action(&mut self, action: MenuAction) {
        let player = &mut self.music_player;
        match action {
            MenuAction::PlayNow(video) => SoundAction::PlayNow(video).apply_sound_action(player),
            MenuAction::PlayNext(video) => {
                SoundAction::PlayNext(video).apply_sound_action(player);
                self.toast("Plays next");
            }
            MenuAction::AddToQueue(video) => {
                let queued = player.list.iter().any(|v| v.video_id == video.video_id);
                SoundAction::AddVideosToQueue(vec![video]).apply_sound_action(player);
                self.toast(if queued { "Already in the queue" } else { "Added to queue" });
            }
            MenuAction::PlayIndex(index) => player.activate(index),
            MenuAction::RemoveAt(index) => {
                SoundAction::RemoveAt(index).apply_sound_action(player);
                self.toast("Removed from queue");
            }
            MenuAction::SetLiked(video, liked) => {
                crate::account::set_liked(player.updater.clone(), video, liked)
            }
            MenuAction::ChoosePlaylist(video) => {
                let playlists = crate::account::playlists();
                if playlists.is_empty() {
                    self.toast(if self.signed_in {
                        "No playlists yet: create one in Home with n"
                    } else {
                        "Sign in first (F8)"
                    });
                } else {
                    let (x, y) = self.menu_position;
                    self.menu = Some(Menu::for_playlists(video, playlists, x, y));
                }
            }
            MenuAction::AddToPlaylist {
                browse_id,
                name,
                video,
            } => crate::account::add_to_playlist(player.updater.clone(), browse_id, name, video),
            MenuAction::RemoveFromPlaylist {
                browse_id,
                video_id,
            } => crate::account::remove_from_playlist(player.updater.clone(), browse_id, video_id),
            MenuAction::OpenInBrowser(id) => {
                crate::open_browser(&format!("https://music.youtube.com/watch?v={id}"));
                self.toast("Opened in the browser");
            }
            MenuAction::CopyLink(id) => {
                let copied = copy_to_clipboard(&format!("https://music.youtube.com/watch?v={id}"));
                self.toast(if copied { "Link copied" } else { "Could not copy the link" });
            }
        }
    }

    /// Starts the in-app sign-in and shows its progress on the Home screen.
    fn sign_in(&mut self) {
        let expired = crate::SESSION_EXPIRED.load(std::sync::atomic::Ordering::Relaxed);
        if self.signed_in && !expired {
            self.toast("Already signed in");
            return;
        }
        crate::start_login(self.music_player.updater.clone());
        self.handle_event(ManagerMessage::ChangeState(Screens::Playlist).event());
    }

    /// Everything a key press can do. Returns true to quit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.mouse_input = false;
        if matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
            && key.modifiers == KeyModifiers::CONTROL
        {
            return true;
        }
        if self.help_open {
            self.help_open = false;
            return false;
        }
        if let Some(menu) = &mut self.menu {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => menu.move_by(-1),
                KeyCode::Down | KeyCode::Char('j') => menu.move_by(1),
                KeyCode::Enter => {
                    let action = menu.action(menu.selected);
                    self.menu = None;
                    if let Some(action) = action {
                        self.run_menu_action(action);
                    }
                }
                _ => self.menu = None,
            }
            return false;
        }

        let jump = match key.code {
            KeyCode::F(1) => Some(Screens::Playlist),
            KeyCode::F(2) => Some(Screens::Search),
            KeyCode::F(3) => Some(Screens::MusicPlayer),
            KeyCode::F(4) => Some(Screens::PlaylistViewer),
            _ => None,
        };
        if let Some(screen) = jump {
            return self.handle_event(ManagerMessage::ChangeState(screen).event());
        }
        match key.code {
            KeyCode::F(6) => {
                self.panel_open = !self.panel_open;
                return false;
            }
            KeyCode::F(7) => {
                if self.panel_open && self.panel.tab == Panel::Lyrics {
                    self.panel.tab = Panel::Queue;
                } else {
                    self.panel_open = true;
                    self.panel.tab = Panel::Lyrics;
                }
                return false;
            }
            KeyCode::F(8) => {
                self.sign_in();
                return false;
            }
            _ => {}
        }

        if !self.screen_ref().wants_text() {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let player = &mut self.music_player;
            let action = match key.code {
                KeyCode::Char(' ') => Some(SoundAction::PlayPause),
                KeyCode::Left if ctrl => Some(SoundAction::Previous(1)),
                KeyCode::Right if ctrl => Some(SoundAction::Next(1)),
                KeyCode::Left => Some(SoundAction::Backward),
                KeyCode::Right => Some(SoundAction::Forward),
                KeyCode::Char('+') | KeyCode::Char('=') => Some(SoundAction::Plus),
                KeyCode::Char('-') => Some(SoundAction::Minus),
                KeyCode::Char('s') if !ctrl => Some(SoundAction::ShuffleRest),
                KeyCode::Char('r') if !ctrl => Some(SoundAction::CycleRepeat),
                _ => None,
            };
            if let Some(action) = action {
                let notice = match &action {
                    SoundAction::ShuffleRest => Some("Queue shuffled".to_owned()),
                    SoundAction::CycleRepeat => {
                        Some(format!("Repeat: {}", player.repeat.next().label()))
                    }
                    _ => None,
                };
                action.apply_sound_action(player);
                if let Some(notice) = notice {
                    self.toast(&notice);
                }
                return false;
            }
            if key.code == KeyCode::Char('?') {
                self.help_open = true;
                return false;
            }
        }

        let content = self.shell.content;
        let response = self.current_screen().on_key_press(key, &content);
        self.handle_event(response)
    }

    fn zone_at(&self, x: u16, y: u16) -> Option<Zone> {
        let shell = &self.shell;
        [
            (shell.top, Zone::Top),
            (
                if shell.narrow() {
                    Rect::default()
                } else {
                    grow(shell.side)
                },
                Zone::Side,
            ),
            (shell.panel, Zone::Panel),
            (shell.bar, Zone::Bar),
            (grow(shell.content), Zone::Content),
        ]
        .into_iter()
        .find(|(rect, _)| rect.width > 0 && rect_contains(rect, x, y, 0))
        .map(|(_, zone)| zone)
    }

    /// Everything a mouse event can do. Returns true to quit.
    fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let (x, y) = (mouse.column, mouse.row);
        let down = matches!(mouse.kind, MouseEventKind::Down(_));
        if down {
            self.mouse_input = true;
        }
        if self.help_open {
            if down {
                self.help_open = false;
            }
            return false;
        }
        if let Some(menu) = &self.menu {
            if down {
                let screen = self.shell_screen();
                let action = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    .then(|| menu.item_at(screen, x, y))
                    .flatten()
                    .and_then(|index| menu.action(index));
                if !(menu.contains(screen, x, y) && action.is_none()) {
                    self.menu = None;
                }
                if let Some(action) = action {
                    self.run_menu_action(action);
                }
            }
            return false;
        }

        // A drag (and the release) belongs to where the button went down.
        let zone = match mouse.kind {
            MouseEventKind::Drag(_) | MouseEventKind::Up(_) => {
                let zone = self.drag;
                if matches!(mouse.kind, MouseEventKind::Up(_)) {
                    self.drag = None;
                }
                zone
            }
            MouseEventKind::Moved => None,
            _ => self.zone_at(x, y),
        };
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.drag = zone;
        }
        let Some(zone) = zone else {
            return false;
        };

        match zone {
            Zone::Top => {
                if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                    return false;
                }
                let top = self.shell.top;
                if x + 7 >= top.x + top.width {
                    return true;
                }
                if rect_contains(&panel_button(top), x, y, 0) {
                    self.panel_open = !self.panel_open;
                } else if self.shell.narrow() {
                    if let Some((_, screen)) =
                        top_tabs(top).into_iter().find(|(r, _)| rect_contains(r, x, y, 0))
                    {
                        return self.handle_event(ManagerMessage::ChangeState(screen).event());
                    }
                }
            }
            Zone::Side => {
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                    let side = self.shell.side;
                    if y + 1 == side.y + side.height {
                        self.sign_in();
                    } else if let Some(target) = sidebar_target(y, side) {
                        return self.handle_event(ManagerMessage::ChangeState(target).event());
                    }
                }
            }
            Zone::Panel => match self.panel.mouse(mouse, self.shell.panel, &self.music_player) {
                PanelMouse::Play(index) => self.music_player.activate(index),
                PanelMouse::Menu(index) => self.open_menu(MenuTarget::Queue(index)),
                PanelMouse::None => {}
            },
            Zone::Bar => return self.player_bar_mouse(mouse),
            Zone::Content => {
                let content = self.shell.content;
                let response = self.current_screen().on_mouse_press(mouse, &content);
                return self.handle_event(response);
            }
        }
        false
    }

    /// The whole terminal, for popups.
    fn shell_screen(&self) -> Rect {
        let shell = &self.shell;
        Rect {
            x: 0,
            y: 0,
            width: shell.top.width,
            height: shell.bar.y + shell.bar.height,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        let mut hints = self.screen_ref().hints();
        if self.screen_ref().wants_text() {
            hints.push(Hint::new("Esc", "back", KeyCode::Esc));
        } else {
            hints.extend(global_hints());
        }
        hints
    }

    /// Handles the mouse over the player box. Returns true to quit.
    fn player_bar_mouse(&mut self, mouse: MouseEvent) -> bool {
        let layout = player_layout(self.shell.bar);
        let (x, y) = (mouse.column, mouse.row);
        let inside = |rect: &Rect| rect_contains(rect, x, y, 0);
        let left_down = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
        let progress = layout.progress;
        let volume = layout.volume;
        let progress_ratio = |x: u16| {
            (x.saturating_sub(progress.x) as f64 / progress.width.saturating_sub(1).max(1) as f64)
                .clamp(0.0, 1.0)
        };
        let volume_ratio = |x: u16| {
            let gauge = volume.x + 4;
            (x.saturating_sub(gauge) as f64 / (VOLUME_CELLS - 1) as f64).clamp(0.0, 1.0)
        };

        // The wheel seeks over the progress line and changes the volume elsewhere.
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = matches!(mouse.kind, MouseEventKind::ScrollUp);
                let action = match (y == progress.y, up) {
                    (true, true) => SoundAction::Forward,
                    (true, false) => SoundAction::Backward,
                    (false, true) => SoundAction::Plus,
                    (false, false) => SoundAction::Minus,
                };
                action.apply_sound_action(&mut self.music_player);
                return false;
            }
            // A drag moves only the control it started on, wherever the pointer goes.
            MouseEventKind::Drag(MouseButton::Left) => {
                match self.bar_drag {
                    Some(BarControl::Progress) => self.seek_preview = Some(progress_ratio(x)),
                    Some(BarControl::Volume) => self
                        .music_player
                        .sink
                        .set_volume((volume_ratio(x) * 100.0).round() as i32),
                    None => {}
                }
                return false;
            }
            MouseEventKind::Up(_) => {
                if let Some(ratio) = self.seek_preview.take() {
                    if let Some(total) = self.music_player.total_seconds() {
                        self.music_player
                            .sink
                            .seek_to(Duration::from_secs_f64(total as f64 * ratio));
                    }
                }
                self.bar_drag = None;
                return false;
            }
            _ => {}
        }
        if !left_down {
            return false;
        }
        if inside(&progress) {
            // Seek when the button is released, so dragging does not stutter.
            self.bar_drag = Some(BarControl::Progress);
            self.seek_preview = Some(progress_ratio(x));
            return false;
        }
        if inside(&volume) {
            self.bar_drag = Some(BarControl::Volume);
            self.music_player
                .sink
                .set_volume((volume_ratio(x) * 100.0).round() as i32);
            return false;
        }
        let player = &mut self.music_player;
        if let Some((_, button)) = layout.buttons.iter().find(|(rect, _)| inside(rect)) {
            match button {
                PlayerButton::Previous => SoundAction::Previous(1).apply_sound_action(player),
                PlayerButton::PlayPause => SoundAction::PlayPause.apply_sound_action(player),
                PlayerButton::Next => SoundAction::Next(1).apply_sound_action(player),
                PlayerButton::Shuffle => {
                    SoundAction::ShuffleRest.apply_sound_action(player);
                    self.toast("Queue shuffled");
                }
                PlayerButton::Repeat => {
                    SoundAction::CycleRepeat.apply_sound_action(player);
                    let label = format!("Repeat: {}", self.music_player.repeat.label());
                    self.toast(&label);
                }
            }
            return false;
        }
        if inside(&layout.hints) {
            let mut start = layout.hints.x + 1;
            for hint in self.hints() {
                let end = start + hint.width();
                if x >= start && x < end {
                    let key = if x < start + hint.width() / 2 {
                        hint.left
                    } else {
                        hint.right
                    };
                    return self.handle_key(key);
                }
                start = end + 3;
            }
        }
        false
    }

    /// A number that changes when anything worth saving changes.
    fn state_signature(&self) -> u64 {
        let player = &self.music_player;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        player.list.len().hash(&mut hasher);
        player.current.hash(&mut hasher);
        player.list.first().map(|v| &v.video_id).hash(&mut hasher);
        player.list.last().map(|v| &v.video_id).hash(&mut hasher);
        player.sink.volume().hash(&mut hasher);
        self.panel_open.hash(&mut hasher);
        (self.panel.tab == Panel::Lyrics).hash(&mut hasher);
        player.repeat.label().hash(&mut hasher);
        (self.current_screen as u8).hash(&mut hasher);
        self.playlist_viewer.name.hash(&mut hasher);
        self.playlist_viewer.videos.len().hash(&mut hasher);
        hasher.finish()
    }

    fn save_state(&mut self) {
        let signature = self.state_signature();
        if signature == self.saved_signature {
            return;
        }
        self.saved_signature = signature;
        let player = &self.music_player;
        state::save(&state::SavedState {
            queue: player.list.clone(),
            current: player.current,
            volume: Some(player.sink.volume()),
            panel_open: self.panel_open,
            lyrics_tab: self.panel.tab == Panel::Lyrics,
            repeat: player.repeat,
            screen: self.current_screen as u8,
            position: match player.resume_at {
                Some(at) if player.start_paused => at.as_secs(),
                _ => player.sink.elapsed().as_secs(),
            },
            viewed: (!self.playlist_viewer.videos.is_empty()).then(|| state::Viewed {
                name: self.playlist_viewer.name.clone(),
                browse_id: self.playlist_viewer.browse_id.clone(),
                videos: self.playlist_viewer.videos.clone(),
            }),
        });
    }

    /// Loads the cover and the lyrics of the playing track when it changes.
    fn follow_current_track(&mut self) {
        widgets::set_now_playing(
            self.music_player
                .current()
                .map(|v| v.video_id.as_str())
                .unwrap_or(""),
        );
        let Some(id) = self.music_player.current().map(|v| v.video_id.clone()) else {
            self.cover = None;
            return;
        };
        let stale = match &self.cover {
            Some((cover_id, pixels, checked)) => {
                cover_id != &id || (pixels.is_none() && checked.elapsed() > Duration::from_secs(1))
            }
            None => true,
        };
        if stale {
            let pixels = crate::covers::load(&id);
            if pixels.is_none() {
                crate::covers::fetch(&id);
            }
            self.cover = Some((id.clone(), pixels, Instant::now()));
        }
        if self.panel_open && self.panel.tab == Panel::Lyrics && self.panel.want_lyrics(&id) {
            let updater = self.music_player.updater.clone();
            crate::run_service(async move {
                let lyrics = match crate::api_instance().await {
                    Some(api) => api.get_lyrics(&id).await.ok().flatten(),
                    None => None,
                };
                let _ = updater.send(ManagerMessage::Lyrics(id, lyrics));
            });
        }
    }

    /// Draws the global shell (top bar, sidebar, panel, player) plus the screen.
    pub fn draw(&mut self, f: &mut Frame) {
        let size = f.size();
        if size.width < MIN_WIDTH || size.height < MIN_HEIGHT {
            // Too small for the layout: say so instead of drawing out of bounds.
            self.shell = Shell::default();
            self.menu = None;
            self.help_open = false;
            f.render_widget(Block::default().style(Style::default().bg(THEME.bg)), size);
            let lines = ["Aria", "Make the", "window bigger"];
            for (i, line) in lines.iter().enumerate() {
                if (i as u16) < size.height {
                    f.buffer_mut().set_stringn(
                        size.x,
                        size.y + i as u16,
                        line,
                        size.width as usize,
                        Style::default().fg(THEME.text_dim).bg(THEME.bg),
                    );
                }
            }
            return;
        }
        self.shell = Shell::new(size, self.panel_open);
        let shell = self.shell;
        self.follow_current_track();
        if let Some((_, at)) = &self.toast {
            if at.elapsed() > TOAST_TIME {
                self.toast = None;
            }
        }

        f.render_widget(Block::default().style(Style::default().bg(THEME.bg)), f.size());
        if !shell.narrow() {
            draw_box(f, grow(shell.side), " Aria ");
        }
        draw_box(
            f,
            grow(shell.content),
            &format!(" {} ", section_name(self.current_screen)),
        );
        draw_box(f, shell.bar, "");
        self.draw_top_bar(f);
        if !shell.narrow() {
            self.draw_sidebar(f);
        }
        self.current_screen().render(f, shell.content);
        if shell.panel.width > 0 {
            draw_box(f, grow(shell.panel), "");
            let cover = self.cover.as_ref().and_then(|(_, pixels, _)| pixels.as_deref());
            self.panel.render(
                f.buffer_mut(),
                shell.panel,
                &self.music_player,
                cover.filter(|_| *TRUECOLOR),
            );
        }
        self.draw_player_bar(f);

        let screen = self.shell_screen();
        if let Some(menu) = &self.menu {
            menu.render(f, screen);
        }
        if self.help_open {
            Self::draw_help(f, screen);
        }
    }

    fn draw_top_bar(&self, f: &mut Frame) {
        let top = self.shell.top;
        let buf = f.buffer_mut();
        let base = Style::default().bg(THEME.bg);
        buf.set_string(
            top.x,
            top.y,
            " ARIA ",
            base.fg(THEME.accent).add_modifier(Modifier::BOLD),
        );
        buf.set_string(top.x + 6, top.y, "│", base.fg(THEME.border));
        if self.shell.narrow() {
            for (rect, screen) in top_tabs(top) {
                let style = if screen == self.current_screen {
                    base.fg(THEME.text).bg(THEME.accent_bg).add_modifier(Modifier::BOLD)
                } else if hovered(rect, rect.y) {
                    base.fg(THEME.text).bg(THEME.hover)
                } else {
                    base.fg(THEME.text_dim)
                };
                buf.set_stringn(
                    rect.x,
                    rect.y,
                    format!(" {} ", section_name(screen)),
                    rect.width as usize,
                    style,
                );
            }
        } else {
            buf.set_string(
                top.x + 7,
                top.y,
                format!(" {} ", section_name(self.current_screen)),
                base.fg(THEME.text_dim),
            );
        }

        if let Some((text, _)) = &self.toast {
            let text = format!(" ✓ {text} ");
            let width = text.chars().count() as u16;
            let x = top.x + top.width.saturating_sub(QUIT_GROUP_WIDTH + PANEL_BUTTON_WIDTH + width + 2);
            buf.set_string(x, top.y, text, base.fg(THEME.text).bg(THEME.accent_bg));
        }

        let button = panel_button(top);
        let style = if self.panel_open {
            base.fg(THEME.text).bg(THEME.accent).add_modifier(Modifier::BOLD)
        } else if hovered(button, button.y) {
            base.fg(THEME.text).bg(THEME.hover)
        } else {
            base.fg(THEME.text).bg(THEME.accent_bg)
        };
        buf.set_stringn(button.x, button.y, " Panel ", button.width as usize, style);

        let quit_x = top.x + top.width.saturating_sub(QUIT_GROUP_WIDTH);
        buf.set_string(quit_x, top.y, " Ctrl+C ", base.fg(THEME.border));
        let quit_rect = Rect {
            x: quit_x + 7,
            y: top.y,
            width: 6,
            height: 1,
        };
        let quit_style = if hovered(quit_rect, top.y) {
            base.fg(THEME.text).bg(THEME.accent)
        } else {
            base.fg(THEME.text).bg(THEME.accent_bg)
        };
        buf.set_string(quit_x + 7, top.y, " Quit ", quit_style.add_modifier(Modifier::BOLD));
    }

    fn draw_sidebar(&self, f: &mut Frame) {
        let area = self.shell.side;
        let buf = f.buffer_mut();
        let width = area.width.saturating_sub(1) as usize;
        for (index, (target, label)) in SIDEBAR_SECTIONS.iter().enumerate() {
            let y = area.y + 1 + index as u16;
            if y >= area.y + area.height {
                break;
            }
            let active = *target == self.current_screen;
            let style = if active {
                Style::default()
                    .fg(THEME.text)
                    .bg(THEME.accent_bg)
                    .add_modifier(Modifier::BOLD)
            } else if hovered(area, y) {
                Style::default().fg(THEME.text).bg(THEME.hover)
            } else {
                Style::default().fg(THEME.text_dim).bg(THEME.bg)
            };
            buf.set_string(
                area.x,
                y,
                if active { "▌" } else { " " },
                Style::default().fg(THEME.accent).bg(THEME.bg),
            );
            buf.set_string(area.x + 1, y, fit(label, width), style);
        }

        // Account row at the bottom: the sign-in button or the session state.
        if area.height < 7 {
            return;
        }
        let y = area.y + area.height - 1;
        let expired = crate::SESSION_EXPIRED.load(std::sync::atomic::Ordering::Relaxed);
        let (text, style) = if self.signed_in && !expired {
            (" ● Signed in", Style::default().fg(Color::Green).bg(THEME.bg))
        } else if crate::LOGIN_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
            (" Waiting for browser…", Style::default().fg(THEME.text_dim).bg(THEME.bg))
        } else {
            (
                " F8 Sign in",
                Style::default()
                    .fg(THEME.text)
                    .bg(if hovered(area, y) {
                        THEME.accent
                    } else {
                        THEME.accent_bg
                    })
                    .add_modifier(Modifier::BOLD),
            )
        };
        buf.set_string(area.x, y, fit(text, area.width as usize), style);
    }

    fn draw_player_bar(&self, f: &mut Frame) {
        let layout = player_layout(self.shell.bar);
        let now = self.music_player.now_playing();
        let base = Style::default().bg(THEME.bg);
        let buf = f.buffer_mut();

        // Cover: 8 x 4 cells, each showing two pixels with a half block.
        if let Some(rect) = layout.cover {
            match self.cover.as_ref().and_then(|(_, p, _)| p.as_deref()) {
                Some(pixels) => crate::covers::draw(buf, rect, pixels),
                None => {
                    for row in 0..rect.height {
                        for col in 0..rect.width {
                            buf.get_mut(rect.x + col, rect.y + row)
                                .set_symbol(if row == 1 && col == 3 { "♪" } else { "░" })
                                .set_fg(THEME.border)
                                .set_bg(THEME.bg);
                        }
                    }
                }
            }
        }

        // Title, artist and time.
        let title = layout.title;
        let (name, artist) = match &now {
            Some(n) => (n.title.as_str(), n.artist.as_str()),
            None => ("Nothing playing", ""),
        };
        let time = now
            .as_ref()
            .map(|n| {
                // While the knob is dragged, show where the release will land.
                let elapsed = self
                    .seek_preview
                    .map(|ratio| (ratio * n.total as f64) as u64)
                    .unwrap_or(n.elapsed);
                format!(
                    "{}:{:02} / {}:{:02}",
                    elapsed / 60,
                    elapsed % 60,
                    n.total / 60,
                    n.total % 60
                )
            })
            .unwrap_or_default();
        let time_width = time.chars().count() as u16;
        let text_width = title.width.saturating_sub(time_width + 2) as usize;
        let name_width = if artist.is_empty() {
            text_width
        } else {
            (text_width * 3 / 5).max(text_width.saturating_sub(unicode_width::UnicodeWidthStr::width(artist) + 3))
        };
        let shown = fit(name, name_width);
        buf.set_string(
            title.x,
            title.y,
            shown.trim_end(),
            base.fg(THEME.text).add_modifier(Modifier::BOLD),
        );
        if !artist.is_empty() {
            let used = unicode_width::UnicodeWidthStr::width(shown.trim_end()) as u16;
            buf.set_string(
                title.x + used,
                title.y,
                format!(
                    "  ·  {}",
                    fit(artist, text_width.saturating_sub(used as usize + 5)).trim_end()
                ),
                base.fg(THEME.text_dim),
            );
        }
        buf.set_string(
            title.x + title.width.saturating_sub(time_width),
            title.y,
            &time,
            base.fg(THEME.text_dim),
        );

        // Buttons.
        let paused = now.as_ref().map(|n| n.paused).unwrap_or(true);
        let repeat = self.music_player.repeat;
        for (rect, button) in &layout.buttons {
            let label = match button {
                PlayerButton::Previous => " ◀◀ ".to_owned(),
                PlayerButton::PlayPause => (if paused { "  ▶  " } else { " ▮▮  " }).to_owned(),
                PlayerButton::Next => " ▶▶ ".to_owned(),
                PlayerButton::Shuffle => " ⇄ Shuffle ".to_owned(),
                PlayerButton::Repeat => format!(" ↻ Repeat {:<4}", repeat.label()),
            };
            let active = *button == PlayerButton::Repeat && repeat != crate::systems::player::Repeat::Off;
            let style = if *button == PlayerButton::PlayPause {
                base.fg(THEME.text).bg(THEME.accent_bg).add_modifier(Modifier::BOLD)
            } else if hovered(*rect, rect.y) {
                base.fg(THEME.text).bg(THEME.hover)
            } else if active {
                base.fg(THEME.accent).add_modifier(Modifier::BOLD)
            } else {
                base.fg(THEME.text_dim)
            };
            buf.set_stringn(rect.x, rect.y, label, rect.width as usize, style);
        }

        // Volume.
        let volume = self.music_player.sink.volume().clamp(0, 100) as usize;
        let filled = (volume * 7 + 50) / 100;
        let volume_rect = layout.volume;
        buf.set_string(
            volume_rect.x,
            volume_rect.y,
            format!(
                "vol {}{} {:>3}",
                "█".repeat(filled),
                "░".repeat(7 - filled),
                volume
            ),
            base.fg(if hovered(volume_rect, volume_rect.y) {
                THEME.text
            } else {
                THEME.text_dim
            }),
        );

        // Progress line with a knob.
        let progress = layout.progress;
        let ratio = self
            .seek_preview
            .or(now.as_ref().map(|n| n.ratio))
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let width = progress.width as usize;
        if width > 1 {
            let filled = ((ratio * (width - 1) as f64).round() as usize).min(width - 1);
            buf.set_string(progress.x, progress.y, "━".repeat(filled), base.fg(THEME.accent));
            buf.set_string(
                progress.x + filled as u16,
                progress.y,
                "●",
                base.fg(if hovered(progress, progress.y) {
                    THEME.accent
                } else {
                    THEME.text
                }),
            );
            buf.set_string(
                progress.x + filled as u16 + 1,
                progress.y,
                "─".repeat(width - filled - 1),
                base.fg(THEME.border),
            );
        }

        // Keyboard hints for the current screen; each one is clickable.
        let hints = layout.hints;
        let mut x = hints.x + 1;
        for hint in self.hints() {
            let width = hint.width();
            if x + width > hints.x + hints.width {
                break;
            }
            let rect = Rect {
                x,
                y: hints.y,
                width,
                height: 1,
            };
            let over = hovered(rect, hints.y);
            let key_style = base.fg(THEME.accent).add_modifier(Modifier::BOLD);
            let desc_style = if over {
                base.fg(THEME.text).bg(THEME.hover)
            } else {
                base.fg(THEME.text_dim)
            };
            buf.set_string(x, hints.y, hint.key, if over { key_style.bg(THEME.hover) } else { key_style });
            buf.set_string(
                x + hint.key.chars().count() as u16,
                hints.y,
                format!(" {}", hint.desc),
                desc_style,
            );
            x += width + 3;
        }
    }

    fn draw_help(f: &mut Frame, screen: Rect) {
        let width = 96.min(screen.width.saturating_sub(4));
        let height = (HELP.len() as u16 + 4).min(screen.height);
        let area = Rect {
            x: screen.x + (screen.width - width) / 2,
            y: screen.y + (screen.height - height) / 2,
            width,
            height,
        };
        f.render_widget(Clear, area);
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(THEME.accent))
                .title(" Keys and mouse ")
                .style(Style::default().bg(THEME.bg).fg(THEME.text)),
            area,
        );
        let buf = f.buffer_mut();
        let inner = width.saturating_sub(4) as usize;
        for (line, (topic, keys)) in HELP.iter().enumerate() {
            let y = area.y + 1 + line as u16;
            buf.set_string(
                area.x + 2,
                y,
                fit_right(topic, 9),
                Style::default().fg(THEME.accent).bg(THEME.bg).add_modifier(Modifier::BOLD),
            );
            buf.set_string(
                area.x + 13,
                y,
                fit(keys, inner.saturating_sub(11)),
                Style::default().fg(THEME.text).bg(THEME.bg),
            );
        }
        buf.set_string(
            area.x + 2,
            area.y + height.saturating_sub(2),
            "Press any key or click to close",
            Style::default().fg(THEME.text_dim).bg(THEME.bg),
        );
    }

    /// The main loop of the manager
    pub fn run(&mut self, updater: &Receiver<ManagerMessage>) -> Result<(), io::Error> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            crossterm::terminal::SetTitle("Aria")
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let playing_tick = Duration::from_millis(250);
        let mut last_draw = Instant::now();
        let mut last_chores = Instant::now();
        // Redraw only on input, messages or a tick (the progress bar), not every loop turn.
        let mut dirty = true;
        let mut was_syncing = false;
        'a: loop {
            if is_shutdown_sent() {
                break;
            }
            // Loading the library peaks memory: hand it back as soon as it is done.
            let syncing =
                crate::tasks::api::SYNCING.load(std::sync::atomic::Ordering::Relaxed) > 0;
            if was_syncing && !syncing {
                crate::trim_memory();
            }
            was_syncing = syncing;
            if last_chores.elapsed() >= Duration::from_secs(20) {
                last_chores = Instant::now();
                self.save_state();
                crate::trim_memory();
            }
            // Paused or empty: nothing moves on screen, so wake up less often.
            let tick = if self.music_player.is_idle() && self.toast.is_none() {
                Duration::from_millis(1000)
            } else {
                playing_tick
            };
            while let Ok(e) = updater.try_recv() {
                dirty = true;
                if self.handle_manager_message(e) {
                    break 'a;
                }
            }
            if dirty || last_draw.elapsed() >= tick {
                dirty = false;
                last_draw = Instant::now();
                terminal.draw(|f| {
                    self.music_player.update();
                    self.draw(f);
                })?;
                let title = match self.music_player.now_playing() {
                    Some(n) if !n.artist.is_empty() => format!("▶ {} · {}", n.title, n.artist),
                    Some(n) => format!("▶ {}", n.title),
                    None => "Aria".to_owned(),
                };
                if title != self.window_title {
                    execute!(terminal.backend_mut(), crossterm::terminal::SetTitle(&title))?;
                    self.window_title = title;
                }
            }

            let timeout = tick.saturating_sub(last_draw.elapsed());
            if crossterm::event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        dirty = true;
                        if self.handle_key(key) {
                            break;
                        }
                    }
                    Event::Mouse(mouse) => {
                        let position = Some((mouse.column, mouse.row));
                        if matches!(mouse.kind, MouseEventKind::Moved) {
                            // Hover only matters when the pointer changes cell.
                            if position != self.mouse_pos {
                                dirty = true;
                            }
                        } else {
                            dirty = true;
                        }
                        self.mouse_pos = position;
                        widgets::set_hover(position);
                        if self.handle_mouse(mouse) {
                            break;
                        }
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => (),
                }
            }
        }

        self.saved_signature = 0;
        self.save_state();

        // restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            crossterm::terminal::SetTitle("")
        )?;
        terminal.show_cursor()?;

        shutdown();

        Ok(())
    }
}

// UTILS SECTION TO SPLIT THE TERMINAL INTO DIFFERENT PARTS

pub fn split_y_start(f: Rect, start_size: u16) -> [Rect; 2] {
    let mut rectlistvol = f;
    rectlistvol.height = start_size.min(f.height);
    let mut rectprogress = f;
    rectprogress.y += rectlistvol.height;
    rectprogress.height = rectprogress.height.saturating_sub(start_size);
    [rectlistvol, rectprogress]
}

pub fn split_y(f: Rect, end_size: u16) -> [Rect; 2] {
    let mut rectlistvol = f;
    rectlistvol.height = rectlistvol.height.saturating_sub(end_size);
    let mut rectprogress = f;
    rectprogress.y += rectprogress.height.saturating_sub(end_size);
    rectprogress.height = end_size.min(f.height);
    [rectlistvol, rectprogress]
}

pub fn split_x_start(f: Rect, start_size: u16) -> [Rect; 2] {
    let mut left = f;
    left.width = start_size.min(f.width);
    let mut right = f;
    right.x += left.width;
    right.width = right.width.saturating_sub(start_size);
    [left, right]
}

/// True when (`x`, `y`) is inside `rect` shrunk by `margin` on every side.
pub fn rect_contains(rect: &Rect, x: u16, y: u16, margin: u16) -> bool {
    rect.x + margin <= x
        && x < (rect.x + rect.width).saturating_sub(margin)
        && rect.y + margin <= y
        && y < (rect.y + rect.height).saturating_sub(margin)
}
