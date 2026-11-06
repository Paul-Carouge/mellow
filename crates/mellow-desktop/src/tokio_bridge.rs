//! Runs network work on Tokio and awaits it from GPUI.
//!
//! `mellow-client-core` uses Tokio's reactor (sockets, timers), which GPUI's executors do not
//! provide: such futures panic when polled there. They run on a small Tokio runtime instead, and
//! GPUI awaits their `JoinHandle`, which works on any executor (ADR-0051 "Threads"). The UI state
//! stays on the main thread; nothing is locked between the two.

use std::future::Future;

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, Global, Task};
use tokio::runtime::{Handle, Runtime};
use tokio::task::{AbortHandle, JoinError};

/// Network I/O and client-core work are light: two workers keep a burst of requests from waiting
/// on a busy one without competing with media and rendering threads.
const NETWORK_WORKER_THREADS: usize = 2;
const NETWORK_THREAD_NAME: &str = "mellow-network";

/// Builds the runtime `main` keeps alive for the whole process, so it shuts down after the UI.
///
/// # Errors
/// When the operating system refuses the worker threads.
pub fn build_network_runtime() -> Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(NETWORK_WORKER_THREADS)
        .thread_name(NETWORK_THREAD_NAME)
        .enable_all()
        .build()
        .context("cannot start the network runtime")
}

/// The runtime's handle, kept as a GPUI global so views can spawn on it.
#[derive(Debug, Clone)]
pub struct NetworkRuntime {
    handle: Handle,
}

impl Global for NetworkRuntime {}

pub fn install_network_runtime(handle: Handle, cx: &mut App) {
    cx.set_global(NetworkRuntime { handle });
}

/// Runs `future` on the network runtime; the returned GPUI task resolves to its output. Dropping
/// the task cancels the Tokio task, so a view that goes away leaves no orphan request behind.
///
/// `install_network_runtime` runs before the first window opens, so the global is always there.
pub fn spawn_on_network_runtime<F>(cx: &App, future: F) -> Task<Result<F::Output, JoinError>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let join_handle = cx.global::<NetworkRuntime>().handle.spawn(future);
    // Built outside the async block, so dropping the task before its first poll still aborts.
    let abort_on_drop = AbortOnDrop(join_handle.abort_handle());
    cx.background_spawn(async move {
        let _abort_on_drop = abort_on_drop;
        join_handle.await
    })
}

#[derive(Debug)]
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        // A no-op once the task has finished.
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tokio::sync::oneshot;

    use super::*;

    #[gpui::test]
    async fn futures_run_on_tokio_and_their_output_reaches_gpui(cx: &mut TestAppContext) {
        let runtime = build_network_runtime().unwrap();
        cx.update(|cx| install_network_runtime(runtime.handle().clone(), cx));
        // The answer comes from a Tokio thread, outside GPUI's deterministic scheduler.
        cx.executor().allow_parking();

        let task = cx.update(|cx| {
            spawn_on_network_runtime(cx, async {
                // Only a Tokio worker has a current runtime and its reactor.
                tokio::runtime::Handle::try_current().is_ok()
            })
        });

        assert!(task.await.unwrap());
        runtime.shutdown_background();
    }

    #[gpui::test]
    async fn dropping_the_gpui_task_cancels_the_tokio_task(cx: &mut TestAppContext) {
        let runtime = build_network_runtime().unwrap();
        cx.update(|cx| install_network_runtime(runtime.handle().clone(), cx));
        cx.executor().allow_parking();
        let (dropped_sender, dropped_receiver) = oneshot::channel::<()>();

        let task = cx.update(|cx| {
            spawn_on_network_runtime(cx, async move {
                // Held until the future is dropped: its drop closes the channel.
                let _dropped_sender = dropped_sender;
                std::future::pending::<()>().await;
            })
        });
        drop(task);

        assert!(
            dropped_receiver.await.is_err(),
            "the Tokio future was dropped without sending"
        );
        runtime.shutdown_background();
    }
}
