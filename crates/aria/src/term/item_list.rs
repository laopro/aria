use crossterm::event::{KeyCode, KeyEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{List, ListState, Paragraph, StatefulWidget, Widget},
};

use super::{rect_contains, relative_pos, ACCENT_BG, BG, TEXT};

pub trait ListItemAction {
    fn render_style(&self, string: &str, selected: bool) -> Style;
}

pub struct ListItem<Action> {
    list: Vec<(String, Action)>,
    current_position: usize,
    title: String,
}

impl<Action> Default for ListItem<Action> {
    fn default() -> Self {
        Self {
            list: Default::default(),
            current_position: Default::default(),
            title: Default::default(),
        }
    }
}

impl<Action: Clone> ListItem<Action> {
    pub fn new(title: String) -> Self {
        Self {
            list: Default::default(),
            current_position: Default::default(),
            title,
        }
    }

    pub fn on_mouse_press(
        &mut self,
        mouse_event: crossterm::event::MouseEvent,
        frame_data: &Rect,
    ) -> Option<Action> {
        if let MouseEventKind::Down(_) = mouse_event.kind {
            let x = mouse_event.column;
            let y = mouse_event.row;
            if rect_contains(frame_data, x, y, 1) {
                let (_, y) = relative_pos(frame_data, x, y, 1);
                if let Some((i, b)) = self
                    .get_item_frame(frame_data.height as usize)
                    .get(y as usize)
                    .map(|(a, (_, c))| (*a, c.clone()))
                {
                    self.current_position = i;
                    return Some(b);
                }
            }
        } else if let MouseEventKind::ScrollDown = &mouse_event.kind {
            self.select_down();
        } else if let MouseEventKind::ScrollUp = &mouse_event.kind {
            self.select_up();
        }
        None
    }

    pub fn on_key_press(&mut self, key: KeyEvent) -> Option<&Action> {
        match key.code {
            KeyCode::Enter => {
                if let Some(a) = self.select() {
                    return Some(a);
                }
            }
            KeyCode::Char('+') | KeyCode::Up | KeyCode::Char('k') => self.select_up(),
            KeyCode::Char('-') | KeyCode::Down | KeyCode::Char('j') => self.select_down(),
            _ => {}
        }
        None
    }

    pub fn get_item_frame(&self, height: usize) -> Vec<(usize, &(String, Action))> {
        let height = height.saturating_sub(1); // Remove the title line
                                               // Add a little offset when the list is full
        let start = self.current_position.saturating_sub(3);
        let length = self.list.len();
        let length_after_start = length.saturating_sub(start);
        // Tries to take all the space left if length_after_start is smaller than height
        let start = start.saturating_sub(height.saturating_sub(length_after_start));
        self.list
            .iter()
            .enumerate()
            .skip(start)
            .take(height)
            .collect::<Vec<_>>()
    }

    pub fn click_on(&mut self, y_position: usize, height: usize) -> Option<(usize, &Action)> {
        let row = y_position;
        self.get_item_frame(height)
            .iter()
            .enumerate()
            .find(|(i, _)| *i == row)
            .map(|(_, w)| (w.0, &w.1 .1))
    }

    pub fn select(&self) -> Option<&Action> {
        self.list
            .get(self.current_position)
            .map(|(_, action)| action)
    }

    pub fn select_down(&mut self) {
        if self.list.is_empty() {
            return;
        }
        if self.current_position == self.list.len() - 1 {
            self.select_to(0);
        } else {
            self.select_to(self.current_position.saturating_add(1));
        }
    }

    pub fn select_up(&mut self) {
        if self.list.is_empty() {
            return;
        }
        if self.current_position == 0 {
            self.select_to(self.list.len() - 1);
        } else {
            self.select_to(self.current_position.saturating_sub(1));
        }
    }

    pub fn select_to(&mut self, position: usize) {
        self.current_position = position.min(self.list.len().saturating_sub(1));
    }

    pub fn update(&mut self, list: Vec<(String, Action)>, current: usize) {
        self.list = list;
        self.current_position = current.min(self.list.len().saturating_sub(1));
    }

    pub fn update_contents(&mut self, list: Vec<(String, Action)>) {
        self.list = list;
        self.current_position = self.current_position.min(self.list.len().saturating_sub(1));
    }
    pub fn clear(&mut self) {
        self.list.clear();
        self.current_position = 0;
    }

    pub fn add_element(&mut self, element: (String, Action)) {
        self.list.push(element);
    }

    pub fn set_title(&mut self, a: String) {
        self.title = a;
    }

    pub fn current_position(&self) -> usize {
        self.current_position
    }
}

impl<Action: ListItemAction + Clone> Widget for &ListItem<Action> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }
        let header = Rect { height: 1, ..area };
        Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                self.title.trim().to_string(),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(Style::default().bg(BG))
        .render(header, buf);

        let list_area = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(1),
            ..area
        };
        let width = list_area.width as usize;
        StatefulWidget::render(
            List::new(
                self.get_item_frame(list_area.height as usize)
                    .iter()
                    .map(|(i, (string, action))| {
                        let style = if self.current_position == *i {
                            Style::default()
                                .bg(ACCENT_BG)
                                .fg(TEXT)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            action.render_style(string, false)
                        };
                        ratatui::widgets::ListItem::new(Text::from(format!("{string:<width$}")))
                            .style(style)
                    })
                    .collect::<Vec<_>>(),
            ),
            list_area,
            buf,
            &mut ListState::default(),
        );
    }
}
