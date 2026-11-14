//! Right-hand queue panel: the tracks waiting to play, with the current one highlighted.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::systems::player::PlayerState;

use super::{format_track, ACCENT_BG, BG, TEXT, TEXT_DIM};

/// First visible row, keeping the current track near the middle of the panel.
fn offset(len: usize, current: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    current.saturating_sub(height / 2).min(len - height)
}

/// Pads or cuts `text` to exactly `width` characters.
fn fit(text: &str, width: usize) -> String {
    let mut out: String = text.chars().take(width).collect();
    let used = out.chars().count();
    out.extend(std::iter::repeat(' ').take(width.saturating_sub(used)));
    out
}

/// Draws the queue inside `area`: one header row, then the tracks.
pub fn render(f: &mut Frame, area: Rect, player: &PlayerState) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let bg = Style::default().bg(BG);
    let body = area;
    let position = if player.list.is_empty() {
        0
    } else {
        player.current + 1
    };
    let header = Rect { height: 1, ..body };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Queue",
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {position}/{}", player.list.len()),
                Style::default().fg(TEXT_DIM),
            ),
        ]))
        .style(bg),
        header,
    );

    let rows = Rect {
        y: body.y + 1,
        height: body.height - 1,
        ..body
    };
    if player.list.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                " Nothing queued",
                Style::default().fg(TEXT_DIM),
            ))
            .style(bg),
            rows,
        );
        return;
    }

    let height = rows.height as usize;
    let width = rows.width as usize;
    let start = offset(player.list.len(), player.current, height);
    let lines: Vec<Line> = player
        .list
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, video)| {
            let marker = if index == player.current { ">" } else { " " };
            let text = fit(
                &format!("{marker}{}", format_track(video).trim_start()),
                width,
            );
            let style = if index == player.current {
                Style::default()
                    .fg(TEXT)
                    .bg(ACCENT_BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT_DIM).bg(BG)
            };
            Line::from(Span::styled(text, style))
        })
        .collect();
    f.render_widget(Paragraph::new(lines).style(bg), rows);
    super::draw_scrollbar(f.buffer_mut(), rows, player.list.len(), start);
}

/// The queue index shown on screen row `row`, if any.
pub fn index_at(area: Rect, row: u16, player: &PlayerState) -> Option<usize> {
    let first_row = area.y + 1; // header line, then the list
    if row < first_row || area.height < 3 {
        return None;
    }
    let height = (area.height - 1) as usize;
    let index = offset(player.list.len(), player.current, height) + (row - first_row) as usize;
    (index < player.list.len()).then_some(index)
}
