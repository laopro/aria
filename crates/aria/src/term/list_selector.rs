use crossterm::event::MouseEvent;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::{
    theme::THEME,
    view::{ListMouse, ListView},
    widgets::{columns, draw_scrollbar, hovered},
};

/// The queue list of the "Now playing" screen: a cursor the user moves, next to
/// the track that is playing.
#[derive(Default)]
pub struct ListSelector {
    pub list_size: usize,
    /// The playing track.
    current_position: usize,
    /// The row the user selected.
    scroll_position: usize,
    view: ListView,
    height: usize,
    /// Playing track at the last draw; the view follows it when it changes.
    followed: Option<usize>,
}

/// The rows of the list drawn in `area`: everything under the title line.
fn rows_of(area: Rect) -> Rect {
    Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    }
}

impl ListSelector {
    pub fn is_scrolling(&self) -> bool {
        self.scroll_position != self.current_position
    }

    /// The selected row.
    pub fn cursor(&self) -> usize {
        self.scroll_position
    }

    pub fn play(&mut self) -> Option<usize> {
        self.current_position = self.scroll_position;
        self.select()
    }

    pub fn scroll_down(&mut self) {
        self.scroll_to(self.scroll_position.saturating_add(1));
    }

    pub fn scroll_up(&mut self) {
        self.scroll_to(self.scroll_position.saturating_sub(1));
    }

    pub fn scroll_to(&mut self, position: usize) {
        self.scroll_position = position.min(self.list_size.saturating_sub(1));
        self.view
            .reveal(self.scroll_position, self.list_size, self.height);
    }

    pub fn select(&self) -> Option<usize> {
        (self.current_position < self.list_size).then_some(self.current_position)
    }

    pub fn update(&mut self, list_size: usize, current: usize) {
        if !self.is_scrolling() {
            self.scroll_position = current;
        }
        self.current_position = current;
        self.list_size = list_size;
        self.current_position = self.current_position.min(self.list_size.saturating_sub(1));
        self.scroll_position = self.scroll_position.min(self.list_size.saturating_sub(1));
    }

    /// Handles the mouse for the list drawn in `area`.
    pub fn mouse(&mut self, event: MouseEvent, area: Rect) -> ListMouse {
        let result = self.view.mouse(event, rows_of(area), self.list_size);
        if let ListMouse::Select(index) | ListMouse::Activate(index) | ListMouse::Menu(index) =
            result
        {
            self.scroll_position = index;
        }
        result
    }

    /// Draws the list. `row` gives, for a track index, its style and its label.
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        row: impl Fn(usize) -> (Style, String),
        title: &str,
    ) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let header = format!(
            " {}  {}",
            title.trim(),
            if self.list_size == 0 {
                String::new()
            } else {
                format!("{}/{}", self.current_position + 1, self.list_size)
            }
        );
        buf.set_stringn(
            area.x,
            area.y,
            &header,
            area.width as usize,
            Style::default()
                .fg(THEME.text)
                .bg(THEME.bg)
                .add_modifier(Modifier::BOLD),
        );

        let rows = rows_of(area);
        self.height = rows.height as usize;
        if self.followed != Some(self.current_position) {
            self.followed = Some(self.current_position);
            self.view
                .reveal(self.current_position, self.list_size, self.height);
        }
        self.view.clamp(self.list_size, self.height);
        let bar = ListView::has_scrollbar(rows, self.list_size);
        let width = rows.width as usize - usize::from(bar);

        let first = self.view.offset;
        let last = (first + self.height).min(self.list_size);
        for (line, index) in (first..last).enumerate() {
            let y = rows.y + line as u16;
            let (style, label) = row(index);
            let style = if index == self.scroll_position && self.is_scrolling() {
                style.bg(THEME.hover).add_modifier(Modifier::BOLD)
            } else if index != self.current_position && hovered(rows, y) {
                style.bg(THEME.hover)
            } else {
                style
            };
            // The label starts with a 4-column status marker, then the columns.
            let (marker, rest) = label.split_at(
                label
                    .char_indices()
                    .nth(4)
                    .map(|(i, _)| i)
                    .unwrap_or(label.len()),
            );
            buf.set_string(
                rows.x,
                y,
                format!("{marker}{}", columns(rest, width.saturating_sub(4))),
                style,
            );
        }
        draw_scrollbar(buf, rows, self.list_size, first);
    }
}
