//! Right-hand panel with two tabs: the queue ("Up next") and the lyrics of the
//! playing track.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use unicode_width::UnicodeWidthChar;

use crate::systems::player::PlayerState;

use super::{
    format_track,
    theme::THEME,
    view::{ListMouse, ListView},
    widgets::{columns, draw_scrollbar, equalizer, fit, hovered},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Panel {
    Queue,
    Lyrics,
}

pub enum LyricsState {
    Loading,
    Missing,
    Text(String),
}

/// What a click on the panel asks the manager to do.
pub enum PanelMouse {
    None,
    Play(usize),
    Menu(usize),
}

pub struct SidePanel {
    pub tab: Panel,
    queue_view: ListView,
    /// Playing track at the last draw; the view jumps to it when it changes.
    followed: Option<usize>,
    lyrics_view: ListView,
    /// Track the lyrics belong to.
    pub lyrics_id: String,
    pub lyrics: LyricsState,
    /// Rows the "now playing" card took at the last draw (0 when hidden).
    card_rows: u16,
}

/// Cover size of the card, in cells: 16 x 8 cells show 32 x 16 pixels.
const CARD_COVER: (u16, u16) = (16, 8);

const TABS: [(Panel, &str); 2] = [(Panel::Queue, " Up next "), (Panel::Lyrics, " Lyrics ")];

fn tab_rects(area: Rect) -> Vec<(Rect, Panel, &'static str)> {
    let mut x = area.x;
    TABS.iter()
        .map(|(panel, label)| {
            let width = label.chars().count() as u16;
            let rect = Rect {
                x,
                y: area.y,
                width,
                height: 1,
            };
            x += width + 1;
            (rect, *panel, *label)
        })
        .collect()
}

fn rows_of(area: Rect) -> Rect {
    Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    }
}

/// Splits `text` into lines at most `width` columns wide, breaking at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split(' ') {
            let word_width: usize = word.chars().map(|c| c.width().unwrap_or(0)).sum();
            if used > 0 && used + 1 + word_width > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            if used > 0 {
                line.push(' ');
                used += 1;
            }
            line.push_str(word);
            used += word_width;
        }
        lines.push(line);
    }
    lines
}

impl SidePanel {
    pub fn new(tab: Panel) -> Self {
        Self {
            tab,
            queue_view: ListView::default(),
            followed: None,
            lyrics_view: ListView::default(),
            lyrics_id: String::new(),
            lyrics: LyricsState::Loading,
            card_rows: 0,
        }
    }

    pub fn set_lyrics(&mut self, video_id: String, lyrics: Option<String>) {
        if video_id != self.lyrics_id {
            return;
        }
        self.lyrics = match lyrics {
            Some(text) => LyricsState::Text(text),
            None => LyricsState::Missing,
        };
        self.lyrics_view.offset = 0;
    }

    /// Marks the lyrics of `video_id` as loading; false if they already were requested.
    pub fn want_lyrics(&mut self, video_id: &str) -> bool {
        if self.lyrics_id == video_id {
            return false;
        }
        self.lyrics_id = video_id.to_owned();
        self.lyrics = LyricsState::Loading;
        self.lyrics_view.offset = 0;
        true
    }

    fn lyrics_lines(&self, width: usize) -> Vec<String> {
        match &self.lyrics {
            LyricsState::Text(text) => wrap(text, width),
            _ => Vec::new(),
        }
    }

    /// The part of the panel under the card.
    fn below_card(&self, area: Rect) -> Rect {
        Rect {
            y: area.y + self.card_rows,
            height: area.height.saturating_sub(self.card_rows),
            ..area
        }
    }

    /// Draws the big cover with the track's title, artist and album next to it.
    fn render_card(&self, buf: &mut Buffer, area: Rect, player: &PlayerState, cover: &[u8]) {
        let Some(video) = player.current() else {
            return;
        };
        let cover_rect = Rect {
            x: area.x + 1,
            y: area.y,
            width: CARD_COVER.0,
            height: CARD_COVER.1,
        };
        crate::covers::draw(buf, cover_rect, cover);
        let x = cover_rect.x + cover_rect.width + 2;
        let width = (area.x + area.width).saturating_sub(x + 1) as usize;
        let mut y = area.y + 1;
        let bold = Style::default()
            .fg(THEME.text)
            .bg(THEME.bg)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(THEME.text_dim).bg(THEME.bg);
        for line in wrap(&video.title, width).into_iter().take(3) {
            buf.set_stringn(x, y, line, width, bold);
            y += 1;
        }
        for text in [&video.author, &video.album] {
            if !text.is_empty() && y < area.y + CARD_COVER.1 {
                buf.set_string(x, y, fit(text, width), dim);
                y += 1;
            }
        }
        if crate::account::is_liked(&video.video_id) && y < area.y + CARD_COVER.1 {
            buf.set_string(x, y, "♥ Liked", Style::default().fg(THEME.accent).bg(THEME.bg));
        }
    }

