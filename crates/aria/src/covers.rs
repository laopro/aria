//! Album covers of the tracks that were played, saved on disk as small 32x32
//! RGB thumbnails (3 KB each). Only the playing track's cover is in memory.

use std::{
    io::Read,
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};

use crate::consts::CACHE_DIR;

/// Side of the saved thumbnail, in pixels.
pub const SIDE: usize = 32;

/// Tracks whose cover is being downloaded right now.
static FETCHING: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn path(video_id: &str) -> PathBuf {
    CACHE_DIR.join("covers").join(format!("{video_id}.rgb"))
}

/// The saved cover of a track played before.
pub fn load(video_id: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path(video_id)).ok()?;
    (bytes.len() == SIDE * SIDE * 3).then_some(bytes)
}

/// Downloads and saves the cover of a playing track, in the background.
pub fn fetch(video_id: &str) {
    // A file of another size is a thumbnail from an older version: replace it.
    if load(video_id).is_some() {
        return;
    }
    {
        let mut fetching = FETCHING.lock().unwrap();
        if fetching.iter().any(|id| id == video_id) {
            return;
        }
        fetching.push(video_id.to_owned());
    }
    let id = video_id.to_owned();
    std::thread::spawn(move || {
        if let Some(cover) = download(&id) {
            let _ = std::fs::create_dir_all(CACHE_DIR.join("covers"));
            let _ = std::fs::write(path(&id), cover);
        }
        FETCHING.lock().unwrap().retain(|x| x != &id);
    });
}

fn download(video_id: &str) -> Option<Vec<u8>> {
    let url = format!("https://i.ytimg.com/vi/{video_id}/mqdefault.jpg");
    let response = ureq::get(&url)
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?;
    let mut jpeg = Vec::new();
    response
        .into_reader()
        .take(1_000_000)
        .read_to_end(&mut jpeg)
        .ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(jpeg));
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (width, height) = (info.width as usize, info.height as usize);
    let channels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => 3,
        jpeg_decoder::PixelFormat::L8 => 1,
        _ => return None,
    };
    if width == 0 || height == 0 || pixels.len() < width * height * channels {
        return None;
    }
    // Music videos show the square album art between side bars: keep the center.
    let side = width.min(height);
    let (x0, y0) = ((width - side) / 2, (height - side) / 2);
    let mut out = vec![0u8; SIDE * SIDE * 3];
    for cy in 0..SIDE {
        for cx in 0..SIDE {
            let (sx0, sx1) = (x0 + cx * side / SIDE, x0 + (cx + 1) * side / SIDE);
            let (sy0, sy1) = (y0 + cy * side / SIDE, y0 + (cy + 1) * side / SIDE);
            let mut sum = [0u64; 3];
            let mut count = 0u64;
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    let i = (y * width + x) * channels;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += pixels[i + if channels == 3 { c } else { 0 }] as u64;
                    }
                    count += 1;
                }
            }
            let o = (cy * SIDE + cx) * 3;
            for c in 0..3 {
                out[o + c] = (sum[c] / count.max(1)) as u8;
            }
        }
    }
    Some(out)
}

/// Average color of the source pixels behind cell (`gx`, `gy`) of a
/// `grid_w` x `grid_h` grid laid over the cover.
fn sample(cover: &[u8], gx: usize, gy: usize, grid_w: usize, grid_h: usize) -> [u8; 3] {
    let (x0, x1) = (gx * SIDE / grid_w, ((gx + 1) * SIDE / grid_w).max(gx * SIDE / grid_w + 1));
    let (y0, y1) = (gy * SIDE / grid_h, ((gy + 1) * SIDE / grid_h).max(gy * SIDE / grid_h + 1));
    let mut sum = [0u32; 3];
    let mut count = 0u32;
    for y in y0..y1.min(SIDE) {
        for x in x0..x1.min(SIDE) {
            let i = (y * SIDE + x) * 3;
            for c in 0..3 {
                sum[c] += cover[i + c] as u32;
            }
            count += 1;
        }
    }
    let count = count.max(1);
    [(sum[0] / count) as u8, (sum[1] / count) as u8, (sum[2] / count) as u8]
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3)
        .map(|c| (a[c] as i32 - b[c] as i32).pow(2) as u32)
        .sum()
}

/// Quadrant glyphs indexed by which pixels take the foreground color:
/// bit 0 top-left, bit 1 top-right, bit 2 bottom-left, bit 3 bottom-right.
const QUADRANTS: [&str; 16] = [
    " ", "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

/// Draws the cover in `rect`. Every cell shows 2x2 pixels with a quadrant
/// glyph and two colors, so an 8x4 cell box holds a 16x8 pixel picture.
pub fn draw(buf: &mut ratatui::buffer::Buffer, rect: ratatui::layout::Rect, cover: &[u8]) {
    use ratatui::style::Color;
    if cover.len() != SIDE * SIDE * 3 {
        return;
    }
    let (grid_w, grid_h) = (rect.width as usize * 2, rect.height as usize * 2);
    for row in 0..rect.height as usize {
        for col in 0..rect.width as usize {
            let pixels = [
                sample(cover, col * 2, row * 2, grid_w, grid_h),
                sample(cover, col * 2 + 1, row * 2, grid_w, grid_h),
                sample(cover, col * 2, row * 2 + 1, grid_w, grid_h),
                sample(cover, col * 2 + 1, row * 2 + 1, grid_w, grid_h),
            ];
            // The two most different pixels seed the two colors of the cell.
            let (mut a, mut b, mut far) = (0, 1, 0);
            for i in 0..4 {
                for j in i + 1..4 {
                    let d = distance(pixels[i], pixels[j]);
                    if d > far {
                        (a, b, far) = (i, j, d);
                    }
                }
            }
            let mut mask = 0;
            let mut sums = [[0u32; 3]; 2];
            let mut counts = [0u32; 2];
            for (i, pixel) in pixels.iter().enumerate() {
                let group = usize::from(distance(*pixel, pixels[a]) > distance(*pixel, pixels[b]));
                if group == 0 {
                    mask |= 1 << i;
                }
                for c in 0..3 {
                    sums[group][c] += pixel[c] as u32;
                }
                counts[group] += 1;
            }
            let color = |group: usize| {
                let n = counts[group].max(1);
                Color::Rgb(
                    (sums[group][0] / n) as u8,
                    (sums[group][1] / n) as u8,
                    (sums[group][2] / n) as u8,
                )
            };
            let background = if counts[1] == 0 { color(0) } else { color(1) };
            buf.get_mut(rect.x + col as u16, rect.y + row as u16)
                .set_symbol(QUADRANTS[mask])
                .set_fg(color(0))
                .set_bg(background);
        }
    }
}
