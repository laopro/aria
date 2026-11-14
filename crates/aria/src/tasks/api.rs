use std::sync::{Arc, Mutex};

use flume::Sender;
use log::{error, info};
use once_cell::sync::Lazy;
use tokio::task::JoinSet;
use ytpapi2::{Endpoint, YoutubeMusicInstance, YoutubeMusicPlaylistRef};

use crate::{
    consts::CONFIG,
    get_header_file, run_service,
    structures::performance,
    term::{ManagerMessage, Screens},
};

pub fn get_text_cookies_expired_or_invalid() -> String {
    let (Ok((_, path)) | Err((_, path))) = get_header_file();
    format!(
        "The `{}` file is not configured correctly. \nThe cookies are expired or invalid.",
        path.display()
    )
}

pub fn spawn_api_task(updater_s: Sender<ManagerMessage>) {
    run_service(async move {
        info!("API task on");
        if crate::is_guest() {
            info!("Guest mode: skipping library sync");
            return;
        }
        let guard = performance::guard("API task");
        let client =
            YoutubeMusicInstance::from_header_file(get_header_file().unwrap().1.as_path()).await;
        match client {
            Ok(api) => {
                let api = Arc::new(api);
                let mut set = JoinSet::new();
                // Listening history, synced like the playlists below.
                // (Liked Music already comes with the library.)
                spawn_browse_playlist_task(
                    YoutubeMusicPlaylistRef {
                        name: "Historial".to_owned(),
                        subtitle: "Reproducido recientemente".to_owned(),
                        browse_id: "FEmusic_history".to_owned(),
                    },
                    api.clone(),
                    updater_s.clone(),
                );
                let api_ = api.clone();
                let updater_s_ = updater_s.clone();
                set.spawn(async move {
                    let search_results = api_.get_home(1).await;
                    match search_results {
                        Ok(e) => {
                            for playlist in e.playlists {
                                spawn_browse_playlist_task(
                                    playlist.clone(),
                                    api_.clone(),
                                    updater_s_.clone(),
                                )
                            }
                        }
                        Err(e) => {
                            error!("get_home {e:?}")
                        }
                    }
                });
                let api_ = api.clone();
                let updater_s_ = updater_s.clone();
                set.spawn(async move {
                    let search_results = api_.get_library(&Endpoint::MusicLikedPlaylists, 1).await;
                    match search_results {
                        Ok(e) if e.is_empty() => {
                            crate::refresh_session(updater_s_.clone());
                        }
                        Ok(e) => {
                            for playlist in e {
                                spawn_browse_playlist_task(
                                    playlist.clone(),
                                    api_.clone(),
                                    updater_s_.clone(),
                                )
                            }
                        }
                        Err(e) => {
                            error!("MusicLikedPlaylists -> {e:?}");
                        }
                    }
                });
                let api_ = api.clone();
                let updater_s_ = updater_s.clone();
                set.spawn(async move {
                    let search_results = api_.get_library(&Endpoint::MusicLibraryLanding, 1).await;
                    match search_results {
                        Ok(e) => {
                            for playlist in e {
                                spawn_browse_playlist_task(
                                    playlist.clone(),
                                    api_.clone(),
                                    updater_s_.clone(),
                                )
                            }
                        }
                        Err(e) => {
                            error!("MusicLibraryLanding -> {e:?}");
                        }
                    }
                });
                while let Some(e) = set.join_next().await {
                    e.unwrap();
                }
            }
            Err(e) => match &e {
                ytpapi2::YoutubeMusicError::NoCookieAttribute
                | ytpapi2::YoutubeMusicError::NoSapsidInCookie
                | ytpapi2::YoutubeMusicError::InvalidCookie(_)
                | ytpapi2::YoutubeMusicError::NeedToLogin
                | ytpapi2::YoutubeMusicError::CantFindInnerTubeApiKey(_)
                | ytpapi2::YoutubeMusicError::CantFindInnerTubeClientVersion(_)
                | ytpapi2::YoutubeMusicError::CantFindVisitorData(_)
                | ytpapi2::YoutubeMusicError::IoError(_) => {
                    error!("{}", get_text_cookies_expired_or_invalid());
                    error!("{e:?}");
                    updater_s
                        .send(
                            ManagerMessage::Error(
                                get_text_cookies_expired_or_invalid(),
                                Box::new(Some(ManagerMessage::Quit)),
                            )
                            .pass_to(Screens::DeviceLost),
                        )
                        .unwrap();
                }
                e => {
                    error!("{e:?}");
                }
            },
        }
        drop(guard);
    });
}

