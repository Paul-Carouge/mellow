//! Keeps picture downloads within the server's file budget: 120 per user per minute, each one
//! decrypting the whole file (ADR-0020), so scrolling an image-heavy channel must not burst.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use tokio::time::Instant;

/// Below the server's 120: the same account's other clients and the app's sounds draw on it too.
pub(crate) const DOWNLOADS_PER_WINDOW: usize = 100;
pub(crate) const BUDGET_WINDOW: Duration = Duration::from_secs(60);
/// Enough to fill a screen of pictures quickly without queueing the server's decryptions.
pub(crate) const CONCURRENT_DOWNLOADS: usize = 4;

#[derive(Debug)]
pub(crate) struct DownloadLimiter {
    concurrent_downloads: Semaphore,
    /// When each download of the last window started, oldest first.
    recent_starts: Mutex<VecDeque<Instant>>,
    downloads_per_window: usize,
    window: Duration,
}

impl DownloadLimiter {
    pub(crate) fn new(
        concurrent_downloads: usize,
        downloads_per_window: usize,
        window: Duration,
    ) -> Self {
        Self {
            concurrent_downloads: Semaphore::new(concurrent_downloads),
            recent_starts: Mutex::new(VecDeque::with_capacity(downloads_per_window)),
            downloads_per_window,
            window,
        }
    }

    /// Waits for a download slot and for room in the window; the download holds the permit.
    pub(crate) async fn start_download(&self) -> Option<SemaphorePermit<'_>> {
        // The semaphore is never closed, so `None` cannot happen; it is not unwrapped anyway.
        let permit = self.concurrent_downloads.acquire().await.ok()?;
        loop {
            let wait_until = {
                let mut recent_starts = self.recent_starts.lock().await;
                let now = Instant::now();
                while recent_starts
                    .front()
                    .is_some_and(|start| now.duration_since(*start) >= self.window)
                {
                    recent_starts.pop_front();
                }
                if recent_starts.len() < self.downloads_per_window {
                    recent_starts.push_back(now);
                    return Some(permit);
                }
                recent_starts.front().map(|oldest| *oldest + self.window)
            };
            if let Some(wait_until) = wait_until {
                tokio::time::sleep_until(wait_until).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn downloads_past_the_budget_wait_for_the_window_to_slide() {
        let limiter = DownloadLimiter::new(8, 3, Duration::from_secs(60));
        let start = Instant::now();
        for _ in 0..3 {
            drop(limiter.start_download().await);
        }
        assert_eq!(start.elapsed(), Duration::ZERO);
        tokio::time::advance(Duration::from_secs(10)).await;
        drop(limiter.start_download().await);
        // The fourth waits until the first leaves the window, 60 s after it started.
        assert_eq!(start.elapsed(), Duration::from_secs(60));
        // The fifth and sixth fit (the second and third leave at 60 s too); the seventh waits
        // for the fourth, started at 60 s.
        drop(limiter.start_download().await);
        drop(limiter.start_download().await);
        assert_eq!(start.elapsed(), Duration::from_secs(60));
        drop(limiter.start_download().await);
        assert_eq!(start.elapsed(), Duration::from_secs(120));
    }

    #[tokio::test(start_paused = true)]
    async fn no_more_than_the_concurrency_limit_run_at_once() {
        let limiter = Arc::new(DownloadLimiter::new(2, 100, Duration::from_secs(60)));
        let running = Arc::new(AtomicUsize::new(0));
        let most_running = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..6 {
            let (limiter, running, most_running) = (
                Arc::clone(&limiter),
                Arc::clone(&running),
                Arc::clone(&most_running),
            );
            tasks.push(tokio::spawn(async move {
                let _permit = limiter.start_download().await;
                let now_running = running.fetch_add(1, Ordering::SeqCst) + 1;
                most_running.fetch_max(now_running, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                running.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(most_running.load(Ordering::SeqCst), 2);
    }
}
