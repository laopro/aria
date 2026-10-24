use std::path::PathBuf;

use ytpapi2::{Endpoint, YoutubeMusicInstance};

#[tokio::main]
async fn main() {
    let headers = PathBuf::from(std::env::var("APPDATA").unwrap())
        .join("ccgauche")
        .join("ytermusic")
        .join("config")
        .join("headers.txt");
    let api = YoutubeMusicInstance::from_header_file(&headers)
        .await
        .expect("login required");

    match api.get_library(&Endpoint::MusicLibraryLanding, 1).await {
        Ok(list) => {
            println!("library: {}", list.len());
            for p in list.iter().take(10) {
                println!("- {} [{}]", p.name, p.browse_id);
            }
        }
        Err(e) => println!("library error: {e:?}"),
    }

    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "VLPLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI".to_string());
    match api.get_playlist_named(&id, 1).await {
        Ok((name, v)) => {
            println!("playlist name: {} ({} videos)", name, v.len());
            for x in v.iter().take(5) {
                println!("  {} | {} | {}", x.title, x.author, x.duration);
            }
        }
        Err(e) => println!("playlist error: {e:?}"),
    }
}
