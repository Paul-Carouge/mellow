//! One Mellow per person (ADR-0056): launching it again shows the running one's window instead of
//! starting a second session, which would sign in twice and fight over the microphone. Checked in
//! `main` before GPUI starts, so a second launch opens no window and reads no secret.
//!
//! - **macOS and Linux**: a lock file held for the app's life, and a Unix socket beside it in a
//!   directory only the person can open (`$XDG_RUNTIME_DIR/mellow` on Linux, the local data
//!   directory otherwise). A second launch finds the lock taken and connects to the socket. The
//!   lock tells a socket left by a crash from a live one, which a socket alone cannot. On macOS,
//!   Launch Services already keeps one instance of the bundle (a second launch reaches
//!   `applicationShouldHandleReopen`); this also covers a binary started directly.
//! - **Windows**: a named pipe per account, created with `FILE_FLAG_FIRST_PIPE_INSTANCE` so only
//!   the first launch owns it; a second launch opens it as a client. Rejects remote clients.
//!
//! The running Mellow answers one byte once it took the request. No answer means it is quitting
//! (Mellow launched again right after Quit): the launch waits, bounded, for it to let go, then
//! starts.
//!
//! Any failure to set this up lets the launch go on as before: a second instance is better than
//! no Mellow.

use std::io::{self, Read, Write};
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;

