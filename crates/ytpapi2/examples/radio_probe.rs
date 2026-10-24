use ytpapi2::YoutubeMusicInstance;

#[tokio::main]
async fn main() {
    let api = YoutubeMusicInstance::anonymous().await.unwrap();
    match api.get_radio("dQw4w9WgXcQ").await {
        Ok(videos) => {
            println!("radio: {} videos", videos.len());
            for v in videos.iter().take(6) { println!("  {} | {} | {}", v.title, v.author, v.duration); }
        }
        Err(e) => println!("error {e:?}"),
    }
}
