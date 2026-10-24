use consts::{CACHE_DIR, CONFIG, INTRODUCTION};
use flume::{Receiver, Sender};
use log::{error, info};
use once_cell::sync::Lazy;
use structures::performance::STARTUP_TIME;
use term::{Manager, ManagerMessage};
use tokio::select;

use std::{
    future::Future,
    panic,
    path::{Path, PathBuf},
    str::FromStr,
    sync::RwLock,
};
use systems::{logger::init, player::player_system};

use crate::{
    structures::{media::run_window_handler, sound_action::download_manager_handler},
    systems::{logger::get_log_file_path, DOWNLOAD_MANAGER},
    utils::get_project_dirs,
};

mod config;
mod consts;
mod database;
mod errors;
mod shutdown;
mod structures;
mod systems;
mod term;
mod utils;
#[cfg(target_os = "windows")]
mod webview_login;
pub use shutdown::{is_shutdown_sent, shutdown, ShutdownSignal};
mod tasks;

pub use database::DATABASE;

use mimalloc::MiMalloc;

// Changes the allocator to improve performance especially on Windows
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn run_service<T>(future: T) -> tokio::task::JoinHandle<()>
where
    T: Future + Send + 'static,
{
    tokio::task::spawn(async move {
        select! {
            _ = future => {},
            _ = ShutdownSignal => {},
        }
    })
}

static COOKIES: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub fn try_get_cookies() -> Option<String> {
    let cookies = COOKIES.read().unwrap();
    cookies.clone()
}

fn main() {
    // Check if the first param is --files
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", INTRODUCTION);
                return;
            }
            "--files" => {
                println!("# Location of Aria files");
                println!(" - Logs: {}", get_log_file_path().display());
                let headers = match get_header_file() {
                    Ok((_, p)) | Err((_, p)) => p,
                };
                println!(" - Headers: {}", headers.display());
                println!(" - Cache: {}", CACHE_DIR.display());
                return;
            }
            "--fix-db" => {
                DATABASE.fix_db();
                DATABASE.write();
                println!("[INFO] Database fixed");
                return;
            }
            "--clear-cache" => {
                match std::fs::remove_dir_all(&*CACHE_DIR) {
                    Ok(_) => {
                        println!("[INFO] Cache cleared");
                    }
                    Err(e) => {
                        println!("[ERROR] Can't clear cache: {e}");
                    }
                }
                return;
            }
            "--with-auto-cookies" => {
                std::fs::write(get_log_file_path(), "# Aria log file\n\n").unwrap();
                init().expect("Failed to initialize logger");
                let param = std::env::args().nth(2);
                if let Some(cookies) = cookies(param) {
                    let mut cookies_guard = COOKIES.write().unwrap();
                    info!("Cookies: {cookies}");
                    *cookies_guard = Some(cookies);
                    info!("Cookies loaded");
                } else {
                    error!("Can't load cookies");
                    error!("Maybe rookie didn't find any cookies or any browser");
                    error!("Please make sure you have cookies in your browser");
                    return;
                }
            }
            "--login-window" => {
                #[cfg(target_os = "windows")]
                webview_login::run_login_window();
                #[cfg(not(target_os = "windows"))]
                {
                    println!("The login window is only available on Windows.");
                    std::process::exit(1);
                }
            }
            "--login" => {
                std::fs::write(get_log_file_path(), "# Aria log file\n\n").unwrap();
                init().expect("Failed to initialize logger");
                login(std::env::args().nth(2));
                return;
            }
            e => {
                println!("Unknown argument `{e}`");
                println!("Here are the available arguments:");
                println!(" - --files: Show the location of the Aria files");
                println!(" - --clear-cache: Erase all the files in cache");
                println!(" - --fix-db: Fix the database");
                return;
            }
        }
    } else {
        std::fs::write(get_log_file_path(), "# Aria log file\n\n").unwrap();
        init().expect("Failed to initialize logger");
    }
    panic::set_hook(Box::new(|e| {
        println!("{e}");
        error!("{e}");
        shutdown();
    }));
    app_start();
}

