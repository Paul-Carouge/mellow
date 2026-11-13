//! A share's sound on Windows through WASAPI's process loopback (Windows 10 2004 and later): the
//! whole system without Mellow's own process tree for a screen (so the call's voices never go
//! back into it), or one application's process tree for a window, never a tree holding Mellow.
//! Read on a thread of its own as 48 kHz stereo float, written to the share's Opus encoder.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, E_FAIL, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceCompletionHandler, IAudioCaptureClient, IAudioClient,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{Interface, PWSTR};

use crate::screen_capture::{ScreenCaptureError, ScreenShareContext, ShareSound, ShareSoundState};
use crate::screen_sound::{PcmSoundEncoder, PcmSoundWriter, SCREEN_SOUND_SAMPLE_RATE};
use crate::windows_media_foundation::{ensure_com, size_as_u32};

/// `WAVE_FORMAT_IEEE_FLOAT` (mmreg.h).
const IEEE_FLOAT_FORMAT_TAG: u16 = 3;
const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = 4;
/// The engine's buffer: 200 ms, in 100 ns units, ample for a thread woken every 10 ms.
const BUFFER_DURATION_HUNDRED_NANOSECONDS: i64 = 2_000_000;
/// How long a wait for the next packet lasts, in milliseconds, before the stop flag is checked
/// again.
const PACKET_WAIT_MILLISECONDS: u32 = 100;
/// The longest executable path read (`MAX_PATH` is 260; long paths go further).
const PROCESS_PATH_CAPACITY: u32 = 1_024;
/// How long Windows may take to activate the loopback client.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);
/// The most parents followed up from Mellow (a reused process id can make a loop).
const MAXIMUM_ANCESTOR_DEPTH: usize = 64;

/// Whose sound a process loopback captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopbackTarget {
    /// Everything but this process tree (Mellow's own).
    AllBut(u32),
    /// This process tree only.
    Only(u32),
}

/// The loopback for `sound`, given the shared source: a whole screen, or a window and its
/// process. A process tree holding Mellow carries no sound (its sounds would include the
/// call's): Mellow itself, and the `refused_processes` (its ancestors, the shell).
pub(crate) fn loopback_target(
    sound: &ShareSound,
    window_process: Option<u32>,
    own_process: u32,
    refused_processes: &[u32],
) -> (Option<LoopbackTarget>, ShareSoundState) {
    let only = |process: u32| {
        (process != own_process && !refused_processes.contains(&process))
            .then_some(LoopbackTarget::Only(process))
    };
    let target = match sound {
        ShareSound::Off => return (None, ShareSoundState::Off),
        ShareSound::Application(id) => id.parse().ok().and_then(only),
        ShareSound::FromSource => match window_process {
            None => Some(LoopbackTarget::AllBut(own_process)),
            Some(process) => only(process),
        },
    };
    match target {
        Some(target) => (Some(target), ShareSoundState::Captured),
        None => (None, ShareSoundState::NotAvailable),
    }
}

/// This process's id.
pub(crate) fn own_process_id() -> u32 {
    // SAFETY: a plain call.
    unsafe { GetCurrentProcessId() }
}

/// The ancestors of `process` (parent first) in `parents`, a list of `(process, parent)`.
fn ancestors_of(process: u32, parents: &[(u32, u32)]) -> Vec<u32> {
    let mut ancestors = Vec::new();
    let mut child = process;
    while ancestors.len() < MAXIMUM_ANCESTOR_DEPTH {
        let Some(&(_, parent)) = parents.iter().find(|(listed, _)| *listed == child) else {
            break;
        };
        if parent == 0 || parent == process || ancestors.contains(&parent) {
            break;
        }
        ancestors.push(parent);
        child = parent;
    }
    ancestors
}

