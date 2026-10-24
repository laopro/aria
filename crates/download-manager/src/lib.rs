mod stream;
mod task;

pub use stream::stream_bytes;

use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use database::YTLocalDatabase;
use tokio::{select, sync::Notify, task::JoinHandle};
use ytpapi2::YoutubeMusicVideoRef;

use common_structs::MusicDownloadStatus;

pub type MessageHandler = Arc<dyn Fn(DownloadManagerMessage) + Send + Sync + 'static>;

pub enum DownloadManagerMessage {
    VideoStatusUpdate(String, MusicDownloadStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Downloader {
    YtDlp,
    #[cfg(feature = "rusty-ytdl-backend")]
    RustyYtdl,
}

pub struct DownloadManager {
    database: &'static YTLocalDatabase,
    cache_dir: PathBuf,
    parallel_downloads: u16,
    pub downloader: Downloader,
    handles: Mutex<Vec<JoinHandle<()>>>,
    download_list: Mutex<VecDeque<YoutubeMusicVideoRef>>,
    in_download: Mutex<HashSet<String>>,
    notify: Notify,
}

impl DownloadManager {
    pub fn new(
        cache_dir: PathBuf,
        database: &'static YTLocalDatabase,
        parallel_downloads: u16,
        downloader: Downloader,
    ) -> Self {
        Self {
            database,
            cache_dir,
            parallel_downloads,
            downloader,
            handles: Mutex::new(Vec::new()),
            download_list: Mutex::new(VecDeque::new()),
            in_download: Mutex::new(HashSet::new()),
            notify: Notify::new(),
        }
    }

    pub fn remove_from_in_downloads(&self, video: &String) {
        self.in_download.lock().unwrap().remove(video);
    }

    fn take(&self) -> Option<YoutubeMusicVideoRef> {
        self.download_list.lock().unwrap().pop_front()
    }

    /// This has to be called as a service stream
    /// HANDLES.lock().unwrap().push(run_service(async move {
    ///     run_service_stream(sender);
    /// }));
    pub fn run_service_stream(
        &'static self,
        cancelation: impl Future<Output = ()> + Clone + Send + 'static,
        sender: MessageHandler,
    ) {
        let fut = async move {
            loop {
                if let Some(id) = self.take() {
                    self.start_download(id, sender.clone()).await;
                } else {
                    self.notify.notified().await;
                }
            }
        };
        let service = tokio::task::spawn(async move {
            select! {
                _ = fut => {},
                _ = cancelation => {},
            }
        });
        self.handles.lock().unwrap().push(service);
    }

    pub fn spawn_system(
        &'static self,
        cancelation: impl Future<Output = ()> + Clone + Send + 'static,
        sender: MessageHandler,
    ) {
        for _ in 0..self.parallel_downloads {
            self.run_service_stream(cancelation.clone(), sender.clone());
        }
    }

    pub fn clean(
        &'static self,
        cancelation: impl Future<Output = ()> + Clone + Send + 'static,
        sender: MessageHandler,
    ) {
        self.download_list.lock().unwrap().clear();
        self.in_download.lock().unwrap().clear();
        {
            let mut handle = self.handles.lock().unwrap();
            for i in handle.iter() {
                i.abort()
            }
            handle.clear();
        }
        self.spawn_system(cancelation, sender);
    }

    pub fn set_download_list(&self, to_add: impl IntoIterator<Item = YoutubeMusicVideoRef>) {
        let mut list = self.download_list.lock().unwrap();
        list.clear();
        list.extend(to_add);
        drop(list);
        self.notify.notify_one();
    }

    pub fn add_to_download_list(&self, to_add: impl IntoIterator<Item = YoutubeMusicVideoRef>) {
        let mut list = self.download_list.lock().unwrap();
        let was_empty = list.is_empty();
        list.extend(to_add);
        drop(list);
        if was_empty {
            self.notify.notify_one();
        }
    }
}
