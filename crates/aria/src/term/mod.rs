pub mod device_lost;
pub mod item_list;
pub mod list_selector;
pub mod music_player;
pub mod playlist;
pub mod playlist_view;
pub mod queue_panel;
pub mod search;

use std::{
    io::{self},
    time::{Duration, Instant},
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use flume::{Receiver, Sender};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame, Terminal,
};
use ytpapi2::YoutubeMusicVideoRef;

use crate::{
    is_shutdown_sent, shutdown, structures::sound_action::SoundAction, systems::player::PlayerState,
};

use self::{device_lost::DeviceLost, item_list::ListItem, playlist::Chooser, search::Search};

use crate::term::playlist_view::PlaylistView;

// A trait to handle the different screens
pub trait Screen {
    fn on_mouse_press(&mut self, mouse_event: MouseEvent, frame_data: &Rect) -> EventResponse;
    fn on_key_press(&mut self, mouse_event: KeyEvent, frame_data: &Rect) -> EventResponse;
    fn render(&mut self, frame: &mut Frame, area: Rect);
    fn handle_global_message(&mut self, message: ManagerMessage) -> EventResponse;
    fn close(&mut self, new_screen: Screens) -> EventResponse;
    fn open(&mut self) -> EventResponse;
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
    Inspect(String, Screens, Vec<YoutubeMusicVideoRef>),
    ChangeState(Screens),
    SearchFrom(Screens),
    Autoplay(String),
    Status(String),
    #[allow(dead_code)]
    PlaylistFrom(Screens),
    RestartPlayer,
    Quit,
    AddElementToChooser((String, Vec<YoutubeMusicVideoRef>)),
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

// --- YouTube Music inspired palette (16 colors for wide terminal support) ---
pub const ACCENT: Color = Color::LightRed;
pub const ACCENT_BG: Color = Color::Indexed(52);
pub const BG: Color = Color::Black;
pub const BORDER: Color = Color::DarkGray;
pub const TEXT: Color = Color::White;
pub const TEXT_DIM: Color = Color::Gray;

const SIDEBAR_WIDTH: u16 = 24;
const TOP_BAR_HEIGHT: u16 = 1;
const PLAYER_BAR_HEIGHT: u16 = 5;

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

/// Keyboard hints shown in the player bar; they are also clickable.
const PLAYER_HINTS: [(&str, &str); 4] = [
    ("Space", " pause"),
    ("←/→", " 10s"),
    ("Ctrl+←/→", " track"),
    ("+/-", " volume"),
];

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

const QUEUE_WIDTH: u16 = 40;
const QUEUE_BUTTON_WIDTH: u16 = 8;
/// Width of the "Ctrl+C  Quit" group at the right end of the top bar.
const QUIT_GROUP_WIDTH: u16 = 13;

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

/// The box around the player bar, whose first row doubles as the top border.
fn player_box(bar: Rect) -> Rect {
    Rect {
        x: bar.x.saturating_sub(1),
        y: bar.y,
        width: bar.width + 2,
        height: bar.height + 1,
    }
}

/// Draws a rounded box on `outer` with `title` on its top border.
fn draw_box(f: &mut Frame, outer: Rect, title: &str) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(BG));
    f.render_widget(block, outer);
}

/// Splits the terminal into `(top bar, sidebar, content, player bar, queue)`.
/// Every area but the top bar is the inside of a box; the queue is empty while
/// its panel is closed.
fn shell_layout(size: Rect, queue_open: bool) -> (Rect, Rect, Rect, Rect, Rect) {
    let [top, rest] = split_y_start(size, TOP_BAR_HEIGHT);
    let [rest, bottom] = split_y(rest, PLAYER_BAR_HEIGHT);
    let [side, mut content] = split_x_start(rest, SIDEBAR_WIDTH);
    let mut queue = Rect::new(u16::MAX, u16::MAX, 0, 0);
    if queue_open && content.width > QUEUE_WIDTH + 20 {
        queue = shrink(Rect {
            x: content.x + content.width - QUEUE_WIDTH,
            width: QUEUE_WIDTH,
            ..content
        });
        content.width -= QUEUE_WIDTH;
    }
    let bar = Rect {
        x: bottom.x + 1,
        width: bottom.width.saturating_sub(2),
        height: bottom.height.saturating_sub(1),
        ..bottom
    };
    (top, shrink(side), shrink(content), bar, queue)
}