fn cookies(specific_browser: Option<String>) -> Option<String> {
    if let Some(cookies) = cookies_from_rookie(specific_browser.clone()) {
        if !cookies.is_empty() {
            return Some(cookies);
        }
    }

    // Fallback: yt-dlp understands Chromium forks (Thorium, Brave...) and
    // custom profile paths, which `rookie` does not.
    for browser in ytdlp_browsers(specific_browser.as_deref()) {
        if let Some(cookies) = cookies_from_ytdlp(&browser) {
            return Some(cookies);
        }
    }

    None
}

fn cookies_from_rookie(specific_browser: Option<String>) -> Option<String> {
    let loaded = match specific_browser {
        Some(browser) => match browser.as_str() {
            "all" => rookie::load,
            "firefox" => rookie::firefox,
            "chrome" => rookie::chrome,
            "edge" => rookie::edge,
            "opera" => rookie::opera,
            "brave" => rookie::brave,
            "vivaldi" => rookie::vivaldi,
            "chromium" => rookie::chromium,
            #[cfg(target_os = "macos")]
            "safari" => rookie::safari,
            "arc" => rookie::arc,
            "librewolf" => rookie::librewolf,
            "opera-gx" | "opera_gx" => rookie::opera_gx,
            #[cfg(target_os = "windows")]
            "internet_explorer" | "internet-explorer" | "ie" => rookie::internet_explorer,
            #[cfg(target_os = "windows")]
            "octo_browser" | "octo-browser" => rookie::octo_browser,
            _ => {
                println!("Unknown browser `{browser}`");
                error!("Unknown browser `{browser}`");
                return None;
            }
        },
        None => rookie::load,
    }(Some(vec!["youtube.com".to_string()]));

    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("[ERROR] Can't read browser cookies: {e}");
            return None;
        }
    };
    let mut cookies = Vec::new();
    let current_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for cookie in loaded {
        if cookie.domain != ".youtube.com" && cookie.domain != "music.youtube.com" {
            continue;
        }
        if cookie
            .expires
            .map(|e| e < current_timestamp)
            .unwrap_or(false)
        {
            continue;
        }
        if cookies.iter().any(|(name, _)| name == &cookie.name) {
            continue;
        }
        cookies.push((cookie.name, cookie.value));
    }
    let cookies = cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>();
    let cookies = cookies.join("; ");
    Some(cookies)
}

/// Browsers yt-dlp should try, in order. Chromium forks are passed by profile
/// path because they are not detected by name.
fn ytdlp_browsers(requested: Option<&str>) -> Vec<String> {
    let mut browsers = Vec::new();
    match requested {
        Some("thorium") => {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                browsers.push(format!("chrome:{local}\\Thorium\\User Data\\Default"));
            }
        }
        Some(browser) if browser != "all" => browsers.push(browser.to_string()),
        _ => {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let thorium = format!("{local}\\Thorium\\User Data\\Default");
                if std::path::Path::new(&thorium).exists() {
                    browsers.push(format!("chrome:{thorium}"));
                }
            }
            for browser in ["edge", "chrome", "firefox"] {
                browsers.push(browser.to_string());
            }
        }
    }
    browsers
}

/// Asks yt-dlp for the browser cookie jar and keeps the youtube.com entries.
fn cookies_from_ytdlp(browser: &str) -> Option<String> {
    let jar = std::env::temp_dir().join("aria-cookies.txt");
    let _ = std::fs::remove_file(&jar);

    let output = std::process::Command::new("yt-dlp")
        .args([
            "--cookies-from-browser",
            browser,
            "--cookies",
            jar.to_str()?,
            "--skip-download",
            "--no-warnings",
            "--quiet",
            "https://www.youtube.com/watch?v=BaW_jenozKc",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        info!("yt-dlp could not read cookies from {browser}");
        let _ = std::fs::remove_file(&jar);
        return None;
    }

    let text = std::fs::read_to_string(&jar).ok()?;
    let _ = std::fs::remove_file(&jar);

    let mut seen: Vec<String> = Vec::new();
    let mut cookies: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 || !fields[0].contains("youtube.com") {
            continue;
        }
        let name = fields[5];
        if seen.iter().any(|n| n == name) {
            continue;
        }
        seen.push(name.to_string());
        cookies.push(format!("{name}={}", fields[6]));
    }

    if cookies.is_empty() {
        return None;
    }
    Some(cookies.join("; "))
}