/// Mellow's ancestor processes (the launcher, a terminal, `explorer.exe`); empty when Windows
/// does not list the processes.
pub(crate) fn own_ancestor_processes() -> Vec<u32> {
    let mut parents = Vec::new();
    // SAFETY: the snapshot is closed once; each entry is sized as the calls require.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return Vec::new();
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_as_u32::<PROCESSENTRY32W>(),
            ..Default::default()
        };
        let mut listed = Process32FirstW(snapshot, &raw mut entry);
        while listed.is_ok() {
            parents.push((entry.th32ProcessID, entry.th32ParentProcessID));
            listed = Process32NextW(snapshot, &raw mut entry);
        }
        let _ = CloseHandle(snapshot);
    }
    ancestors_of(own_process_id(), &parents)
}

/// The file name of `process`'s executable, without `.exe`; `None` when Windows does not tell.
pub(crate) fn process_name(process: u32) -> Option<String> {
    // SAFETY: the handle is opened for a query only and closed once; the name buffer is ours.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process).ok()?;
        let mut path = [0_u16; PROCESS_PATH_CAPACITY as usize];
        let mut length = PROCESS_PATH_CAPACITY;
        let queried = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &raw mut length,
        );
        let _ = CloseHandle(handle);
        queried.ok()?;
        let path = String::from_utf16_lossy(path.get(..length as usize)?);
        let file = path.rsplit('\\').next()?;
        let name = file
            .strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".EXE"))
            .unwrap_or(file);
        (!name.is_empty()).then(|| name.to_owned())
    }
}

/// The activation's completion callback, a COM object of its own.
mod activation {
    // The `implement` macro's expansion trips these pedantic lints.
    #![allow(clippy::ref_as_ptr, clippy::inline_always)]

    use std::sync::mpsc;

    use windows::Win32::Foundation::E_FAIL;
    use windows::Win32::Media::Audio::{
        IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
        IActivateAudioInterfaceCompletionHandler_Impl, IAudioClient,
    };
    use windows::core::{Interface, Ref, implement};

    /// Hands the activated client (or the failure) to the waiting thread.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    pub(super) struct ActivationDone(pub(super) mpsc::Sender<windows::core::Result<IAudioClient>>);

    impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationDone_Impl {
        fn ActivateCompleted(
            &self,
            operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            let client = (|| {
                let operation = operation.ok()?;
                let mut status = windows::core::HRESULT::default();
                let mut interface = None;
                // SAFETY: called once Windows finished the activation; both outputs are ours.
                unsafe { operation.GetActivateResult(&raw mut status, &raw mut interface)? };
                status.ok()?;
                interface
                    .ok_or_else(|| windows::core::Error::from(E_FAIL))?
                    .cast::<IAudioClient>()
            })();
            // The waiting thread may have given up.
            let _ = self.0.send(client);
            Ok(())
        }
    }
}

/// An audio client capturing `target`'s sound, activated synchronously.
fn activate_loopback(target: LoopbackTarget) -> windows::core::Result<IAudioClient> {
    let (process, mode) = match target {
        LoopbackTarget::AllBut(process) => {
            (process, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE)
        }
        LoopbackTarget::Only(process) => {
            (process, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE)
        }
    };
    let mut parameters = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: process,
                ProcessLoopbackMode: mode,
            },
        },
    };
    let blob = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_as_u32::<AUDIOCLIENT_ACTIVATION_PARAMS>(),
                        pBlobData: (&raw mut parameters).cast(),
                    },
                },
                ..Default::default()
            }),
        },
    };
    let (done, is_done) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = activation::ActivationDone(done).into();
    // SAFETY: the parameters and the variant pointing at them outlive the wait below, after
    // which Windows no longer reads them; the variant owns nothing (no `PropVariantClear`).
    unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&raw const blob),
            &handler,
        )?;
    }
    is_done
        .recv_timeout(ACTIVATION_TIMEOUT)
        .map_err(|_| windows::core::Error::from(E_FAIL))?
}

