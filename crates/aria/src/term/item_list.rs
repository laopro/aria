use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
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

pub trait ListItemAction {
    fn render_style(&self, string: &str, selected: bool) -> Style;
}

/// What a click on the list asks for.
pub enum ListEvent<Action> {
    /// Double click: play or open.
    Activate(Action),
    /// Right click: context menu.
    Menu(Action),
}

/// A titled, scrollable list of labelled actions. Labels may hold tab-separated
/// fields, which are drawn as aligned columns.
pub struct ListItem<Action> {
    list: Vec<(String, Action)>,
    current_position: usize,
    title: String,
    view: ListView,
    /// Rows the list had when last drawn; used to keep the selection visible.
    height: usize,
    /// Shown when the list is empty.
    pub placeholder: String,
    /// Draws placeholder rows under the items while more are on the way.
    pub loading: bool,
}

impl<Action> Default for ListItem<Action> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            current_position: 0,
            title: String::new(),
            view: ListView::default(),
            height: 0,
            placeholder: String::new(),
            loading: false,
        }
    }
}

/// The rows of a list drawn in `area`: everything under the title line.
fn rows_of(area: Rect) -> Rect {
    Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    }
}

impl<Action: Clone> ListItem<Action> {
    pub fn new(title: String) -> Self {
        Self {
            title,
            ..Self::default()
        }
    }

    pub fn on_mouse_press(&mut self, event: MouseEvent, area: &Rect) -> Option<ListEvent<Action>> {
        let rows = rows_of(*area);
        match self.view.mouse(event, rows, self.list.len()) {
            ListMouse::None => None,
            ListMouse::Select(index) => {
                self.current_position = index;
                None
            }
            ListMouse::Activate(index) => {
                self.current_position = index;
                self.list
                    .get(index)
                    .map(|(_, action)| ListEvent::Activate(action.clone()))
            }
            ListMouse::Menu(index) => {
                self.current_position = index;
                self.list
                    .get(index)
                    .map(|(_, action)| ListEvent::Menu(action.clone()))
            }
        }
    }

    pub fn on_key_press(&mut self, key: KeyEvent) -> Option<&Action> {
        let page = self.height.max(2) as isize - 1;
        match key.code {
            KeyCode::Enter => return self.select(),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.move_by(-page),
            KeyCode::PageDown => self.move_by(page),
            KeyCode::Home => self.select_to(0),
            KeyCode::End => self.select_to(usize::MAX),
            _ => {}
        }
        None
    }

    pub fn move_by(&mut self, delta: isize) {
        self.select_to(self.current_position.saturating_add_signed(delta));
    }

    pub fn select(&self) -> Option<&Action> {
        self.list.get(self.current_position).map(|(_, action)| action)
    }

    pub fn select_to(&mut self, position: usize) {
        self.current_position = position.min(self.list.len().saturating_sub(1));
        self.view
            .reveal(self.current_position, self.list.len(), self.height);
    }

    pub fn update(&mut self, list: Vec<(String, Action)>, current: usize) {
        self.list = list;
        self.view.offset = 0;
        self.select_to(current);
    }

    pub fn update_contents(&mut self, list: Vec<(String, Action)>) {
        self.list = list;
        self.current_position = self.current_position.min(self.list.len().saturating_sub(1));
        self.view.clamp(self.list.len(), self.height);
    }

    /// Direct access to the rows; call `refresh` after changing them.
    pub fn items_mut(&mut self) -> &mut Vec<(String, Action)> {
        &mut self.list
    }

    /// Puts the selection and the view back inside the list after it changed.
    pub fn refresh(&mut self) {
        self.current_position = self.current_position.min(self.list.len().saturating_sub(1));
        self.view.clamp(self.list.len(), self.height);
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    pub fn current_position(&self) -> usize {
        self.current_position
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }
}

impl<Action: ListItemAction + Clone> ListItem<Action> {
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let total = self.list.len();
        let header = format!(
            " {}  {}",
            self.title.trim(),
            if total == 0 {
                String::new()
            } else {
                format!("{}/{}", self.current_position + 1, total)
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
        self.view.clamp(total, self.height);
        let bar = ListView::has_scrollbar(rows, total);
        let width = rows.width as usize - usize::from(bar);

        if total == 0 && !self.placeholder.is_empty() && rows.height > 0 {
            buf.set_stringn(
                rows.x,
                rows.y,
                format!(" {}", self.placeholder),
                width,
                Style::default().fg(THEME.text_dim).bg(THEME.bg),
            );
        }

        let mut drawn = 0;
        for (line, (index, (label, action))) in self
            .list
            .iter()
            .enumerate()
            .skip(self.view.offset)
            .take(self.height)
            .enumerate()
        {
            let y = rows.y + line as u16;
            let style = if index == self.current_position {
                Style::default()
                    .fg(THEME.text)
                    .bg(THEME.accent_bg)
                    .add_modifier(Modifier::BOLD)
            } else if hovered(rows, y) {
                action.render_style(label, false).bg(THEME.hover)
            } else {
                action.render_style(label, false)
            };
            buf.set_string(rows.x, y, format!(" {}", columns(label, width.saturating_sub(1))), style);
            drawn += 1;
        }

        if self.loading {
            let skeleton: String = "░".repeat(width.saturating_sub(2).min(40));
            for line in drawn..(drawn + 3).min(self.height) {
                buf.set_string(
                    rows.x + 1,
                    rows.y + line as u16,
                    &skeleton,
                    Style::default().fg(THEME.border).bg(THEME.bg),
                );
            }
        }

        draw_scrollbar(buf, rows, total, self.view.offset);
    }
}