/// The "Queue" toggle button, just left of "Quit" in the top bar.
fn queue_button(top: Rect) -> Rect {
    Rect {
        x: top.x + top.width.saturating_sub(QUIT_GROUP_WIDTH + QUEUE_BUTTON_WIDTH),
        width: QUEUE_BUTTON_WIDTH,
        ..top
    }
}

/// One-line representation of a track: `title · artist · duration`.
pub fn format_track(video: &ytpapi2::YoutubeMusicVideoRef) -> String {
    let mut parts = vec![crate::utils::to_bidi_string(&video.title)];
    if !video.author.is_empty() {
        parts.push(crate::utils::to_bidi_string(&video.author));
    }
    if !video.duration.is_empty() {
        parts.push(video.duration.clone());
    }
    format!(" {}", parts.join("  ·  "))
}

/// Spans of the player bar's keyboard hint line.
fn hint_spans() -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, desc)) in PLAYER_HINTS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(*key, Style::default().fg(ACCENT)));
        spans.push(Span::styled(*desc, Style::default().fg(TEXT_DIM)));
    }
    spans
}

/// Draws a scrollbar on the right edge of `area` when `total` rows do not fit.
/// `first` is the index of the top visible row.
pub fn draw_scrollbar(buf: &mut ratatui::buffer::Buffer, area: Rect, total: usize, first: usize) {
    let height = area.height as usize;
    if height == 0 || area.width == 0 || total <= height {
        return;
    }
    let thumb = (height * height / total).max(1);
    let last_first = total - height;
    let top = first.min(last_first) * (height - thumb) / last_first;
    let x = area.x + area.width - 1;
    for row in 0..height {
        let (symbol, color) = if row >= top && row < top + thumb {
            ("█", TEXT_DIM)
        } else {
            ("│", BORDER)
        };
        buf.set_string(
            x,
            area.y + row as u16,
            symbol,
            Style::default().fg(color).bg(BG),
        );
    }
}

/// Which sidebar entry is under `y`, if any.
fn sidebar_target(y: u16, area: Rect) -> Option<Screens> {
    // Row 0 is a blank line.
    let row = y.saturating_sub(area.y) as usize;
    row.checked_sub(1)
        .and_then(|index| SIDEBAR_SECTIONS.get(index))
        .map(|(screen, _)| *screen)
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
    queue_open: bool,
    signed_in: bool,
}

