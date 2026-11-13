//! One Mellow per person (ADR-0055): launching it again shows the running one's window instead of
//! starting a second session, which would sign in twice and fight over the microphone. Checked in
//! `main` before GPUI starts, so a second launch opens no window and reads no secret.
//!
//! - **macOS and Linux**: a lock file held for the app's life, and a Unix socket beside it in a
//!   directory only the person can open (`$XDG_RUNTIME_DIR/mellow` on Linux, the local data
//!   directory otherwise). A second launch finds the lock taken and connects to the socket; the
//!   connection itself is the request, nothing is read from it. The lock tells a socket left by a
//!   crash from a live one, which a socket alone cannot. On macOS, Launch Services already keeps
//!   one instance of the bundle (a second launch reaches `applicationShouldHandleReopen`); this
//!   also covers a binary started directly.
//! - **Windows**: a named pipe per account, created with `FILE_FLAG_FIRST_PIPE_INSTANCE` so only
//!   the first launch owns it; a second launch opens it as a client. Rejects remote clients.
//!
//! Any failure to set this up lets the launch go on as before: a second instance is better than
//! no Mellow.

use std::io;
#[cfg(unix)]
use std::path::{Path, PathBuf};

use tokio::sync::mpsc;

/// Requests that coalesce: one pending show serves every launch made meanwhile.
const SHOW_REQUEST_QUEUE_CAPACITY: usize = 1;

/// The first instance's side: each item is a later launch asking to see the window.
#[derive(Debug)]
pub struct ShowRequests(mpsc::Receiver<()>);

impl ShowRequests {
    /// The next request; `None` once the listener stopped.
    pub async fn next(&mut self) -> Option<()> {
        self.0.recv().await
    }
}

#[derive(Debug)]
pub enum InstanceClaim {
    /// This launch is the running Mellow.
    First(ShowRequests),
    /// Another Mellow runs and was asked to show its window: this launch ends.
    AlreadyRunning,
}

/// Claims the single instance for this person, or asks the running one to show its window.
///
/// # Errors
/// When the lock, socket or pipe cannot be set up; the caller starts anyway.
pub fn claim_single_instance() -> io::Result<InstanceClaim> {
    #[cfg(unix)]
    {
        let directory = instance_directory().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no directory for the instance lock",
            )
        })?;
        claim_in(&directory)
    }
    #[cfg(windows)]
    {
        windows_pipe::claim(&windows_pipe::pipe_name())
    }
}

#[cfg(unix)]
const LOCK_FILE_NAME: &str = "instance.lock";
#[cfg(unix)]
const SOCKET_FILE_NAME: &str = "instance.socket";

/// Per person, never shared: the runtime directory on Linux (a session's, mode 0700), else the
/// local data directory.
#[cfg(unix)]
fn instance_directory() -> Option<PathBuf> {
    let base = dirs::runtime_dir().or_else(dirs::data_local_dir)?;
    Some(base.join(crate::pictures::disk_cache::APPLICATION_DIRECTORY))
}

