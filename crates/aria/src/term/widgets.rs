//! Small drawing helpers shared by the screens.

use std::{
    iter::repeat,
    sync::atomic::{AtomicU32, Ordering},
    time::Instant,
};

use once_cell::sync::Lazy;
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::THEME;

/// Video id of the track that is playing, so every list can mark it.
static NOW_PLAYING: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

pub fn set_now_playing(video_id: &str) {
    if *NOW_PLAYING.read().unwrap() != video_id {
        *NOW_PLAYING.write().unwrap() = video_id.to_owned();
    }
}

pub fn is_now_playing(video_id: &str) -> bool {
    !video_id.is_empty() && *NOW_PLAYING.read().unwrap() == video_id
}

/// Mouse position, packed as `x << 16 | y`; `u32::MAX` when unknown.
static HOVER: AtomicU32 = AtomicU32::new(u32::MAX);

pub fn set_hover(position: Option<(u16, u16)>) {
    let packed = position
        .map(|(x, y)| (x as u32) << 16 | y as u32)
        .unwrap_or(u32::MAX);
    HOVER.store(packed, Ordering::Relaxed);
}

pub fn hover() -> Option<(u16, u16)> {
    let packed = HOVER.load(Ordering::Relaxed);
    (packed != u32::MAX).then(|| ((packed >> 16) as u16, (packed & 0xFFFF) as u16))
}

/// True when the mouse is over screen line `y` inside the columns of `area`.
pub fn hovered(area: Rect, y: u16) -> bool {
    hover()
        .map(|(hx, hy)| hy == y && hx >= area.x && hx < area.x + area.width)
        .unwrap_or(false)
}

/// Cuts `text` to `width` display columns (ending in "…") and pads it with spaces.
pub fn fit(text: &str, width: usize) -> String {
    let text_width = UnicodeWidthStr::width(text);
    if text_width <= width {
        let mut out = text.to_owned();
        out.extend(repeat(' ').take(width - text_width));
        return out;
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out.extend(repeat(' ').take(width.saturating_sub(used + 1)));
    out
}

/// Like `fit`, but aligned to the right.
pub fn fit_right(text: &str, width: usize) -> String {
    let text_width = UnicodeWidthStr::width(text);
    if text_width >= width {
        return fit(text, width);
    }
    let mut out: String = repeat(' ').take(width - text_width).collect();
    out.push_str(text);
    out
}

/// Lays out a label whose fields are separated by tabs (`title\tartist\tduration`,
/// or `name\tdetail`) in aligned columns `width` wide.
pub fn columns(label: &str, width: usize) -> String {
    let parts: Vec<&str> = label.split('\t').collect();
    match parts.as_slice() {
        [title, artist, duration, ..] => {
            let duration_width = 6.min(width);
            let rest = width.saturating_sub(duration_width + 2);
            if width < 44 {
                // Narrow: the artist does not fit next to a readable title.
                return format!(
                    "{}  {}",
                    fit(title, rest),
                    fit_right(duration, duration_width)
                );
            }
            let artist_width = (rest * 2 / 5).min(32);
            let title_width = rest.saturating_sub(artist_width + 2);
            format!(
                "{}  {}  {}",
                fit(title, title_width),
                fit(artist, artist_width),
                fit_right(duration, duration_width)
            )
        }
        [name, detail] => {
            let detail_width = (width / 3).min(36);
            let name_width = width.saturating_sub(detail_width + 2);
            format!("{}  {}", fit(name, name_width), fit(detail, detail_width))
        }
        [one] => fit(one, width),
        [] => fit("", width),
    }
}

/// Draws a scrollbar in the last column of `rows` when `total` rows do not fit.
/// `first` is the index of the top visible row.
pub fn draw_scrollbar(buf: &mut Buffer, rows: Rect, total: usize, first: usize) {
    let height = rows.height as usize;
    if height == 0 || rows.width < 2 || total <= height {
        return;
    }
    let thumb = (height * height / total).max(1);
    let last_first = total - height;
    let top = first.min(last_first) * (height - thumb) / last_first;
    let x = rows.x + rows.width - 1;
    let over = hover()
        .map(|(hx, hy)| hx == x && hy >= rows.y && hy < rows.y + rows.height)
        .unwrap_or(false);
    for line in 0..height {
        let (symbol, color) = if line >= top && line < top + thumb {
            ("█", if over { THEME.text } else { THEME.text_dim })
        } else {
            ("│", THEME.border)
        };
        buf.set_string(
            x,
            rows.y + line as u16,
            symbol,
            Style::default().fg(color).bg(THEME.bg),
        );
    }
}

/// Frame of the small "now playing" equalizer; it moves while music plays.
pub fn equalizer(playing: bool) -> &'static str {
    const FRAMES: [&str; 6] = ["▁▃▅", "▃▅▇", "▅▇▅", "▇▅▃", "▅▃▁", "▃▁▃"];
    static START: Lazy<Instant> = Lazy::new(Instant::now);
    if !playing {
        return "▮▮ ";
    }
    FRAMES[(START.elapsed().as_millis() / 250 % FRAMES.len() as u128) as usize]
}
