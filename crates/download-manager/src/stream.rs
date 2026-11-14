//! Minimal streaming helper: resolve a direct audio URL with yt-dlp and fetch
//! it into memory, skipping the on-disk download step entirely.

use std::{
    collections::HashMap,
    io::Read,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    time::{Duration, Instant},
};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const FORMAT: &str = "140/bestaudio[ext=m4a]/bestaudio[ext=mp4]/bestaudio";

/// YouTube throttles a plain download to about playback speed (~40 KB/s), but
/// serves explicit `range` requests at full speed. Chunks stay under 10 MB,
/// the size above which ranges get throttled as well.
const CHUNK: u64 = 8 * 1024 * 1024;

/// Resolved stream URLs stay valid for hours; replays within this window skip yt-dlp.
const URL_TTL: Duration = Duration::from_secs(60 * 60);

static URL_CACHE: Mutex<Option<HashMap<String, (String, Instant)>>> = Mutex::new(None);

/// Tracks being resolved right now, so two threads never run yt-dlp for the same one.
static IN_FLIGHT: Mutex<Vec<String>> = Mutex::new(Vec::new());
static RESOLVED: Condvar = Condvar::new();

/// The track the user is most likely to play next, waiting to be resolved ahead of time.
static WANTED: Mutex<Option<String>> = Mutex::new(None);
static PREWARMING: AtomicBool = AtomicBool::new(false);

/// `lean` skips fetches the audio URL does not need; the full run is the retry path.
fn run_ytdlp(video_id: &str, lean: bool) -> Result<String, String> {
    let watch = format!("https://www.youtube.com/watch?v={video_id}");
    let mut command = Command::new("yt-dlp");
    command.args(["--no-playlist", "-f", FORMAT, "-g", "--no-warnings"]);
    if lean {
        // (Skipping the watch page too gets the request flagged as a bot.)
        command.args([
            "--extractor-args",
            "youtube:skip=hls,translated_subs;player_skip=configs",
        ]);
    }
    let out = command
        .arg(&watch)
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

fn remember_url(video_id: &str, url: &str) {
    let mut cache = URL_CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    cache.retain(|_, (_, at)| at.elapsed() < URL_TTL);
    cache.insert(video_id.to_string(), (url.to_string(), Instant::now()));
}

fn forget_url(video_id: &str) {
    if let Some(cache) = URL_CACHE.lock().unwrap().as_mut() {
        cache.remove(video_id);
    }
}

fn cached_url(video_id: &str) -> Option<String> {
    let cache = URL_CACHE.lock().unwrap();
    let (url, at) = cache.as_ref()?.get(video_id)?;
    (at.elapsed() < URL_TTL).then(|| url.clone())
}

fn resolve_url(video_id: &str) -> Result<String, String> {
    loop {
        if let Some(url) = cached_url(video_id) {
            return Ok(url);
        }
        let flying = IN_FLIGHT.lock().unwrap();
        if !flying.iter().any(|id| id == video_id) {
            let mut flying = flying;
            flying.push(video_id.to_string());
            break;
        }
        // Another thread is already resolving this track: wait for its result.
        let _ = RESOLVED
            .wait_timeout(flying, Duration::from_secs(20))
            .unwrap();
    }

    let result = run_ytdlp(video_id, true);
    if let Ok(url) = &result {
        remember_url(video_id, url);
    }
    IN_FLIGHT.lock().unwrap().retain(|id| id != video_id);
    RESOLVED.notify_all();
    result
}

/// Resolves the stream URL of `video_id` ahead of time, so pressing play only
/// has to download the audio. Call it for the row the user rests on: only the
/// latest request is kept, and one yt-dlp runs at a time.
pub fn prewarm(video_id: &str) {
    if cached_url(video_id).is_some() {
        return;
    }
    *WANTED.lock().unwrap() = Some(video_id.to_string());
    if PREWARMING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        // Wait until the selection has rested on the same row for a moment.
        let mut target = WANTED.lock().unwrap().clone();
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let now = WANTED.lock().unwrap().clone();
            if now == target {
                break;
            }
            target = now;
        }
        if let Some(id) = &target {
            let _ = resolve_url(id);
        }
        let mut wanted = WANTED.lock().unwrap();
        if *wanted == target {
            *wanted = None;
            PREWARMING.store(false, Ordering::SeqCst);
            return;
        }
    });
}

fn get(url: &str) -> Result<ureq::Response, String> {
    ureq::get(url)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("audio request failed: {e}"))
}

/// Total size in bytes, taken from the `clen` parameter of a stream URL.
fn content_length(url: &str) -> Option<u64> {
    let rest = url.split("clen=").nth(1)?;
    rest.split('&').next()?.parse().ok().filter(|n| *n > 0)
}

/// Downloads `url` in `range` chunks, which YouTube does not throttle.
fn fetch_ranged(url: &str) -> Result<Vec<u8>, String> {
    let total = content_length(url);
    let mut buf = Vec::with_capacity(total.unwrap_or(4 * 1024 * 1024) as usize);
    let mut start = 0u64;
    loop {
        let end = match total {
            Some(total) => (start + CHUNK).min(total) - 1,
            None => start + CHUNK - 1,
        };
        let before = buf.len();
        match get(&format!("{url}&range={start}-{end}")) {
            Ok(resp) => {
                resp.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("could not read audio body: {e}"))?;
            }
            // Without a known size, a failed request past the first chunk means the end.
            Err(_) if total.is_none() && start > 0 => break,
            Err(e) => return Err(e),
        }
        let got = (buf.len() - before) as u64;
        start += got;
        let done = match total {
            Some(total) => start >= total,
            None => got < CHUNK,
        };
        if done || got == 0 {
            break;
        }
    }
    Ok(buf)
}

/// Downloads the whole audio behind `url`.
fn download(url: &str) -> Result<Vec<u8>, String> {
    let buf = match fetch_ranged(url) {
        Ok(buf) if !buf.is_empty() => buf,
        // Fall back to one plain request: slow, but it works on any URL shape.
        _ => {
            let mut buf = Vec::new();
            get(url)?
                .into_reader()
                .read_to_end(&mut buf)
                .map_err(|e| format!("could not read audio body: {e}"))?;
            buf
        }
    };
    if buf.is_empty() {
        return Err("empty audio body".to_string());
    }
    Ok(buf)
}

/// Fetches the audio for `video_id` into RAM. Never touches the disk.
pub fn stream_bytes(video_id: &str) -> Result<Vec<u8>, String> {
    let url = resolve_url(video_id)?;
    if let Ok(buf) = download(&url) {
        return Ok(buf);
    }
    // YouTube sometimes rejects a URL (403) or a cached one went stale:
    // resolve a new one the long way and try once more.
    forget_url(video_id);
    let url = run_ytdlp(video_id, false)?;
    let buf = download(&url)?;
    remember_url(video_id, &url);
    Ok(buf)
}
