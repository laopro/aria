//! Prints how many tracks a playlist yields: `pages_probe <headers.txt> <browse id> [pages]`.
use std::path::PathBuf;

use ytpapi2::YoutubeMusicInstance;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let headers = PathBuf::from(args.next().expect("headers path"));
    let id = args.next().unwrap_or_else(|| "VLLM".to_string());
    let pages: usize = args.next().and_then(|p| p.parse().ok()).unwrap_or(50);
    let api = YoutubeMusicInstance::from_header_file(&headers)
        .await
        .expect("login required");
    match api.get_playlist_raw(&id, pages).await {
        Ok(videos) => println!("{id}: {} tracks", videos.len()),
        Err(e) => println!("error: {e:?}"),
    }
}
