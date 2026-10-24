use std::path::PathBuf;

use ytpapi2::YoutubeMusicInstance;

#[tokio::main]
async fn main() {
    let headers = PathBuf::from(std::env::var("APPDATA").unwrap())
        .join("ccgauche")
        .join("ytermusic")
        .join("config")
        .join("headers.txt");
    let query = std::env::args().nth(1).unwrap_or_else(|| "amor como estas yeyo".to_string());
    let api = YoutubeMusicInstance::from_header_file(&headers)
        .await
        .expect("login required");
    let results = api.search(&query, 0).await.expect("search failed");
    println!("query: {query}");
    for (i, v) in results.videos.iter().enumerate().take(15) {
        println!(
            "{i:2}. [{}] {} | {} | {} | {}",
            v.video_id, v.title, v.author, v.album, v.duration
        );
    }
    println!("playlists: {}", results.playlists.len());
}