impl Manager {
    pub async fn new(action_sender: Sender<SoundAction>, music_player: PlayerState) -> Self {
        let updater = music_player.updater.clone();
        Self {
            music_player,
            chooser: Chooser::new(Screens::MusicPlayer),
            playlist_viewer: PlaylistView {
                sender: action_sender.clone(),
                items: ListItem::new(" Playlist ".to_owned()),
                goto: Screens::Playlist,
                videos: Vec::new(),
            },
            search: Search::new(action_sender, updater).await,
            current_screen: Screens::Playlist,
            device_lost: DeviceLost(Vec::new(), None),
            mouse_pos: None,
            queue_open: false,
            signed_in: !crate::is_guest(),
        }
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

    /// Draws the global shell (top bar, sidebar, player bar) plus the screen.
    pub fn draw(&mut self, f: &mut Frame) {
        let (top, side, content, bottom, queue) = shell_layout(f.size(), self.queue_open);
        let now_playing = self.music_player.now_playing();

        draw_box(f, grow(side), " Aria ");
        draw_box(
            f,
            grow(content),
            &format!(" {} ", section_name(self.current_screen)),
        );
        draw_box(f, player_box(bottom), "");
        Self::draw_top_bar(f, top, self.current_screen, self.queue_open);
        Self::draw_sidebar(f, side, self.current_screen, self.mouse_pos);
        Self::draw_account(f, side, self.signed_in, self.mouse_pos);
        self.current_screen().render(f, content);
        if self.queue_open && queue.width > 0 {
            draw_box(f, grow(queue), " Up next ");
            queue_panel::render(f, queue, &self.music_player);
        }
        Self::draw_player_bar(
            f,
            bottom,
            now_playing,
            self.music_player.sink.volume() as f32,
            self.mouse_pos,
        );
    }

    fn draw_top_bar(f: &mut Frame, area: Rect, screen: Screens, queue_open: bool) {
        let line = Line::from(vec![
            Span::styled(
                " ARIA ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("│", Style::default().fg(BORDER)),
            Span::styled(
                format!(" {} ", section_name(screen)),
                Style::default().fg(TEXT_DIM),
            ),
        ]);
        f.render_widget(Paragraph::new(line).style(Style::default().bg(BG)), area);

        let hint = "Ctrl+C ";
        let button = " Quit ";
        let width = ((hint.chars().count() + button.chars().count()) as u16).min(area.width);
        let rect = Rect {
            x: area.x + area.width.saturating_sub(width),
            y: area.y,
            width,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(hint, Style::default().fg(BORDER)),
                Span::styled(
                    button,
                    Style::default()
                        .fg(TEXT)
                        .bg(ACCENT_BG)
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Right)
            .style(Style::default().bg(BG)),
            rect,
        );

        let style = if queue_open {
            Style::default().fg(TEXT).bg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT).bg(ACCENT_BG)
        };
        f.render_widget(
            Paragraph::new(Span::styled(" Queue ", style)).style(Style::default().bg(BG)),
            queue_button(area),
        );
    }

    /// Account row at the bottom of the sidebar: the sign-in button or the session state.
    fn draw_account(f: &mut Frame, side: Rect, signed_in: bool, hover: Option<(u16, u16)>) {
        if side.height < 7 {
            return;
        }
        let row = Rect {
            y: side.y + side.height - 1,
            height: 1,
            ..side
        };
        let hovered = hover
            .map(|(x, y)| y == row.y && x >= row.x && x < row.x + row.width)
            .unwrap_or(false);
        let (text, style) = if signed_in {
            (" ● Signed in", Style::default().fg(Color::Green))
        } else if crate::LOGIN_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
            (" Waiting for browser…", Style::default().fg(TEXT_DIM))
        } else {
            (
                " F8 Sign in",
                Style::default()
                    .fg(TEXT)
                    .bg(if hovered { ACCENT } else { ACCENT_BG })
                    .add_modifier(Modifier::BOLD),
            )
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{:<width$}", text, width = row.width as usize),
                style,
            )),
            row,
        );
    }

    /// Starts the in-app sign-in and shows its progress on the Home screen.
    fn sign_in(&mut self) {
        if self.signed_in {
            return;
        }
        crate::start_login(self.music_player.updater.clone());
        self.handle_event(ManagerMessage::ChangeState(Screens::Playlist).event());
    }

    fn draw_sidebar(f: &mut Frame, area: Rect, screen: Screens, hover: Option<(u16, u16)>) {
        let inner_width = area.width.saturating_sub(2) as usize;
        let mut lines = vec![Line::from("")];
        for (index, (target, label)) in SIDEBAR_SECTIONS.iter().enumerate() {
            let target = *target;
            let active = target == screen;
            let hovered = hover
                .map(|(_, y)| y == area.y.saturating_add(1 + index as u16))
                .unwrap_or(false);
            let text_style = if active {
                Style::default()
                    .fg(TEXT)
                    .bg(ACCENT_BG)
                    .add_modifier(Modifier::BOLD)
            } else if hovered {
                Style::default().fg(TEXT).bg(Color::Indexed(236))
            } else {
                Style::default().fg(TEXT_DIM)
            };
            lines.push(Line::from(vec![
                Span::styled(if active { "▌" } else { " " }, Style::default().fg(ACCENT)),
                Span::styled(
                    format!("{:<width$}", label, width = inner_width),
                    text_style,
                ),
            ]));
        }
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(BG)), area);
    }

    fn draw_player_bar(
        f: &mut Frame,
        area: Rect,
        now: Option<NowPlaying>,
        volume: f32,
        hover: Option<(u16, u16)>,
    ) {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

        let (title, artist) = match &now {
            Some(n) => (n.title.clone(), n.artist.clone()),
            None => ("Nothing playing".to_string(), String::new()),
        };
        let play_icon = if now.as_ref().map(|n| n.paused).unwrap_or(true) {
            "▶ "
        } else {
            "▮▮"
        };

        let transport_hover = hover
            .map(|(x, y)| y == rows[1].y && x < area.x + 12)
            .unwrap_or(false);
        let arrow_style = if transport_hover {
            Style::default().fg(TEXT)
        } else {
            Style::default().fg(TEXT_DIM)
        };
        let mut title_spans = vec![
            Span::styled(" ◀◀ ", arrow_style),
            Span::styled(
                format!(" {play_icon} "),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ▶▶ ", arrow_style),
            Span::styled("│ ", Style::default().fg(BORDER)),
            Span::styled(
                title,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ];
        if !artist.is_empty() {
            title_spans.push(Span::styled("  ·  ", Style::default().fg(BORDER)));
            title_spans.push(Span::styled(artist, Style::default().fg(TEXT_DIM)));
        }

        f.render_widget(
            Paragraph::new(Line::from(title_spans)).style(Style::default().bg(BG)),
            rows[1],
        );

        if let Some(n) = &now {
            let time = format!(
                "{}:{:02} / {}:{:02} ",
                n.elapsed / 60,
                n.elapsed % 60,
                n.total / 60,
                n.total % 60
            );
            let width = 16.min(rows[1].width);
            let rect = Rect {
                x: rows[1].x + rows[1].width.saturating_sub(width),
                y: rows[1].y,
                width,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(Span::styled(time, Style::default().fg(TEXT_DIM)))
                    .alignment(Alignment::Right)
                    .style(Style::default().bg(BG)),
                rect,
            );
        }

        let ratio = now.as_ref().map(|n| n.ratio).unwrap_or(0.0).clamp(0.0, 1.0);
        let track_width = rows[2].width as usize;
        let filled =
            ((ratio * track_width as f64).round() as usize).min(track_width.saturating_sub(1));
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("━".repeat(filled), Style::default().fg(ACCENT)),
                Span::styled("●", Style::default().fg(TEXT)),
                Span::styled(
                    "─".repeat(track_width.saturating_sub(filled + 1)),
                    Style::default().fg(BORDER),
                ),
            ]))
            .style(Style::default().bg(BG)),
            rows[2],
        );

        f.render_widget(
            Paragraph::new(Line::from(hint_spans()))
            .style(Style::default().bg(BG)),
            rows[3],
        );

        let volume = volume.clamp(0.0, 100.0);
        let filled = ((volume / 100.0) * 8.0).round().clamp(0.0, 8.0) as usize;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(8 - filled));
        let width = 14.min(rows[3].width);
        let rect = Rect {
            x: rows[3].x + rows[3].width.saturating_sub(width),
            y: rows[3].y,
            width,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("vol {bar} "),
                Style::default().fg(TEXT_DIM),
            ))
            .alignment(Alignment::Right)
            .style(Style::default().bg(BG)),
            rect,
        );
    }

    /// Runs the action of the keyboard hint under column `x` (a click on the hint line).
    fn player_hint_click(&mut self, x: u16) {
        // Hint i spans from `start` to `start + len`; spacing is 3 columns.
        let mut start: u16 = 1;
        for (i, (key, desc)) in PLAYER_HINTS.iter().enumerate() {
            let len = (key.chars().count() + desc.chars().count()) as u16;
            if i > 0 {
                start += 3;
            }
            let end = start + len;
            if x >= start && x < end {
                let mid = start + len / 2;
                let back = x < mid;
                let sender = self.music_player.soundaction_sender.clone();
                let action = match (i, back) {
                    (0, _) => {
                        self.music_player.sink.toggle_playback();
                        return;
                    }
                    (1, true) => SoundAction::Backward,
                    (1, false) => SoundAction::Forward,
                    (2, true) => SoundAction::Previous(1),
                    (2, false) => SoundAction::Next(1),
                    (_, true) => SoundAction::Minus,
                    (_, false) => SoundAction::Plus,
                };
                let _ = sender.send(action);
                return;
            }
            start = end;
        }
    }

    /// Handles clicks, drags and the wheel over the global player bar.
    fn player_bar_mouse(&mut self, mouse: MouseEvent, bar: Rect) {
        let x = mouse.column.saturating_sub(bar.x);
        let row = mouse.row.saturating_sub(bar.y);

        let volume_width = 14.min(bar.width);
        let on_volume = row >= 2 && x + volume_width >= bar.width;

        match mouse.kind {
            MouseEventKind::ScrollUp if on_volume || row == 3 => {
                self.music_player.sink.volume_up()
            }
            MouseEventKind::ScrollDown if on_volume || row == 3 => {
                self.music_player.sink.volume_down()
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
                if on_volume =>
            {
                let inner = x.saturating_sub(bar.width - volume_width);
                let ratio =
                    (inner as f64 / volume_width.saturating_sub(1).max(1) as f64).clamp(0.0, 1.0);
                self.music_player
                    .sink
                    .set_volume((ratio * 100.0).round() as i32);
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                match row {
                    // Transport buttons: previous, play/pause, next.
                    1 if x < 12 => match x / 4 {
                        0 => {
                            let _ = self
                                .music_player
                                .soundaction_sender
                                .send(SoundAction::Previous(1));
                        }
                        1 => self.music_player.sink.toggle_playback(),
                        _ => {
                            let _ = self
                                .music_player
                                .soundaction_sender
                                .send(SoundAction::Next(1));
                        }
                    },
                    // Clicking the title also toggles playback.
                    1 => self.music_player.sink.toggle_playback(),
                    // Hint line: each hint is a click target.
                    3 if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                        self.player_hint_click(x)
                    }
                    // Progress line: seek (also while dragging).
                    2 => {
                        let ratio = (x as f64 / bar.width.max(1) as f64).clamp(0.0, 1.0);
                        if let Some(total) = self.music_player.total_seconds() {
                            self.music_player
                                .sink
                                .seek_to(Duration::from_secs_f64(total as f64 * ratio));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
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

        // create app and run it
        let tick_rate = Duration::from_millis(250);

        let mut last_tick = Instant::now();
        let mut last_trim = Instant::now();
        let mut last_draw = Instant::now();
        // Redraw only on input/messages or once per tick (progress bar), not on every loop turn.
        let mut dirty = true;
        'a: loop {
            if is_shutdown_sent() {
                break;
            }
            if last_trim.elapsed() >= Duration::from_secs(20) {
                last_trim = Instant::now();
                crate::trim_memory();
            }
            // Paused or empty: nothing moves on screen, so wake up less often.
            let tick_rate = if self.music_player.is_idle() {
                Duration::from_millis(1000)
            } else {
                tick_rate
            };
            while let Ok(e) = updater.try_recv() {
                dirty = true;
                if self.handle_manager_message(e) {
                    break 'a;
                }
            }
            let rectsize = terminal.size()?;
            let (top, side, content, player_bar, queue) = shell_layout(rectsize, self.queue_open);
            if dirty || last_draw.elapsed() >= tick_rate {
                dirty = false;
                last_draw = Instant::now();
                terminal.draw(|f| {
                    self.music_player.update();
                    self.draw(f);
                })?;
            }

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            if crossterm::event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        if (key.code == event::KeyCode::Char('c')
                            || key.code == event::KeyCode::Char('d'))
                            && key.modifiers == KeyModifiers::CONTROL
                        {
                            break;
                        }
                        dirty = true;
                        if key.code == event::KeyCode::F(8) {
                            self.sign_in();
                            continue;
                        }
                        // F6 opens or closes the queue panel.
                        if key.code == event::KeyCode::F(6) {
                            self.queue_open = !self.queue_open;
                            continue;
                        }
                        // F1-F4 jump between sections from the keyboard.
                        let jump = match key.code {
                            event::KeyCode::F(1) => Some(Screens::Playlist),
                            event::KeyCode::F(2) => Some(Screens::Search),
                            event::KeyCode::F(3) => Some(Screens::MusicPlayer),
                            event::KeyCode::F(4) => Some(Screens::PlaylistViewer),
                            _ => None,
                        };
                        let k = match jump {
                            Some(screen) => ManagerMessage::ChangeState(screen).event(),
                            None => self.current_screen().on_key_press(key, &content),
                        };
                        if self.handle_event(k) {
                            break;
                        }
                    }
                    Event::Mouse(mouse) => {
                        // Pure hover moves only need a redraw when they change the sidebar or player bar.
                        let prev = self.mouse_pos;
                        self.mouse_pos = Some((mouse.column, mouse.row));
                        if matches!(mouse.kind, MouseEventKind::Moved) {
                            let hover_zone = |p: Option<(u16, u16)>| {
                                p.map(|(x, y)| {
                                    rect_contains(&side, x, y, 0) || rect_contains(&player_bar, x, y, 0)
                                })
                                .unwrap_or(false)
                            };
                            if prev != self.mouse_pos && (hover_zone(prev) || hover_zone(self.mouse_pos)) {
                                dirty = true;
                            }
                        } else {
                            dirty = true;
                        }
                        let is_down =
                            matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
                        let is_drag =
                            matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left));
                        let is_scroll = matches!(
                            mouse.kind,
                            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                        );

                        // Quit button at the right end of the top bar.
                        if is_down
                            && rect_contains(&top, mouse.column, mouse.row, 0)
                            && mouse.column + 7 >= top.x + top.width
                        {
                            break;
                        }
                        if is_down && rect_contains(&queue_button(top), mouse.column, mouse.row, 0) {
                            self.queue_open = !self.queue_open;
                            continue;
                        }
                        if self.queue_open && rect_contains(&queue, mouse.column, mouse.row, 0) {
                            if is_down {
                                if let Some(index) =
                                    queue_panel::index_at(queue, mouse.row, &self.music_player)
                                {
                                    self.music_player.activate(index);
                                }
                            }
                            continue;
                        }
                        if (is_down || is_drag)
                            && rect_contains(&side, mouse.column, mouse.row, 0)
                        {
                            if is_down {
                                if mouse.row + 1 == side.y + side.height {
                                    self.sign_in();
                                } else if let Some(target) = sidebar_target(mouse.row, side) {
                                    self.set_current_screen(target);
                                }
                            }
                            continue;
                        }
                        if (is_down || is_drag || is_scroll)
                            && rect_contains(&player_bar, mouse.column, mouse.row, 0)
                        {
                            self.player_bar_mouse(mouse, player_bar);
                            continue;
                        }
                        if is_down || is_scroll {
                            let k = self.current_screen().on_mouse_press(mouse, &content);
                            if self.handle_event(k) {
                                break;
                            }
                        }
                    }
                    _ => (),
                }
            }
            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }

        // restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        shutdown();

        Ok(())
    }
}