    pub fn mouse(&mut self, event: MouseEvent, area: Rect, player: &PlayerState) -> PanelMouse {
        let area = self.below_card(area);
        if event.row < area.y {
            return PanelMouse::None;
        }
        if event.row == area.y {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind {
                if let Some((_, panel, _)) = tab_rects(area)
                    .into_iter()
                    .find(|(rect, _, _)| event.column >= rect.x && event.column < rect.x + rect.width)
                {
                    self.tab = panel;
                }
            }
            return PanelMouse::None;
        }
        let rows = rows_of(area);
        match self.tab {
            Panel::Queue => match self.queue_view.mouse(event, rows, player.list.len()) {
                ListMouse::Activate(index) => PanelMouse::Play(index),
                ListMouse::Menu(index) => PanelMouse::Menu(index),
                _ => PanelMouse::None,
            },
            Panel::Lyrics => {
                let total = self.lyrics_lines(rows.width.saturating_sub(2) as usize).len();
                self.lyrics_view.mouse(event, rows, total);
                PanelMouse::None
            }
        }
    }

    /// Draws the panel. `cover` holds the pixels of the playing track's cover.
    pub fn render(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        player: &PlayerState,
        cover: Option<&[u8]>,
    ) {
        if area.width < 6 || area.height < 3 {
            return;
        }
        // The card needs room: without it the queue keeps the whole panel.
        self.card_rows = 0;
        if let Some(cover) = cover {
            if area.height >= CARD_COVER.1 + 12 && area.width >= CARD_COVER.0 + 14 {
                self.render_card(buf, area, player, cover);
                self.card_rows = CARD_COVER.1 + 1;
            }
        }
        let area = self.below_card(area);
        for (rect, panel, label) in tab_rects(area) {
            let style = if panel == self.tab {
                Style::default()
                    .fg(THEME.text)
                    .bg(THEME.accent_bg)
                    .add_modifier(Modifier::BOLD)
            } else if hovered(rect, rect.y) {
                Style::default().fg(THEME.text).bg(THEME.hover)
            } else {
                Style::default().fg(THEME.text_dim).bg(THEME.bg)
            };
            buf.set_stringn(rect.x, rect.y, label, rect.width as usize, style);
        }

        let rows = rows_of(area);
        match self.tab {
            Panel::Queue => self.render_queue(buf, rows, player),
            Panel::Lyrics => self.render_lyrics(buf, rows, player),
        }
    }

    fn render_queue(&mut self, buf: &mut Buffer, rows: Rect, player: &PlayerState) {
        let total = player.list.len();
        let height = rows.height as usize;
        if total == 0 {
            buf.set_stringn(
                rows.x,
                rows.y,
                " Nothing queued",
                rows.width as usize,
                Style::default().fg(THEME.text_dim).bg(THEME.bg),
            );
            return;
        }
        if self.followed != Some(player.current) {
            self.followed = Some(player.current);
            // Keep a little of what already played above the current track.
            self.queue_view.offset = player.current.saturating_sub(2);
        }
        self.queue_view.clamp(total, height);
        let bar = ListView::has_scrollbar(rows, total);
        let width = rows.width as usize - usize::from(bar);
        let playing = !player.sink.is_paused();
        for (line, (index, video)) in player
            .list
            .iter()
            .enumerate()
            .skip(self.queue_view.offset)
            .take(height)
            .enumerate()
        {
            let y = rows.y + line as u16;
            let current = index == player.current;
            let marker = if current { equalizer(playing) } else { "   " };
            let text = format!(
                "{marker} {}",
                columns(&format_track(video), width.saturating_sub(4))
            );
            let style = if current {
                Style::default()
                    .fg(THEME.text)
                    .bg(THEME.accent_bg)
                    .add_modifier(Modifier::BOLD)
            } else if hovered(rows, y) {
                Style::default().fg(THEME.text).bg(THEME.hover)
            } else if index < player.current {
                Style::default().fg(THEME.border).bg(THEME.bg)
            } else {
                Style::default().fg(THEME.text_dim).bg(THEME.bg)
            };
            buf.set_string(rows.x, y, text, style);
        }
        draw_scrollbar(buf, rows, total, self.queue_view.offset);
    }

    fn render_lyrics(&mut self, buf: &mut Buffer, rows: Rect, player: &PlayerState) {
        let dim = Style::default().fg(THEME.text_dim).bg(THEME.bg);
        let message = match (&self.lyrics, player.current()) {
            (_, None) => Some("Nothing playing"),
            (LyricsState::Loading, _) => Some("Loading lyrics…"),
            (LyricsState::Missing, _) => Some("No lyrics for this track"),
            (LyricsState::Text(_), _) => None,
        };
        if let Some(message) = message {
            buf.set_stringn(rows.x + 1, rows.y, message, rows.width as usize, dim);
            return;
        }
        let text_width = rows.width.saturating_sub(2) as usize;
        let lines = self.lyrics_lines(text_width);
        let height = rows.height as usize;
        self.lyrics_view.clamp(lines.len(), height);
        for (line, text) in lines
            .iter()
            .skip(self.lyrics_view.offset)
            .take(height)
            .enumerate()
        {
            buf.set_string(
                rows.x + 1,
                rows.y + line as u16,
                fit(text, text_width),
                Style::default().fg(THEME.text).bg(THEME.bg),
            );
        }
        draw_scrollbar(buf, rows, lines.len(), self.lyrics_view.offset);
    }
}