/// YouTube starts answering with empty pages when dozens of playlists are
/// fetched at once, so only a few load at the same time.
/// Playlists still loading; Home shows placeholder rows while it is above zero.
pub static SYNCING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

static BROWSE_LIMIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

/// Playlists loaded from the account in this session: `(name, browse id)`.
pub static BROWSED_PLAYLISTS: Lazy<Mutex<Vec<(String, String)>>> =
    Lazy::new(|| Mutex::new(vec![]));

/// Loads the account library again, so changes made elsewhere show up.
pub fn resync(updater: Sender<ManagerMessage>) {
    if crate::is_guest() {
        let _ = updater.send(ManagerMessage::Toast("Sign in first (F8)".to_owned()));
        return;
    }
    if SYNCING.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        let _ = updater.send(ManagerMessage::Toast("Already syncing".to_owned()));
        return;
    }
    BROWSED_PLAYLISTS.lock().unwrap().clear();
    let _ = updater.send(ManagerMessage::Library(
        crate::account::LibraryChange::Reloading,
    ));
    let _ = updater.send(ManagerMessage::Toast("Syncing with your account…".to_owned()));
    spawn_api_task(updater);
}

fn spawn_browse_playlist_task(
    playlist: YoutubeMusicPlaylistRef,
    api: Arc<YoutubeMusicInstance>,
    updater_s: Sender<ManagerMessage>,
) {
    if playlist.browse_id.starts_with("UC") && CONFIG.player.hide_channels_on_homepage {
        log::info!(
            "Skipping channel (CONFIG) {} {}",
            playlist.name,
            playlist.browse_id
        );
        return;
    }
    if playlist.browse_id.starts_with("MPREb_") && CONFIG.player.hide_albums_on_homepage {
        log::info!(
            "Skipping album (CONFIG) {} {}",
            playlist.name,
            playlist.browse_id
        );
        return;
    }
    {
        let mut k = BROWSED_PLAYLISTS.lock().unwrap();
        if k.iter()
            .any(|(name, id)| name == &playlist.name && id == &playlist.browse_id)
        {
            return;
        }
        k.push((playlist.name.clone(), playlist.browse_id.clone()));
    }

    SYNCING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    run_service(async move {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                SYNCING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let _done = Done;
        let guard = format!("Browse playlist {} {}", playlist.name, playlist.browse_id);
        let guard = performance::guard(&guard);
        let _permit = BROWSE_LIMIT.acquire().await;
        // Up to 50 pages of ~100 tracks, so long lists like Liked Music load whole.
        match api.get_playlist(&playlist, 50).await {
            Ok(videos) => {
                if videos.len() < 2 {
                    info!("Playlist {} is too small so skipped", playlist.name);
                    return;
                }
                if playlist.browse_id == crate::account::LIKED_ID {
                    crate::account::set_liked_tracks(&videos);
                }
                let _ = updater_s.send(
                    ManagerMessage::AddPlaylist(
                        playlist.name.clone(),
                        playlist.browse_id.clone(),
                        videos,
                    )
                    .pass_to(Screens::Playlist),
                );
            }
            Err(e) => {
                error!("{e:?}");
            }
        }

        drop(guard);
    });
}