/// Opens the login window, waits for the user to sign in with their Google
/// account and stores the resulting session in `headers.txt`.
/// On Windows this is a WebView2 window owned by the app.
fn login(specific_browser: Option<String>) {
    const UA: &str = DEFAULT_USER_AGENT;

    let path = match get_header_file() {
        Ok((_, p)) | Err((_, p)) => p,
    };

    #[cfg(target_os = "windows")]
    {
        println!("Opening the sign-in window...");
        println!("Sign in with your Google account; the window closes by itself.");
        if let Ok(exe) = std::env::current_exe() {
            let status = std::process::Command::new(exe)
                .arg("--login-window")
                .status();
            if status.map(|s| s.success()).unwrap_or(false) {
                println!("[OK] Session saved.");
                return;
            }
        }
        println!("Could not sign in through the window. Trying your browser...");
        println!();
    }

    println!("Opening https://music.youtube.com ...");
    open_browser("https://music.youtube.com");

    println!();
    println!("1. Sign in (if you are not already).");
    println!("2. Close the browser, so its cookie database is unlocked.");
    println!("3. Come back here and press Enter.");
    println!();
    println!("Tip: close your browser first (Thorium, Chrome, Edge lock the cookie database).");
    println!("You can also pick one: aria --login thorium");
    println!();

    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    let Some(cookie) = cookies(specific_browser) else {
        eprintln!("[ERROR] Could not read cookies. Are you signed in?");
        return;
    };

    if cookie.is_empty() {
        eprintln!("[ERROR] The cookie jar is empty. Are you signed in?");
        return;
    }

    write_headers(&path, &cookie, UA);
}

/// Stores the session in `headers.txt`, backing up the previous one.
pub fn save_session(cookie: &str, user_agent: &str) {
    let path = match get_header_file() {
        Ok((_, p)) | Err((_, p)) => p,
    };
    write_headers(&path, cookie, user_agent);
}