#[cfg(unix)]
fn claim_in(directory: &Path) -> io::Result<InstanceClaim> {
    use std::fs::{OpenOptions, TryLockError};
    use std::os::unix::net::{UnixListener, UnixStream};

    crate::pictures::disk_cache::create_private_directory(directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(directory.join(LOCK_FILE_NAME))?;
    let socket_path = directory.join(SOCKET_FILE_NAME);
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            // The first instance may still be starting, its socket not yet bound: it runs all the
            // same, so this launch ends either way.
            if let Err(error) = UnixStream::connect(&socket_path) {
                tracing::warn!(error = %error.kind(), "could not ask the running Mellow to show its window");
            }
            return Ok(InstanceClaim::AlreadyRunning);
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
    // Left by an instance that crashed: the lock proves nobody listens on it.
    match std::fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(&socket_path)?;
    let (sender, receiver) = mpsc::channel(SHOW_REQUEST_QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("mellow-single-instance".into())
        .spawn(move || {
            // Held until the process exits: dropping it would let a second Mellow start.
            let _lock = lock;
            for connection in listener.incoming() {
                match connection {
                    Ok(_request) => {
                        if !request_show(&sender) {
                            return;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error.kind(), "a later launch could not reach Mellow");
                    }
                }
            }
        })?;
    Ok(InstanceClaim::First(ShowRequests(receiver)))
}

/// False once the app stopped listening.
fn request_show(sender: &mpsc::Sender<()>) -> bool {
    match sender.try_send(()) {
        // Full: a show is already pending, and serves this launch too.
        Ok(()) | Err(mpsc::error::TrySendError::Full(())) => true,
        Err(mpsc::error::TrySendError::Closed(())) => false,
    }
}

// Win32's named pipes, which the standard library can open but not create: every call is
// commented.
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_pipe {
    use std::io;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::core::HSTRING;

    use super::{InstanceClaim, SHOW_REQUEST_QUEUE_CAPACITY, ShowRequests, request_show};

    const PIPE_NAME_PREFIX: &str = r"\\.\pipe\org.mellow.desktop-";
    /// Nothing is read: a connection is the request.
    const PIPE_BUFFER_SIZE: u32 = 0;
    const DEFAULT_TIMEOUT_MILLISECONDS: u32 = 0;

    /// Pipe names are machine-wide: the account's name keeps two people's Mellows apart.
    pub(super) fn pipe_name() -> String {
        let account: String = std::env::var("USERNAME")
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        format!("{PIPE_NAME_PREFIX}{account}")
    }

    pub(super) fn claim(name: &str) -> io::Result<InstanceClaim> {
        // SAFETY: the name is a valid wide string for the call's duration; no security attributes
        // means the default ones (the creator and administrators write, others only read).
        let handle = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                DEFAULT_TIMEOUT_MILLISECONDS,
                None,
            )
        };
        if handle.is_invalid() {
            // Taken: another Mellow owns the pipe. Opening it is the request; busy with another
            // launch, that one's request serves this one too.
            if let Err(error) = std::fs::OpenOptions::new().write(true).open(name) {
                tracing::warn!(error = %error.kind(), "could not ask the running Mellow to show its window");
            }
            return Ok(InstanceClaim::AlreadyRunning);
        }
        // SAFETY: `handle` is a valid pipe handle that nothing else owns or closes.
        let pipe = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let (sender, receiver) = mpsc::channel(SHOW_REQUEST_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("mellow-single-instance".into())
            .spawn(move || listen(&pipe, &sender))?;
        Ok(InstanceClaim::First(ShowRequests(receiver)))
    }

    fn listen(pipe: &OwnedHandle, sender: &mpsc::Sender<()>) {
        let handle = HANDLE(pipe.as_raw_handle());
        loop {
            // SAFETY: `handle` stays open while `pipe` lives; without overlapped I/O the call
            // blocks this thread until a client connects.
            let connected = unsafe { ConnectNamedPipe(handle, None) };
            let has_client = match connected {
                Ok(()) => true,
                // The client connected between the creation and this call.
                Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => true,
                Err(error) => {
                    tracing::warn!(
                        code = error.code().0,
                        "a later launch could not reach Mellow"
                    );
                    false
                }
            };
            if has_client && !request_show(sender) {
                return;
            }
            // SAFETY: as above; ends this client's connection so the next launch can connect.
            if unsafe { DisconnectNamedPipe(handle) }.is_err() {
                return;
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_second_launch_asks_the_first_to_show_its_window() {
        let directory = tempfile::tempdir().unwrap();
        let InstanceClaim::First(mut requests) = claim_in(directory.path()).unwrap() else {
            panic!("the first launch is the running Mellow");
        };
        assert!(matches!(
            claim_in(directory.path()).unwrap(),
            InstanceClaim::AlreadyRunning
        ));
        assert_eq!(requests.next().await, Some(()));
    }

    #[test]
    fn a_socket_left_by_a_crash_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        // A crashed instance leaves its socket file, but no lock is held on it.
        drop(
            std::os::unix::net::UnixListener::bind(directory.path().join(SOCKET_FILE_NAME))
                .unwrap(),
        );
        assert!(matches!(
            claim_in(directory.path()).unwrap(),
            InstanceClaim::First(_)
        ));
    }
}
