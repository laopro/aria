//! Context menu: a small popup with actions for one track, opened with a right
//! click or the `m` key.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, Clear},
    Frame,
};
use ytpapi2::YoutubeMusicVideoRef;

use super::{
    theme::THEME,
    widgets::{fit, hovered},
};

/// What the menu is about.
#[derive(Debug, Clone)]
pub enum MenuTarget {
    /// A track from a search or a playlist; the browse id is the account
    /// playlist it is shown in, when there is one.
    Video(YoutubeMusicVideoRef, Option<String>),
    /// The queue entry at this index.
    Queue(usize),
}

#[derive(Debug, Clone)]
pub enum MenuAction {
    PlayNow(YoutubeMusicVideoRef),
    PlayNext(YoutubeMusicVideoRef),
    AddToQueue(YoutubeMusicVideoRef),
    PlayIndex(usize),
    RemoveAt(usize),
    SetLiked(YoutubeMusicVideoRef, bool),
    /// Opens the list of playlists to add the track to.
    ChoosePlaylist(YoutubeMusicVideoRef),
    AddToPlaylist {
        browse_id: String,
        name: String,
        video: YoutubeMusicVideoRef,
    },
    RemoveFromPlaylist {
        browse_id: String,
        video_id: String,
    },
    OpenInBrowser(String),
    CopyLink(String),
}

pub struct Menu {
    /// Where it was opened; the popup stays inside the screen.
    x: u16,
    y: u16,
    title: String,
    items: Vec<(String, MenuAction)>,
    pub selected: usize,
}

const WIDTH: u16 = 30;

impl Menu {
    /// Builds the menu for a track. `queue_index` is set for a queue entry and
    /// `playlist` when the track is listed inside an account playlist.
    pub fn for_track(
        video: YoutubeMusicVideoRef,
        queue_index: Option<usize>,
        playlist: Option<String>,
        liked: bool,
        x: u16,
        y: u16,
    ) -> Self {
        let id = video.video_id.clone();
        let mut items: Vec<(String, MenuAction)> = Vec::new();
        let mut add = |label: &str, action: MenuAction| items.push((label.to_owned(), action));
        match queue_index {
            Some(index) => {
                add("Play", MenuAction::PlayIndex(index));
                add("Play next", MenuAction::PlayNext(video.clone()));
                add("Remove from queue", MenuAction::RemoveAt(index));
            }
            None => {
                add("Play now", MenuAction::PlayNow(video.clone()));
                add("Play next", MenuAction::PlayNext(video.clone()));
                add("Add to queue", MenuAction::AddToQueue(video.clone()));
            }
        }
        add(
            if liked { "♥ Remove like" } else { "♡ Like" },
            MenuAction::SetLiked(video.clone(), !liked),
        );
        add("Add to playlist…", MenuAction::ChoosePlaylist(video.clone()));
        if let Some(browse_id) = playlist {
            add(
                "Remove from this playlist",
                MenuAction::RemoveFromPlaylist {
                    browse_id,
                    video_id: id.clone(),
                },
            );
        }
        add("Open in browser", MenuAction::OpenInBrowser(id.clone()));
        add("Copy link", MenuAction::CopyLink(id));
        Self {
            x,
            y,
            title: video.title,
            items,
            selected: 0,
        }
    }

    /// The second step of "Add to playlist…": one row per account playlist.
    pub fn for_playlists(
        video: YoutubeMusicVideoRef,
        playlists: Vec<(String, String)>,
        x: u16,
        y: u16,
    ) -> Self {
        Self {
            x,
            y,
            title: "Add to playlist".to_owned(),
            items: playlists
                .into_iter()
                .map(|(name, browse_id)| {
                    (
                        name.clone(),
                        MenuAction::AddToPlaylist {
                            browse_id,
                            name,
                            video: video.clone(),
                        },
                    )
                })
                .collect(),
            selected: 0,
        }
    }

    /// The popup rectangle, kept inside `screen`.
    pub fn area(&self, screen: Rect) -> Rect {
        let height = (self.items.len() as u16 + 2).min(screen.height);
        let width = WIDTH.min(screen.width);
        Rect {
            x: self.x.min(screen.x + screen.width.saturating_sub(width)),
            y: self.y.min(screen.y + screen.height.saturating_sub(height)),
            width,
            height,
        }
    }

    /// First item drawn: the list scrolls with the selection when it is too long.
    fn first(&self, area: Rect) -> usize {
        let rows = area.height.saturating_sub(2) as usize;
        (self.selected + 1).saturating_sub(rows)
    }

    pub fn item_at(&self, screen: Rect, x: u16, y: u16) -> Option<usize> {
        let area = self.area(screen);
        if x <= area.x || x + 1 >= area.x + area.width || y <= area.y || y + 1 >= area.y + area.height
        {
            return None;
        }
        let index = self.first(area) + (y - area.y - 1) as usize;
        (index < self.items.len()).then_some(index)
    }

    pub fn contains(&self, screen: Rect, x: u16, y: u16) -> bool {
        let area = self.area(screen);
        x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
    }

    pub fn action(&self, index: usize) -> Option<MenuAction> {
        self.items.get(index).map(|(_, action)| action.clone())
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.items.len().max(1) as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    pub fn render(&self, f: &mut Frame, screen: Rect) {
        let area = self.area(screen);
        f.render_widget(Clear, area);
        let inner_width = area.width.saturating_sub(2) as usize;
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(THEME.accent))
                .title(fit(&format!(" {} ", self.title), inner_width))
                .style(Style::default().bg(THEME.bg).fg(THEME.text)),
            area,
        );
        let buf = f.buffer_mut();
        let rows = area.height.saturating_sub(2) as usize;
        for (line, (index, (label, _))) in self
            .items
            .iter()
            .enumerate()
            .skip(self.first(area))
            .take(rows)
            .enumerate()
        {
            let y = area.y + 1 + line as u16;
            let row = Rect {
                x: area.x + 1,
                y,
                width: area.width.saturating_sub(2),
                height: 1,
            };
            let style = if index == self.selected || hovered(row, y) {
                Style::default()
                    .fg(THEME.text)
                    .bg(THEME.accent_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(THEME.text).bg(THEME.bg)
            };
            buf.set_string(row.x, y, fit(&format!(" {label}"), inner_width), style);
        }
    }
}
