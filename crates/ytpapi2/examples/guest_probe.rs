use ytpapi2::YoutubeMusicInstance;

#[tokio::main]
async fn main() {
    let api = YoutubeMusicInstance::anonymous().await.expect("anonymous client");
    println!("anonymous client ok");

    match api.search("amor como estas yeyo", 1).await {
        Ok(results) => {
            println!("search: {} videos, {} playlists", results.videos.len(), results.playlists.len());
            for v in results.videos.iter().take(3) {
                println!("  {} | {} | {}", v.title, v.author, v.duration);
            }
        }
        Err(e) => println!("search error: {e:?}"),
    }

    match api
        .get_playlist_named("VLPLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI", 1)
        .await
    {
        Ok((name, videos)) => println!("playlist: {} ({} videos)", name, videos.len()),
        Err(e) => println!("playlist error: {e:?}"),
    }
}