/// An owned event handle, closed when dropped.
struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: created by `CreateEventW`, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Starts the client and returns its capture service and the event it signals.
fn start_capture(client: &IAudioClient) -> windows::core::Result<(IAudioCaptureClient, Event)> {
    let format = WAVEFORMATEX {
        wFormatTag: IEEE_FLOAT_FORMAT_TAG,
        nChannels: CHANNELS,
        nSamplesPerSec: SCREEN_SOUND_SAMPLE_RATE,
        nAvgBytesPerSec: SCREEN_SOUND_SAMPLE_RATE * u32::from(CHANNELS * BYTES_PER_SAMPLE),
        nBlockAlign: CHANNELS * BYTES_PER_SAMPLE,
        wBitsPerSample: BYTES_PER_SAMPLE * 8,
        cbSize: 0,
    };
    // SAFETY: plain WASAPI calls on a fresh client with a format that lives for the call.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            BUFFER_DURATION_HUNDRED_NANOSECONDS,
            0,
            &raw const format,
            None,
        )?;
        let event = Event(CreateEventW(None, false, false, None)?);
        client.SetEventHandle(event.0)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok((capture, event))
    }
}

/// Moves every waiting packet into `writer`, deinterleaved through the reused `left` and
/// `right`.
fn drain_packets(
    capture: &IAudioCaptureClient,
    writer: &mut PcmSoundWriter,
    (left, right): (&mut Vec<f32>, &mut Vec<f32>),
) -> windows::core::Result<()> {
    // SAFETY: each buffer is read while held, for the frame count WASAPI gave, then released.
    unsafe {
        while capture.GetNextPacketSize()? > 0 {
            let mut data = std::ptr::null_mut();
            let mut frames = 0_u32;
            let mut flags = 0_u32;
            capture.GetBuffer(&raw mut data, &raw mut frames, &raw mut flags, None, None)?;
            left.clear();
            right.clear();
            let is_silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0.cast_unsigned() != 0;
            if is_silent || data.is_null() {
                left.resize(frames as usize, 0.0);
                right.resize(frames as usize, 0.0);
            } else {
                // 48 kHz stereo float, as asked in `start_capture`: 8 bytes a frame.
                let bytes = std::slice::from_raw_parts(data, frames as usize * 8);
                for frame in bytes.as_chunks::<8>().0 {
                    let (left_bytes, right_bytes) = frame.split_at(4);
                    left.push(f32::from_ne_bytes(
                        left_bytes.try_into().unwrap_or_default(),
                    ));
                    right.push(f32::from_ne_bytes(
                        right_bytes.try_into().unwrap_or_default(),
                    ));
                }
            }
            capture.ReleaseBuffer(frames)?;
            writer.write_planar(left, right);
        }
    }
    Ok(())
}

/// A running loopback; dropping it stops it.
pub(crate) struct RunningShareSound {
    should_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _encoder: PcmSoundEncoder,
}

impl Drop for RunningShareSound {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the screen sound's capture thread panicked");
        }
    }
}

