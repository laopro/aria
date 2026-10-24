pub mod device_lost;
pub mod item_list;
pub mod list_selector;
pub mod music_player;
pub mod playlist;
pub mod playlist_view;
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
    widgets::Paragraph,
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

const SIDEBAR_WIDTH: u16 = 22;
const TOP_BAR_HEIGHT: u16 = 1;
const PLAYER_BAR_HEIGHT: u16 = 4;

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
    (Screens::Playlist, " Home"),
    (Screens::Search, " Search"),
    (Screens::MusicPlayer, " Now playing"),
    (Screens::PlaylistViewer, " Playlist"),
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

/// Splits the terminal into `(top bar, sidebar, content, player bar)`.
fn shell_layout(size: Rect) -> (Rect, Rect, Rect, Rect) {
    let [top, rest] = split_y_start(size, TOP_BAR_HEIGHT);
    let [rest, bottom] = split_y(rest, PLAYER_BAR_HEIGHT);
    let [side, content] = split_x_start(rest, SIDEBAR_WIDTH);
    (top, side, content, bottom)
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
}

impl Manager {
    pub async fn new(action_sender: Sender<SoundAction>, music_player: PlayerState) -> Self {
        Self {
            music_player,
            chooser: Chooser::new(Screens::MusicPlayer),
            playlist_viewer: PlaylistView {
                sender: action_sender.clone(),
                items: ListItem::new(" Playlist ".to_owned()),
                goto: Screens::Playlist,
                videos: Vec::new(),
            },
            search: Search::new(action_sender).await,
            current_screen: Screens::Playlist,
            device_lost: DeviceLost(Vec::new(), None),
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
        let (top, side, content, bottom) = shell_layout(f.size());
        let now_playing = self.music_player.now_playing();

        Self::draw_top_bar(f, top, self.current_screen);
        Self::draw_sidebar(f, side, self.current_screen);
        self.current_screen().render(f, content);
        Self::draw_player_bar(f, bottom, now_playing, self.music_player.sink.volume() as f32);
    }

    fn draw_top_bar(f: &mut Frame, area: Rect, screen: Screens) {
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
    }

    fn draw_sidebar(f: &mut Frame, area: Rect, screen: Screens) {
        let inner_width = area.width.saturating_sub(2) as usize;
        let mut lines = vec![Line::from("")];
        for (target, label) in SIDEBAR_SECTIONS {
            let active = target == screen;
            let text_style = if active {
                Style::default()
                    .fg(TEXT)
                    .bg(ACCENT_BG)
                    .add_modifier(Modifier::BOLD)
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

    fn draw_player_bar(f: &mut Frame, area: Rect, now: Option<NowPlaying>, volume: f32) {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

        // Thin rule above the player.
        f.render_widget(
            Paragraph::new(Span::styled(
                "─".repeat(area.width as usize),
                Style::default().fg(BORDER),
            ))
            .style(Style::default().bg(BG)),
            rows[0],
        );

        let (title, artist) = match &now {
            Some(n) => (n.title.clone(), n.artist.clone()),
            None => ("Nothing playing".to_string(), String::new()),
        };
        let play_icon = if now.as_ref().map(|n| n.paused).unwrap_or(true) {
            ">"
        } else {
            "||"
        };

        let mut title_spans = vec![
            Span::styled(" |< ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                format!(" {play_icon} "),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" >| ", Style::default().fg(TEXT_DIM)),
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
        let filled = (ratio * track_width as f64).round() as usize;
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("━".repeat(filled), Style::default().fg(ACCENT)),
                Span::styled(
                    "─".repeat(track_width.saturating_sub(filled)),
                    Style::default().fg(BORDER),
                ),
            ]))
            .style(Style::default().bg(BG)),
            rows[2],
        );

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Space", Style::default().fg(ACCENT)),
                Span::styled(" pause   ", Style::default().fg(TEXT_DIM)),
                Span::styled("←/→", Style::default().fg(ACCENT)),
                Span::styled(" 10s   ", Style::default().fg(TEXT_DIM)),
                Span::styled("Ctrl+←/→", Style::default().fg(ACCENT)),
                Span::styled(" track   ", Style::default().fg(TEXT_DIM)),
                Span::styled("+/-", Style::default().fg(ACCENT)),
                Span::styled(" volume", Style::default().fg(TEXT_DIM)),
            ]))
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
                    1 if x < 9 => match x / 3 {
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
        'a: loop {
            if is_shutdown_sent() {
                break;
            }
            while let Ok(e) = updater.try_recv() {
                if self.handle_manager_message(e) {
                    break 'a;
                }
            }
            let rectsize = terminal.size()?;
            let (top, side, content, player_bar) = shell_layout(rectsize);
            terminal.draw(|f| {
                self.music_player.update();
                self.draw(f);
            })?;

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
                        let k = self.current_screen().on_key_press(key, &content);
                        if self.handle_event(k) {
                            break;
                        }
                    }
                    Event::Mouse(mouse) => {
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
                        if (is_down || is_drag)
                            && rect_contains(&side, mouse.column, mouse.row, 0)
                        {
                            if is_down {
                                if let Some(target) = sidebar_target(mouse.row, side) {
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