/// Stores the session in `headers.txt`, backing up the previous one.
fn write_headers(path: &Path, cookie: &str, user_agent: &str) -> bool {
    if path.exists() {
        let backup = path.with_extension("txt.bak");
        let _ = std::fs::copy(path, &backup);
        println!("[INFO] Previous session backed up to {}", backup.display());
    }

    let content = format!("Cookie: {cookie}\nUser-Agent: {user_agent}\n");
    match std::fs::write(path, content) {
        Ok(_) => {
            println!("[OK] Session saved to {}", path.display());
            true
        }
        Err(e) => {
            eprintln!("[ERROR] Can't write {}: {e}", path.display());
            false
        }
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

fn get_header_file() -> Result<(String, PathBuf), (std::io::Error, PathBuf)> {
    let fp = PathBuf::from_str("headers.txt").unwrap();
    if let Ok(e) = std::fs::read_to_string(&fp) {
        return Ok((e, fp));
    }
    let fp = get_project_dirs()
        .ok_or_else(|| {
            (
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Can't find project dir. This is a `directories` crate issue",
                ),
                Path::new("./").to_owned(),
            )
        })?
        .config_dir()
        .to_owned();
    if let Err(e) = std::fs::create_dir_all(&fp) {
        println!("Can't create app directory {e} in `{}`", fp.display());
    }
    let fp = fp.join("headers.txt");
    if let Ok(content) = std::fs::read_to_string(&fp) {
        return Ok((content, fp));
    }
    // Legacy location used by YTerMusic, kept so existing sessions keep working.
    if let Some(old) = directories::ProjectDirs::from("com", "ccgauche", "ytermusic") {
        let old = old.config_dir().join("headers.txt");
        if let Ok(content) = std::fs::read_to_string(&old) {
            return Ok((content, old));
        }
    }
    Err((std::io::Error::new(std::io::ErrorKind::NotFound, "no session"), fp))
}

/// True when there is no Google session and the app runs anonymously.
pub fn is_guest() -> bool {
    try_get_cookies().is_none() && get_header_file().is_err()
}

/// Builds the API client: saved session first, anonymous InnerTube otherwise.
pub async fn api_instance() -> Option<std::sync::Arc<ytpapi2::YoutubeMusicInstance>> {
    use std::sync::Arc;
    if let Some(cookies) = try_get_cookies() {
        let mut headermap = ytpapi2::HeaderMap::new();
        if let Ok(value) = ytpapi2::HeaderValue::from_str(&cookies) {
            headermap.insert("cookie", value);
        }
        headermap.insert(
            "user-agent",
            ytpapi2::HeaderValue::from_static(DEFAULT_USER_AGENT),
        );
        if let Ok(api) = ytpapi2::YoutubeMusicInstance::new(headermap, None).await {
            return Some(Arc::new(api));
        }
    }
    if let Ok((_, path)) = get_header_file() {
        if let Ok(api) = ytpapi2::YoutubeMusicInstance::from_header_file(&path).await {
            return Some(Arc::new(api));
        }
    }
    ytpapi2::YoutubeMusicInstance::anonymous()
        .await
        .ok()
        .map(Arc::new)
}

async fn app_start_main(updater_r: Receiver<ManagerMessage>, updater_s: Sender<ManagerMessage>) {
    STARTUP_TIME.log("Init");

    std::fs::create_dir_all(CACHE_DIR.join("downloads")).unwrap();

    if CONFIG.global.downloader == crate::config::DownloaderConfig::Ytdlp {
        match std::process::Command::new("yt-dlp")
            .arg("--version")
            .output()
        {
            Ok(out) if out.status.success() => {
                info!(
                    "yt-dlp found: {}",
                    String::from_utf8_lossy(&out.stdout).trim()
                );
            }
            _ => {
                println!("yt-dlp not found in PATH");
                println!("get it at https://github.com/yt-dlp/yt-dlp or set downloader = \"rusty_ytdl\" in config");
                std::io::stdin().read_line(&mut String::new()).unwrap();
                return;
            }
        }
    }

    if try_get_cookies().is_none() && get_header_file().is_err() {
        info!("No session found: starting in guest mode");
    }

    STARTUP_TIME.log("Startup");

    // Spawn the clean task
    tasks::clean::spawn_clean_task();

    STARTUP_TIME.log("Spawned clean task");
    // Spawn the player task
    let (sa, player) = player_system(updater_s.clone());
    // Spawn the downloader system
    DOWNLOAD_MANAGER.spawn_system(ShutdownSignal, download_manager_handler(sa.clone()));
    STARTUP_TIME.log("Spawned system task");
    tasks::last_playlist::spawn_last_playlist_task(updater_s.clone());
    STARTUP_TIME.log("Spawned last playlist task");
    // Spawn the API task
    tasks::api::spawn_api_task(updater_s.clone());
    STARTUP_TIME.log("Spawned api task");
    // Spawn the database getter task
    tasks::local_musics::spawn_local_musics_task(updater_s);

    STARTUP_TIME.log("Running manager");
    let mut manager = Manager::new(sa, player).await;
    manager.run(&updater_r).unwrap();
}

fn app_start() {
    let (updater_s, updater_r) = flume::unbounded::<ManagerMessage>();
    let updater_s_c = updater_s.clone();
    ctrlc::set_handler(move || {
        info!("CTRL-C received");
        shutdown()
    })
    .expect("Error setting Ctrl-C handler");
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to build runtime")
            .block_on(async move {
                select! {
                    _ = app_start_main(updater_r, updater_s) => {},
                    _ = ShutdownSignal => {},
                };
            });
        info!("Runtime closed");
    });
    run_window_handler(&updater_s_c);
}