/// Starts capturing `target` into the share's sound line.
///
/// # Errors
/// The share has no picture yet, or WASAPI refused (Windows before 10 2004 has no process
/// loopback).
pub(crate) fn start_share_sound(
    target: LoopbackTarget,
    share: &ScreenShareContext,
) -> Result<RunningShareSound, ScreenCaptureError> {
    let input = share.sound_input().ok_or(ScreenCaptureError::Unavailable)?;
    let (encoder, mut writer) = PcmSoundEncoder::start(input)
        .map_err(|error| ScreenCaptureError::System(error.to_string()))?;
    let should_stop = Arc::new(AtomicBool::new(false));
    let (started, has_started) = mpsc::sync_channel(1);
    let thread = {
        let should_stop = Arc::clone(&should_stop);
        std::thread::Builder::new()
            .name("mellow-screen-sound-windows".into())
            .spawn(move || {
                ensure_com();
                let running = activate_loopback(target).and_then(|client| {
                    let (capture, event) = start_capture(&client)?;
                    Ok((client, capture, event))
                });
                let (client, capture, event) = match running {
                    Ok(running) => {
                        let _ = started.send(Ok(()));
                        running
                    }
                    Err(error) => {
                        let _ = started.send(Err(error.message()));
                        return;
                    }
                };
                let mut left = Vec::new();
                let mut right = Vec::new();
                while !should_stop.load(Ordering::Relaxed) {
                    // SAFETY: a valid event handle, owned by `event` for the loop's life.
                    let woken = unsafe { WaitForSingleObject(event.0, PACKET_WAIT_MILLISECONDS) };
                    if woken == WAIT_OBJECT_0
                        && let Err(error) =
                            drain_packets(&capture, &mut writer, (&mut left, &mut right))
                    {
                        tracing::warn!(%error, "the screen's sound stopped");
                        break;
                    }
                }
                // SAFETY: a started client, stopped once.
                let _ = unsafe { client.Stop() };
            })
            .map_err(|error| ScreenCaptureError::System(error.to_string()))?
    };
    let running = RunningShareSound {
        should_stop,
        thread: Some(thread),
        _encoder: encoder,
    };
    match has_started.recv() {
        Ok(Ok(())) => Ok(running),
        Ok(Err(message)) => Err(ScreenCaptureError::System(message)),
        Err(_) => Err(ScreenCaptureError::System(
            "the screen sound's thread stopped".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_s_sound_leaves_mellow_out_and_a_window_s_is_its_application_s() {
        let own = 10;
        let loopback_target = |sound: &ShareSound, window| loopback_target(sound, window, own, &[]);
        assert_eq!(
            loopback_target(&ShareSound::FromSource, None),
            (Some(LoopbackTarget::AllBut(own)), ShareSoundState::Captured)
        );
        assert_eq!(
            loopback_target(&ShareSound::FromSource, Some(42)),
            (Some(LoopbackTarget::Only(42)), ShareSoundState::Captured)
        );
        assert_eq!(
            loopback_target(&ShareSound::FromSource, Some(own)),
            (None, ShareSoundState::NotAvailable)
        );
        assert_eq!(
            loopback_target(&ShareSound::Application("7".into()), None),
            (Some(LoopbackTarget::Only(7)), ShareSoundState::Captured)
        );
        assert_eq!(
            loopback_target(&ShareSound::Application("seven".into()), None),
            (None, ShareSoundState::NotAvailable)
        );
        assert_eq!(
            loopback_target(&ShareSound::Off, None),
            (None, ShareSoundState::Off)
        );
    }

    #[test]
    fn a_window_of_a_process_holding_mellow_shares_no_sound() {
        // Explorer (4) started a terminal (8) which started Mellow (10).
        let parents = [(10, 8), (8, 4), (4, 1), (1, 0), (42, 4)];
        let ancestors = ancestors_of(10, &parents);
        assert_eq!(ancestors, [8, 4, 1]);
        for ancestor in [8, 4] {
            assert_eq!(
                loopback_target(&ShareSound::FromSource, Some(ancestor), 10, &ancestors),
                (None, ShareSoundState::NotAvailable)
            );
            assert_eq!(
                loopback_target(
                    &ShareSound::Application(ancestor.to_string()),
                    None,
                    10,
                    &ancestors
                ),
                (None, ShareSoundState::NotAvailable)
            );
        }
        // A sibling application keeps its own sound.
        assert_eq!(
            loopback_target(&ShareSound::FromSource, Some(42), 10, &ancestors),
            (Some(LoopbackTarget::Only(42)), ShareSoundState::Captured)
        );
        // A reused process id making a loop ends the walk.
        assert_eq!(ancestors_of(10, &[(10, 8), (8, 10)]), [8]);
    }

    #[test]
    fn this_process_has_a_name_and_a_parent() {
        assert!(process_name(own_process_id()).is_some());
        assert_ne!(own_ancestor_processes(), Vec::<u32>::new());
    }
}
