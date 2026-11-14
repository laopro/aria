//! Round trip on the signed-in account, leaving it as it was:
//! `account_probe <headers.txt> <video id>` creates a playlist, adds the track,
//! removes it, deletes the playlist, then likes and unlikes the track.
use std::path::PathBuf;

use ytpapi2::YoutubeMusicInstance;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let headers = PathBuf::from(args.next().expect("headers path"));
    let video = args.next().expect("video id");
    let api = YoutubeMusicInstance::from_header_file(&headers)
        .await
        .expect("login required");

    let id = api.create_playlist("Aria probe (safe to delete)").await;
    println!("create: {id:?}");
    if let Ok(id) = id {
        println!("add: {:?}", api.add_to_playlist(&id, &video).await);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        for pages in [0, 1, 50] {
            let tracks = api.get_playlist_raw(&id, pages).await.unwrap_or_default();
            let titles: Vec<&str> = tracks.iter().map(|v| v.title.as_str()).take(4).collect();
            println!("tracks with {pages} pages: {} {titles:?}", tracks.len());
        }
        println!("remove: {:?}", api.remove_from_playlist(&id, &video).await);
        println!("delete: {:?}", api.delete_playlist(&id).await);
    }
    if args.next().as_deref() == Some("like") {
        // Only on a track that is not liked, so the account ends up unchanged.
        let liked = api.get_playlist_raw("VLLM", 50).await.unwrap_or_default();
        if liked.iter().any(|v| v.video_id == video) {
            println!("like: skipped, the track is already liked");
            return;
        }
        println!("like: {:?}", api.set_liked(&video, true).await);
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let now = api.get_playlist_raw("VLLM", 1).await.unwrap_or_default();
        println!("in liked now: {}", now.iter().any(|v| v.video_id == video));
        println!("unlike: {:?}", api.set_liked(&video, false).await);
    }
}