/// Requests that coalesce: one pending show serves every launch made meanwhile.
const SHOW_REQUEST_QUEUE_CAPACITY: usize = 1;
/// What the running Mellow answers once a launch's request is taken.
const REQUEST_TAKEN: u8 = 1;
/// The listener answers at once unless Mellow is quitting, when it closes without answering.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(1);
/// Longer than a quit takes: leaving the call (0.5 s), closing the gateway (0.2 s) and signing
/// out (3 s) at most.
const QUITTING_INSTANCE_WAIT: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

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
        windows_pipe::claim(&windows_pipe::pipe_name()?)
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
    Some(base.join(&*crate::profile::with_profile_suffix(
        crate::pictures::disk_cache::APPLICATION_DIRECTORY,
    )))
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
    let deadline = std::time::Instant::now() + QUITTING_INSTANCE_WAIT;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Err(error),
        }
        let was_taken = UnixStream::connect(&socket_path).is_ok_and(was_request_taken);
        // Still held after the wait: it is starting (its socket not yet bound) and shows its
        // window anyway.
        if was_taken || std::time::Instant::now() >= deadline {
            return Ok(InstanceClaim::AlreadyRunning);
        }
        std::thread::sleep(RETRY_INTERVAL);
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
            // Held until the app quits or the process exits: dropping it lets the next Mellow
            // start.
            let _lock = lock;
            for connection in listener.incoming() {
                match connection {
                    Ok(connection) => {
                        if !take_request(connection, &sender) {
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

/// Passes a launch's request on and answers it; false once the app stopped listening (it is
/// quitting), leaving the launch unanswered so that it waits for this one to end.
fn take_request(mut connection: impl Write, sender: &mpsc::Sender<()>) -> bool {
    match sender.try_send(()) {
        // Full: a show is already pending, and serves this launch too.
        Ok(()) | Err(mpsc::error::TrySendError::Full(())) => {}
        Err(mpsc::error::TrySendError::Closed(())) => return false,
    }
    if let Err(error) = connection.write_all(&[REQUEST_TAKEN]) {
        tracing::debug!(error = %error.kind(), "the later launch left before its answer");
    }
    true
}

/// Whether the running Mellow answered that it took the request. Read on a thread of its own,
/// since a Windows pipe read cannot time out: a stuck one ends with the launch.
fn was_request_taken(mut connection: impl Read + Send + 'static) -> bool {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::Builder::new()
        .name("mellow-single-instance-answer".into())
        .spawn(move || {
            let mut answer = [0_u8; 1];
            let is_taken =
                matches!(connection.read(&mut answer), Ok(1)) && answer == [REQUEST_TAKEN];
            // The launch stopped waiting: nobody to tell.
            let _ = sender.send(is_taken);
        });
    reader.is_ok() && receiver.recv_timeout(ANSWER_TIMEOUT).unwrap_or(false)
}

// Win32's named pipes, which the standard library can open but not create: every call is
// commented.
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_pipe {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::time::Instant;

    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::core::HSTRING;

    use super::{
        InstanceClaim, QUITTING_INSTANCE_WAIT, RETRY_INTERVAL, SHOW_REQUEST_QUEUE_CAPACITY,
        ShowRequests, take_request, was_request_taken,
    };

    const PIPE_NAME_PREFIX: &str = r"\\.\pipe\org.mellow.desktop-";
    /// Nothing is read (a connection is the request); one byte is answered.
    const INPUT_BUFFER_SIZE: u32 = 0;
    const OUTPUT_BUFFER_SIZE: u32 = 1;
    const DEFAULT_TIMEOUT_MILLISECONDS: u32 = 0;

    /// Pipe names are machine-wide: the account's domain and full name keep two people's Mellows
    /// apart, in any script (a pipe name takes every character but `\`).
    pub(super) fn pipe_name() -> io::Result<String> {
        let account = std::env::var("USERNAME").unwrap_or_default();
        if account.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no account name for the instance pipe",
            ));
        }
        let domain = std::env::var("USERDOMAIN").unwrap_or_default();
        let account = format!("{domain}.{account}").replace('\\', ".");
        let profile = crate::profile::active_profile_name()
            // `;` cannot appear in a Windows account name, so no account's pipe can spell another's.
            .map(|name| format!(";profile={name}"))
            .unwrap_or_default();
        Ok(format!("{PIPE_NAME_PREFIX}{account}{profile}"))
    }

    pub(super) fn claim(name: &str) -> io::Result<InstanceClaim> {
        let deadline = Instant::now() + QUITTING_INSTANCE_WAIT;
        loop {
            if let Some(pipe) = create_first_pipe(name) {
                let (sender, receiver) = mpsc::channel(SHOW_REQUEST_QUEUE_CAPACITY);
                std::thread::Builder::new()
                    .name("mellow-single-instance".into())
                    .spawn(move || listen(&pipe, &sender))?;
                return Ok(InstanceClaim::First(ShowRequests(receiver)));
            }
            // Taken: another Mellow owns the pipe, and opening it is the request.
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(name)
            {
                Ok(running) => {
                    if was_request_taken(running) {
                        return Ok(InstanceClaim::AlreadyRunning);
                    }
                }
                // Answering another launch, whose request serves this one too.
                Err(error) if is_pipe_busy(&error) => return Ok(InstanceClaim::AlreadyRunning),
                // Gone between the two calls: it just quit.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                // Not this person's to open (another account's pipe): start anyway.
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Ok(InstanceClaim::AlreadyRunning);
            }
            std::thread::sleep(RETRY_INTERVAL);
        }
    }

    fn is_pipe_busy(error: &io::Error) -> bool {
        error.raw_os_error() == i32::try_from(ERROR_PIPE_BUSY.0).ok()
    }

    /// The pipe, unless another process owns its name.
    fn create_first_pipe(name: &str) -> Option<File> {
        // SAFETY: the name is a valid wide string for the call's duration; no security attributes
        // means the default ones (the creator and administrators write, others only read).
        let handle = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                OUTPUT_BUFFER_SIZE,
                INPUT_BUFFER_SIZE,
                DEFAULT_TIMEOUT_MILLISECONDS,
                None,
            )
        };
        if handle.is_invalid() {
            return None;
        }
        // SAFETY: `handle` is a valid pipe handle that nothing else owns or closes.
        Some(File::from(unsafe {
            OwnedHandle::from_raw_handle(handle.0)
        }))
    }

    fn listen(pipe: &File, sender: &mpsc::Sender<()>) {
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
            if has_client {
                if !take_request(pipe, sender) {
                    return;
                }
                // Disconnecting drops unread bytes, so this waits until the launch read its
                // answer (`sync_all` is `FlushFileBuffers`); it fails only once the launch left.
                let _ = pipe.sync_all();
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
    fn a_launch_while_mellow_quits_waits_for_it_and_starts() {
        let directory = tempfile::tempdir().unwrap();
        let InstanceClaim::First(requests) = claim_in(directory.path()).unwrap() else {
            panic!("the first launch is the running Mellow");
        };
        // Quitting: the app no longer takes requests, though its process still holds the lock.
        drop(requests);
        assert!(matches!(
            claim_in(directory.path()).unwrap(),
            InstanceClaim::First(_)
        ));
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