// UTILS SECTION TO SPLIT THE TERMINAL INTO DIFFERENT PARTS

pub fn split_y_start(f: Rect, start_size: u16) -> [Rect; 2] {
    let mut rectlistvol = f;
    rectlistvol.height = start_size;
    let mut rectprogress = f;
    rectprogress.y += start_size;
    rectprogress.height = rectprogress.height.saturating_sub(start_size);
    [rectlistvol, rectprogress]
}
pub fn split_y(f: Rect, end_size: u16) -> [Rect; 2] {
    let mut rectlistvol = f;
    rectlistvol.height = rectlistvol.height.saturating_sub(end_size);
    let mut rectprogress = f;
    rectprogress.y += rectprogress.height.saturating_sub(end_size);
    rectprogress.height = end_size;
    [rectlistvol, rectprogress]
}

pub fn split_x_start(f: Rect, start_size: u16) -> [Rect; 2] {
    let mut left = f;
    left.width = start_size;
    let mut right = f;
    right.x += start_size;
    right.width = right.width.saturating_sub(start_size);
    [left, right]
}

pub fn rect_contains(rect: &Rect, x: u16, y: u16, margin: u16) -> bool {
    rect.x + margin <= x
        && x <= rect.x + rect.width.saturating_sub(margin)
        && rect.y + margin <= y
        && y <= rect.y + rect.height.saturating_sub(margin)
}

pub fn relative_pos(rect: &Rect, x: u16, y: u16, margin: u16) -> (u16, u16) {
    (
        x.saturating_sub(rect.x + margin),
        y.saturating_sub(rect.y + margin),
    )
}
