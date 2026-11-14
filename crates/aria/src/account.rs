//! Changes to the signed-in YouTube Music account. Each one is sent to the
//! account first and, once accepted, mirrored in what Aria shows.

use std::{
    collections::HashSet,
    future::Future,
    sync::{Arc, RwLock},
};

use flume::Sender;
use log::error;
use once_cell::sync::Lazy;
use ytpapi2::{YoutubeMusicError, YoutubeMusicInstance, YoutubeMusicVideoRef};

use crate::{
    run_service,
    tasks::api::BROWSED_PLAYLISTS,
    term::ManagerMessage,
};

/// Browse id of the account's "Liked Music" list.
pub const LIKED_ID: &str = "VLLM";

/// Ids of the liked tracks, known once Liked Music has loaded.
static LIKED: Lazy<RwLock<HashSet<String>>> = Lazy::new(Default::default);

pub fn is_liked(video_id: &str) -> bool {
    LIKED.read().unwrap().contains(video_id)
}

pub fn set_liked_tracks(videos: &[YoutubeMusicVideoRef]) {
    *LIKED.write().unwrap() = videos.iter().map(|v| v.video_id.clone()).collect();
}

/// A change already made on the account, to mirror in the lists on screen.
#[derive(Debug, Clone)]
pub enum LibraryChange {
    Added {
        playlist: String,
        video: YoutubeMusicVideoRef,
    },
    Removed {
        playlist: String,
        video_id: String,
    },
    Created {
        name: String,
        browse_id: String,
    },
    Deleted {
        browse_id: String,
    },
    /// The whole library is loading again: drop what came from the account.
    Reloading,
    /// A playlist was fetched again from the account: its current tracks.
    Refreshed {
        playlist: String,
        videos: Vec<YoutubeMusicVideoRef>,
    },
}

/// Playlists a track can be added to: `(name, browse id)`.
pub fn playlists() -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = BROWSED_PLAYLISTS
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, id)| id.starts_with("VLPL"))
        .cloned()
        .collect();
    list.sort_by_key(|(name, _)| name.to_lowercase());
    list
}

fn toast(updater: &Sender<ManagerMessage>, text: impl Into<String>) {
    let _ = updater.send(ManagerMessage::Toast(text.into()));
}

/// Runs `change` against the account; on success shows `done` and mirrors `mirror`.
fn run<F, Fut>(updater: Sender<ManagerMessage>, done: String, mirror: LibraryChange, change: F)
where
    F: FnOnce(Arc<YoutubeMusicInstance>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<bool, YoutubeMusicError>> + Send,
{
    if crate::is_guest() {
        toast(&updater, "Sign in first (F8)");
        return;
    }
    run_service(async move {
        let Some(api) = crate::api_instance().await else {
            toast(&updater, "No connection to YouTube Music");
            return;
        };
        match change(api).await {
            Ok(true) => {
                let _ = updater.send(ManagerMessage::Library(mirror));
                toast(&updater, done);
            }
            Ok(false) => toast(&updater, "Nothing to change"),
            Err(e) => {
                error!("account change failed: {e:?}");
                toast(&updater, "YouTube Music rejected the change");
            }
        }
    });
}

/// Fetches an account playlist again, so changes made in the browser or on the
/// phone show up when it is opened.
pub fn refresh_playlist(updater: Sender<ManagerMessage>, browse_id: String) {
    if crate::is_guest() || browse_id.is_empty() {
        return;
    }
    run_service(async move {
        let Some(api) = crate::api_instance().await else {
            return;
        };
        match api.get_playlist_raw(&browse_id, 50).await {
            Ok(videos) => {
                if browse_id == LIKED_ID {
                    set_liked_tracks(&videos);
                }
                let _ = updater.send(ManagerMessage::Library(LibraryChange::Refreshed {
                    playlist: browse_id,
                    videos,
                }));
            }
            Err(e) => error!("refresh playlist failed: {e:?}"),
        }
    });
}

pub fn set_liked(updater: Sender<ManagerMessage>, video: YoutubeMusicVideoRef, liked: bool) {
    let id = video.video_id.clone();
    let mirror = if liked {
        LibraryChange::Added {
            playlist: LIKED_ID.to_owned(),
            video,
        }
    } else {
        LibraryChange::Removed {
            playlist: LIKED_ID.to_owned(),
            video_id: id.clone(),
        }
    };
    let done = if liked { "Liked" } else { "Like removed" };
    run(updater, done.to_owned(), mirror, move |api| async move {
        api.set_liked(&id, liked).await?;
        let mut set = LIKED.write().unwrap();
        if liked {
            set.insert(id);
        } else {
            set.remove(&id);
        }
        Ok(true)
    });
}

pub fn add_to_playlist(
    updater: Sender<ManagerMessage>,
    browse_id: String,
    name: String,
    video: YoutubeMusicVideoRef,
) {
    let id = video.video_id.clone();
    let mirror = LibraryChange::Added {
        playlist: browse_id.clone(),
        video,
    };
    run(updater, format!("Added to {name}"), mirror, move |api| async move {
        api.add_to_playlist(&browse_id, &id).await.map(|_| true)
    });
}

pub fn remove_from_playlist(updater: Sender<ManagerMessage>, browse_id: String, video_id: String) {
    let mirror = LibraryChange::Removed {
        playlist: browse_id.clone(),
        video_id: video_id.clone(),
    };
    run(
        updater,
        "Removed from the playlist".to_owned(),
        mirror,
        move |api| async move { api.remove_from_playlist(&browse_id, &video_id).await },
    );
}

pub fn create_playlist(updater: Sender<ManagerMessage>, name: String) {
    if crate::is_guest() {
        toast(&updater, "Sign in first (F8)");
        return;
    }
    run_service(async move {
        let Some(api) = crate::api_instance().await else {
            toast(&updater, "No connection to YouTube Music");
            return;
        };
        match api.create_playlist(&name).await {
            Ok(browse_id) => {
                BROWSED_PLAYLISTS
                    .lock()
                    .unwrap()
                    .push((name.clone(), browse_id.clone()));
                let _ = updater.send(ManagerMessage::Library(LibraryChange::Created {
                    name: name.clone(),
                    browse_id,
                }));
                toast(&updater, format!("Playlist \"{name}\" created"));
            }
            Err(e) => {
                error!("create playlist failed: {e:?}");
                toast(&updater, "YouTube Music rejected the change");
            }
        }
    });
}

pub fn delete_playlist(updater: Sender<ManagerMessage>, browse_id: String) {
    let mirror = LibraryChange::Deleted {
        browse_id: browse_id.clone(),
    };
    run(
        updater,
        "Playlist deleted".to_owned(),
        mirror,
        move |api| async move {
            api.delete_playlist(&browse_id).await?;
            BROWSED_PLAYLISTS
                .lock()
                .unwrap()
                .retain(|(_, id)| id != &browse_id);
            Ok(true)
        },
    );
}
