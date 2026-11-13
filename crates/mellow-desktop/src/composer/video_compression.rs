//! Videos compressed before they upload (ADR-0046, the web's `compressThenUpload`): the tray
//! item says "Compressing… 42%" with "Send the original"; the encoder runs on a thread of its
//! own (`mellow_media::compress_video`), and the upload starts with the compressed file, or with
//! the original when the person asked for it, when compression kept it or when it failed.
//! Removing the item stops the encoder and nothing uploads.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::Context;
use mellow_media::{
    VideoCompressionCancelled, VideoCompressionOutcome, compressed_video_file_name,
};

use super::Composer;
use super::drafts::{TrayItemStatus, VideoCompression};

/// Progress reports waiting for the interface; older ones are dropped when it falls behind.
const PROGRESS_QUEUE_CAPACITY: usize = 4;
const PERCENT: f64 = 100.0;

type CompressVideo = dyn Fn(
        &Path,
        Option<u64>,
        &mut dyn FnMut(f64),
        &AtomicBool,
    ) -> Result<VideoCompressionOutcome, VideoCompressionCancelled>
    + Send
    + Sync;

type StartThread = dyn Fn(Box<dyn FnOnce() + Send>) -> std::io::Result<()>;

/// How videos are compressed: the system's encoder on a thread of its own, or a test's fake.
#[derive(Clone)]
pub(crate) struct VideoCompressor {
    pub compress: Arc<CompressVideo>,
    pub start_thread: Arc<StartThread>,
}

impl std::fmt::Debug for VideoCompressor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoCompressor")
            .finish_non_exhaustive()
    }
}

impl VideoCompressor {
    /// The system's encoder, where this system compresses videos (not Windows yet).
    pub(crate) fn system() -> Option<Self> {
        mellow_media::can_compress_videos_here().then(|| Self {
            compress: Arc::new(mellow_media::compress_video),
            start_thread: Arc::new(|work| {
                std::thread::Builder::new()
                    .name("mellow-video-compression".into())
                    .spawn(work)
                    .map(|_| ())
            }),
        })
    }
}

impl Composer {
    /// Compresses the video with `key` before its upload starts.
    pub(super) fn start_compression(
        &mut self,
        key: u64,
        compressor: &VideoCompressor,
        cx: &mut Context<Self>,
    ) {
        let maximum_attachment_bytes = self.maximum_attachment_bytes;
        let Some(item) = self.drafts.tray_item_mut(key) else {
            return;
        };
        let path = item.path.clone();
        let is_cancelled = Arc::new(AtomicBool::new(false));
        let (progress, mut received_progress) =
            tokio::sync::mpsc::channel::<u8>(PROGRESS_QUEUE_CAPACITY);
        let (outcome_sender, outcome) = tokio::sync::oneshot::channel();
        let compress = Arc::clone(&compressor.compress);
        let thread_is_cancelled = Arc::clone(&is_cancelled);
        let started = (compressor.start_thread)(Box::new(move || {
            let outcome = compress(
                &path,
                maximum_attachment_bytes,
                &mut |fraction| {
                    // From 0 to 100.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let percent = (fraction.clamp(0.0, 1.0) * PERCENT).floor() as u8;
                    // A full queue drops this report: a newer one follows.
                    let _ = progress.try_send(percent);
                },
                &thread_is_cancelled,
            );
            // The composer may be gone: nothing waits then.
            let _ = outcome_sender.send(outcome);
        }));
        if let Err(error) = started {
            tracing::warn!(error = %error.kind(), "cannot start a video's compression; the original is sent");
            self.start_upload(key, cx);
            return;
        }
        let task = cx.spawn(async move |composer, cx| {
            while let Some(percent) = received_progress.recv().await {
                let is_gone = composer
                    .update(cx, |composer, cx| {
                        if let Some(item) = composer.drafts.tray_item_mut(key)
                            && matches!(item.status, TrayItemStatus::Compressing(_))
                        {
                            item.status = TrayItemStatus::Compressing(percent);
                            cx.notify();
                        }
                    })
                    .is_err();
                if is_gone {
                    return;
                }
            }
            // A thread that ended without an answer kept the original.
            let outcome = outcome
                .await
                .unwrap_or(Ok(VideoCompressionOutcome::KeptOriginal(
                    mellow_media::KeepOriginalReason::Failed,
                )));
            composer
                .update(cx, |composer, cx| {
                    composer.finish_compression(key, outcome, cx);
                })
                .ok();
        });
        if let Some(item) = self.drafts.tray_item_mut(key) {
            item.status = TrayItemStatus::Compressing(0);
            item.compression = Some(VideoCompression {
                is_cancelled,
                task: Some(task),
            });
        }
    }

    fn finish_compression(
        &mut self,
        key: u64,
        outcome: Result<VideoCompressionOutcome, VideoCompressionCancelled>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.drafts.tray_item_mut(key) else {
            return;
        };
        // The encode may have finished just before "Send the original" was clicked: the click wins.
        let was_original_asked = item.compression.take().is_some_and(|mut compression| {
            // This task is the one running now: let it end on its own rather than cancel it.
            if let Some(task) = compression.task.take() {
                task.detach();
            }
            compression.is_cancelled.load(Ordering::Relaxed)
        });
        // "Send the original" and kept originals upload the file as it is.
        if let Ok(VideoCompressionOutcome::Compressed(bytes)) = outcome
            && !was_original_asked
        {
            item.file_name = compressed_video_file_name(&item.file_name);
            item.size_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            item.compressed_bytes = Some(Arc::new(bytes));
        }
        item.status = TrayItemStatus::Uploading;
        self.start_upload(key, cx);
        cx.notify();
    }

    /// Stops compressing the video with `key`: its original uploads as it is.
    pub(super) fn send_original(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(compression) = self
            .drafts
            .tray_item_mut(key)
            .and_then(|item| item.compression.as_ref())
        {
            compression.is_cancelled.store(true, Ordering::Relaxed);
            cx.notify();
        }
    }
}
