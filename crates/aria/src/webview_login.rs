//! Login window embedded in the app (WebView2 on Windows).
//!
//! It opens YouTube Music in a real window, waits until the user signs in with
//! their Google account and stores the session. The user never has to copy
//! anything and never has to close their own browser.
//!
//! The window runs in its own process (`aria --login-window`) because the
//! platform event loop never returns once started.

use std::process;
use std::time::{Duration, Instant};

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Cookies that the YouTube Music API needs to consider the user signed in.
const LOGIN_COOKIES: [&str; 3] = ["SAPISID", "__Secure-3PSID", "__Secure-1PSID"];

fn wanted_domain(domain: &str) -> bool {
    domain.contains("youtube.com") || domain.contains("google.com")
}

/// Opens the login window and exits the process when done: `0` if the session
/// was stored, `1` if the window was closed before signing in.
pub fn run_login_window() -> ! {
    let event_loop = EventLoop::new();
    let window = match WindowBuilder::new()
        .with_title("Aria - Sign in")
        .with_inner_size(LogicalSize::new(520.0, 780.0))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(_) => process::exit(1),
    };

    let webview = match WebViewBuilder::new()
        .with_url(
            "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F",
        )
        .build(&window)
    {
        Ok(webview) => webview,
        Err(_) => process::exit(1),
    };

    let mut last_check = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500));

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => process::exit(1),
            Event::MainEventsCleared => {
                if last_check.elapsed() < Duration::from_millis(500) {
                    return;
                }
                last_check = Instant::now();

                let Ok(cookies) = webview.cookies() else {
                    return;
                };

                let signed_in = cookies
                    .iter()
                    .any(|cookie| LOGIN_COOKIES.contains(&cookie.name()));

                if !signed_in {
                    return;
                }

                let mut seen: Vec<String> = Vec::new();
                let mut parts: Vec<String> = Vec::new();
                for cookie in &cookies {
                    if !wanted_domain(cookie.domain().unwrap_or_default()) {
                        continue;
                    }
                    let name = cookie.name().to_string();
                    if seen.contains(&name) {
                        continue;
                    }
                    seen.push(name.clone());
                    parts.push(format!("{name}={}", cookie.value()));
                }

                if parts.is_empty() {
                    return;
                }

                crate::save_session(&parts.join("; "), USER_AGENT);
                process::exit(0);
            }
            _ => {}
        }
    })
}
