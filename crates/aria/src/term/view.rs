//! Scroll and click behaviour shared by every list: the visible window moves
//! on its own (wheel, scrollbar) and does not drag the selection with it.

use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

/// What a mouse event over a list asks the list owner to do.
pub enum ListMouse {
    None,
    /// Single click: select the row.
    Select(usize),
    /// Double click: play or open the row.
    Activate(usize),
    /// Right click: open the context menu of the row.
    Menu(usize),
}

/// Rows scrolled per wheel step.
const WHEEL_STEP: isize = 3;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Default)]
pub struct ListView {
    /// Index of the first visible row.
    pub offset: usize,
    last_click: Option<(Instant, usize)>,
    dragging: bool,
}

impl ListView {
    pub fn clamp(&mut self, total: usize, height: usize) {
        self.offset = self.offset.min(total.saturating_sub(height));
    }

    pub fn scroll(&mut self, delta: isize, total: usize, height: usize) {
        self.offset = self.offset.saturating_add_signed(delta);
        self.clamp(total, height);
    }

    /// Scrolls just enough for `index` to be visible.
    pub fn reveal(&mut self, index: usize, total: usize, height: usize) {
        if height == 0 {
            return;
        }
        if index < self.offset {
            self.offset = index;
        } else if index >= self.offset + height {
            self.offset = index + 1 - height;
        }
        self.clamp(total, height);
    }

    /// Index of the row drawn on screen line `y` of `rows`.
    pub fn index_at(&self, rows: Rect, y: u16, total: usize) -> Option<usize> {
        if y < rows.y || y >= rows.y + rows.height {
            return None;
        }
        let index = self.offset + (y - rows.y) as usize;
        (index < total).then_some(index)
    }

    /// True when the list overflows, so its last column holds a scrollbar.
    pub fn has_scrollbar(rows: Rect, total: usize) -> bool {
        total > rows.height as usize && rows.width > 1
    }

    /// Moves the window so the scrollbar thumb follows screen line `y`.
    fn jump(&mut self, rows: Rect, y: u16, total: usize) {
        let height = rows.height as usize;
        if total <= height || height < 2 {
            return;
        }
        let line = y.saturating_sub(rows.y).min(rows.height - 1) as usize;
        self.offset = (line * (total - height) + (height - 2) / 2) / (height - 1);
    }

    /// A second click on the same row within a moment is a double click.
    fn click(&mut self, index: usize) -> bool {
        let double = matches!(
            self.last_click,
            Some((at, last)) if last == index && at.elapsed() < DOUBLE_CLICK
        );
        self.last_click = if double {
            None
        } else {
            Some((Instant::now(), index))
        };
        double
    }

    /// Handles a mouse event for a list whose rows are drawn in `rows`.
    pub fn mouse(&mut self, event: MouseEvent, rows: Rect, total: usize) -> ListMouse {
        let height = rows.height as usize;
        let on_bar = Self::has_scrollbar(rows, total)
            && event.column + 1 == rows.x + rows.width
            && event.row >= rows.y
            && event.row < rows.y + rows.height;
        match event.kind {
            MouseEventKind::ScrollDown => self.scroll(WHEEL_STEP, total, height),
            MouseEventKind::ScrollUp => self.scroll(-WHEEL_STEP, total, height),
            MouseEventKind::Down(MouseButton::Left) if on_bar => {
                self.dragging = true;
                self.jump(rows, event.row, total);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                self.jump(rows, event.row, total);
            }
            MouseEventKind::Up(_) => self.dragging = false,
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = self.index_at(rows, event.row, total) {
                    return if self.click(index) {
                        ListMouse::Activate(index)
                    } else {
                        ListMouse::Select(index)
                    };
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(index) = self.index_at(rows, event.row, total) {
                    return ListMouse::Menu(index);
                }
            }
            _ => {}
        }
        ListMouse::None
    }
}
