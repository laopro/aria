//! Minimal streaming helper: resolve a direct audio URL with yt-dlp and fetch
//! it into memory, skipping the on-disk download step entirely.

use std::{io::Read, process::Command, time::Duration};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const FORMAT: &str = "140/bestaudio[ext=m4a]/bestaudio[ext=mp4]/bestaudio";

fn resolve_url(video_id: &str) -> Result<String, String> {
    let watch = format!("https://www.youtube.com/watch?v={video_id}");
    let out = Command::new("yt-dlp")
        .args(["--no-playlist", "-f", FORMAT, "-g", "--no-warnings", &watch])
        .output()
        .map_err(|e| format!("could not run yt-dlp: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "yt-dlp failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "yt-dlp returned an empty url".to_string())
}

/// Fetches the audio for `video_id` into RAM. Never touches the disk.
pub fn stream_bytes(video_id: &str) -> Result<Vec<u8>, String> {
    let url = resolve_url(video_id)?;

    let resp = ureq::get(&url)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("audio request failed: {e}"))?;

    let mut buf = Vec::with_capacity(4 * 1024 * 1024);
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("could not read audio body: {e}"))?;

    if buf.is_empty() {
        return Err("empty audio body".to_string());
    }

    Ok(buf)
}
