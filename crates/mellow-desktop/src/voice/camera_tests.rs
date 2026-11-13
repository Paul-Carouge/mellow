//! The camera's state machine with synthetic cameras: outside a call (the settings' preview),
//! and in a call against a real in-process `mellow-server`, which must see the camera on.

use std::time::Instant;

use gpui::TestAppContext;
use mellow_media::{RunningCapture, SyntheticCameras};

use super::*;
use crate::session::Session;
use crate::voice::AudioDevicesFactory;

/// Generous: the synthetic camera runs in real time. Never a measure.
const STEP_DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut condition: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + STEP_DEADLINE;
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} did not happen in time");
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn synthetic_cameras() -> CamerasFactory {
    Arc::new(|| {
        Box::new(SyntheticCameras {
            width: 64,
            height: 36,
        })
    })
}

/// Cameras that list one and refuse to open it, as macOS does once access was refused.
struct RefusingCameras;

impl Cameras for RefusingCameras {
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
        Ok(vec![CameraDescription {
            id: "front".into(),
            name: "Front camera".into(),
        }])
    }

    fn open(
        &self,
        _device_id: Option<&str>,
        _frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError> {
        Err(CameraError::PermissionDenied)
    }
}

fn silent_devices() -> AudioDevicesFactory {
    Arc::new(|| {
        let format = mellow_media::StreamFormat {
            sample_rate: 48_000,
            channels: 1,
        };
        Box::new(mellow_media::SyntheticAudioDevices::new(
            mellow_media::SyntheticSound::Silence,
            format,
            format,
        ))
    })
}

fn signed_out_voice(cx: &mut TestAppContext) -> gpui::Entity<Voice> {
    cx.update(|cx| {
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    cx.new(|cx| Voice::new(session, silent_devices(), cx))
}

#[gpui::test]
fn the_preview_shows_a_mirrored_picture_and_stops_with_the_page(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let voice = signed_out_voice(cx);
    voice.update(cx, |voice, cx| {
        voice.set_cameras(synthetic_cameras());
        assert_eq!(voice.camera_block(cx), Some(CameraBlock::NotConnected));
        voice.set_camera_preview_shown(true, cx);
        voice.start_camera_preview(cx);
        assert!(voice.is_camera_preview_running());
    });
    wait_until(cx, "the preview's first picture", |cx| {
        voice.read_with(cx, |voice, _| {
            !voice.is_camera_preview_starting()
                && voice
                    .take_camera_preview_picture()
                    .is_some_and(|picture| (picture.width, picture.height) == (64, 36))
        })
    });
    voice.update(cx, |voice, cx| voice.set_camera_preview_shown(false, cx));
    voice.read_with(cx, |voice, _| {
        assert!(!voice.is_camera_preview_running());
        assert_eq!(voice.camera_problem(), None);
    });
}

#[gpui::test]
fn a_refused_preview_says_so(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let voice = signed_out_voice(cx);
    voice.update(cx, |voice, cx| {
        voice.set_cameras(Arc::new(|| Box::new(RefusingCameras)));
        voice.start_camera_preview(cx);
    });
    wait_until(cx, "the refusal", |cx| {
        voice.read_with(cx, |voice, _| {
            voice.camera_preview_problem() == Some(CameraProblem::Denied)
        })
    });
    voice.read_with(cx, |voice, _| assert!(!voice.is_camera_preview_running()));
}

#[test]
fn a_problem_says_what_to_do() {
    let strings = Strings::new(crate::i18n::Language::English);
    assert_eq!(
        CameraProblem::of_error(&CameraError::Busy).sentence(strings),
        strings.camera().problems().in_use()
    );
    assert_eq!(
        CameraProblem::of_error(&CameraError::NoCamera),
        CameraProblem::NotFound
    );
    assert_eq!(
        CameraProblem::Refused(VoiceVideoRefusal::MissingPermissions).sentence(strings),
        strings.camera().problems().missing_permissions()
    );
    assert_eq!(
        CameraProblem::Denied.can_open_system_settings(),
        cfg!(any(target_os = "macos", windows))
    );
}

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#[cfg(not(windows))]
mod in_a_call {
    use gpui::Entity;
    use tokio::runtime::Runtime;

    use super::*;
    use crate::tokio_bridge::build_network_runtime;
    use mellow_protocol::Snowflake;

    use crate::voice::tests::{RunningInstance, owner_with_two_voice_channels};

    fn is_video_on(session: &Entity<Session>, cx: &mut TestAppContext) -> bool {
        session.read_with(cx, |session, _| {
            let state = session.state();
            state
                .current_user()
                .and_then(|user| state.voice_state_of(user.id))
                .is_some_and(|voice_state| voice_state.self_video)
        })
    }

    #[gpui::test]
    fn the_camera_turns_on_moves_to_another_camera_and_turns_off(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, channel_id, _) =
            owner_with_two_voice_channels(&runtime, &instance, cx);
        voice.update(cx, |voice, cx| {
            voice.set_cameras(synthetic_cameras());
            voice.join(channel_id, cx);
        });
        wait_until(cx, "the call and its camera list", |cx| {
            voice.read_with(cx, |voice, cx| {
                voice.phase() == Some(CallPhase::Connected)
                    && voice.listed_cameras().is_some()
                    && voice.camera_block(cx).is_none()
            })
        });

        voice.update(cx, |voice, cx| {
            voice.set_camera_preview_shown(true, cx);
            voice.toggle_camera(cx);
            assert_eq!(voice.camera_phase(), CameraPhase::Starting);
        });
        wait_until(cx, "the camera on, seen by the instance", |cx| {
            voice.read_with(cx, |voice, _| voice.camera_phase() == CameraPhase::On)
                && is_video_on(&session, cx)
        });
        wait_until(cx, "the call's camera in the settings' preview", |cx| {
            voice.read_with(cx, |voice, _| voice.take_camera_preview_picture().is_some())
        });

        // Another camera chosen: the camera reopens on it and stays on.
        cx.update(|cx| {
            crate::voice::change_voice_settings(cx, |settings| {
                settings.camera_device_id = Some("synthetic".into());
            });
        });
        wait_until(cx, "the camera back on", |cx| {
            voice.read_with(cx, |voice, _| voice.camera_phase() == CameraPhase::On)
        });
        assert!(is_video_on(&session, cx));

        voice.update(cx, Voice::toggle_camera);
        assert_eq!(
            voice.read_with(cx, |voice, _| voice.camera_phase()),
            CameraPhase::Off
        );
        wait_until(cx, "the camera off, seen by the instance", |cx| {
            !is_video_on(&session, cx)
        });

        voice.update(cx, Voice::leave);
        drop((voice, session));
        cx.run_until_parked();
        runtime.block_on(instance.stop());
    }

    #[gpui::test]
    fn a_refused_camera_never_reaches_the_server(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, channel_id, _) =
            owner_with_two_voice_channels(&runtime, &instance, cx);
        voice.update(cx, |voice, cx| {
            voice.set_cameras(Arc::new(|| Box::new(RefusingCameras)));
            voice.join(channel_id, cx);
        });
        wait_until(cx, "the call", |cx| {
            voice.read_with(cx, |voice, cx| {
                voice.phase() == Some(CallPhase::Connected) && voice.camera_block(cx).is_none()
            })
        });
        voice.update(cx, Voice::toggle_camera);
        wait_until(cx, "the refusal", |cx| {
            voice.read_with(cx, |voice, _| {
                voice.camera_problem() == Some(CameraProblem::Denied)
            })
        });
        voice.read_with(cx, |voice, _| {
            assert_eq!(voice.camera_phase(), CameraPhase::Off);
        });
        assert!(!is_video_on(&session, cx));

        voice.update(cx, Voice::leave);
        drop((voice, session));
        cx.run_until_parked();
        runtime.block_on(instance.stop());
    }

    #[gpui::test]
    fn no_camera_disables_the_button(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, channel_id, _) =
            owner_with_two_voice_channels(&runtime, &instance, cx);
        voice.update(cx, |voice, cx| {
            voice.set_cameras(Arc::new(|| Box::new(NoCameras)));
            voice.join(channel_id, cx);
        });
        wait_until(cx, "the empty camera list", |cx| {
            voice.read_with(cx, |voice, cx| {
                voice.phase() == Some(CallPhase::Connected)
                    && voice.camera_block(cx) == Some(CameraBlock::NoCamera)
            })
        });
        voice.update(cx, Voice::toggle_camera);
        assert_eq!(
            voice.read_with(cx, |voice, _| voice.camera_phase()),
            CameraPhase::Off
        );

        voice.update(cx, Voice::leave);
        drop((voice, session));
        cx.run_until_parked();
        runtime.block_on(instance.stop());
    }

    /// Joins the first channel with `cameras` and turns the camera on, seen by the instance.
    fn camera_on_in_a_call(
        runtime: &Runtime,
        instance: &RunningInstance,
        cameras: CamerasFactory,
        cx: &mut TestAppContext,
    ) -> (Entity<Session>, Entity<Voice>, Snowflake) {
        let (session, voice, channel_id, other_channel_id) =
            owner_with_two_voice_channels(runtime, instance, cx);
        voice.update(cx, |voice, cx| {
            voice.set_cameras(cameras);
            voice.join(channel_id, cx);
        });
        wait_until(cx, "the call", |cx| {
            voice.read_with(cx, |voice, cx| {
                voice.phase() == Some(CallPhase::Connected)
                    && voice.listed_cameras().is_some()
                    && voice.camera_block(cx).is_none()
            })
        });
        voice.update(cx, Voice::toggle_camera);
        wait_until(cx, "the camera on, seen by the instance", |cx| {
            voice.read_with(cx, |voice, _| voice.camera_phase() == CameraPhase::On)
                && is_video_on(&session, cx)
        });
        (session, voice, other_channel_id)
    }

    fn end(
        runtime: &Runtime,
        instance: RunningInstance,
        session: Entity<Session>,
        voice: Entity<Voice>,
        cx: &mut TestAppContext,
    ) {
        voice.update(cx, Voice::leave);
        drop((voice, session));
        cx.run_until_parked();
        runtime.block_on(instance.stop());
    }

    /// Whether your tile has a picture of your camera; moves the test clock past one of the
    /// tile's frame periods first, so its pump runs.
    fn is_own_camera_shown(voice: &Entity<Voice>, cx: &mut TestAppContext) -> bool {
        cx.executor().advance_clock(Duration::from_millis(50));
        voice.read_with(cx, |voice, cx| {
            voice.video_pictures().read(cx).own_camera().is_some()
        })
    }

    #[gpui::test]
    fn your_tile_shows_your_camera_while_the_call_view_is_open(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, _) = camera_on_in_a_call(&runtime, &instance, synthetic_cameras(), cx);

        voice.update(cx, Voice::open_call_view);
        wait_until(cx, "your camera in your tile", |cx| {
            is_own_camera_shown(&voice, cx)
        });
        voice.update(cx, Voice::toggle_camera);
        wait_until(
            cx,
            "your tile without a picture once the camera is off",
            |cx| !is_own_camera_shown(&voice, cx),
        );

        voice.update(cx, Voice::toggle_camera);
        wait_until(cx, "your camera back in your tile", |cx| {
            is_own_camera_shown(&voice, cx)
        });
        voice.update(cx, Voice::close_call_view);
        assert!(!is_own_camera_shown(&voice, cx));

        end(&runtime, instance, session, voice, cx);
    }

    #[gpui::test]
    fn the_camera_stays_behind_when_moving_to_another_channel(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, other_channel_id) =
            camera_on_in_a_call(&runtime, &instance, synthetic_cameras(), cx);

        voice.update(cx, |voice, cx| voice.join(other_channel_id, cx));
        voice.read_with(cx, |voice, _| {
            assert_eq!(voice.camera_phase(), CameraPhase::Off);
        });
        wait_until(cx, "the move, without the camera", |cx| {
            voice.read_with(cx, |voice, _| {
                voice.phase() == Some(CallPhase::Connected)
                    && voice.channel_id() == Some(other_channel_id)
            }) && !is_video_on(&session, cx)
        });

        end(&runtime, instance, session, voice, cx);
    }

    #[gpui::test]
    fn a_camera_chosen_live_that_fails_leaves_the_one_on(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, _) =
            camera_on_in_a_call(&runtime, &instance, Arc::new(|| Box::new(PickyCameras)), cx);

        cx.update(|cx| {
            crate::voice::change_voice_settings(cx, |settings| {
                settings.camera_device_id = Some(REFUSING_CAMERA_ID.into());
            });
        });
        wait_until(cx, "the new camera's refusal", |cx| {
            voice.read_with(cx, |voice, _| {
                voice.camera_problem() == Some(CameraProblem::Denied)
            })
        });
        voice.read_with(cx, |voice, _| {
            assert_eq!(voice.camera_phase(), CameraPhase::On);
        });
        assert!(is_video_on(&session, cx));
        voice.update(cx, |voice, cx| voice.set_camera_preview_shown(true, cx));
        wait_until(cx, "the camera that stayed on, in the preview", |cx| {
            voice.read_with(cx, |voice, _| voice.take_camera_preview_picture().is_some())
        });

        end(&runtime, instance, session, voice, cx);
    }

    #[gpui::test]
    fn the_servers_reason_survives_the_camera_closing_first(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, _) = camera_on_in_a_call(&runtime, &instance, synthetic_cameras(), cx);

        // The engine ends the video as a refusal does: the camera's input closes first.
        voice.update(cx, |voice, _| {
            if let Some(engine) = voice.engine() {
                engine.stop_video();
            }
        });
        wait_until(cx, "the camera closed", |cx| {
            voice.read_with(cx, |voice, _| voice.camera_phase() == CameraPhase::Off)
        });
        voice.read_with(cx, |voice, _| {
            assert_eq!(voice.camera_problem(), Some(CameraProblem::Stopped));
        });
        let refusal = VoiceSessionEvent::VideoRefused(VoiceVideoRefusal::TooManyCameras);
        voice.update(cx, |voice, cx| {
            voice.note_video_stopped_by_server(&refusal, cx);
        });
        voice.read_with(cx, |voice, _| {
            assert_eq!(
                voice.camera_problem(),
                Some(CameraProblem::Refused(VoiceVideoRefusal::TooManyCameras))
            );
        });
        // A later refusal belongs to whatever is shared next, not to the camera.
        voice.update(cx, Voice::dismiss_camera_problem);
        voice.update(cx, |voice, cx| {
            voice.note_video_stopped_by_server(&refusal, cx);
        });
        voice.read_with(cx, |voice, _| assert_eq!(voice.camera_problem(), None));

        end(&runtime, instance, session, voice, cx);
    }

    const REFUSING_CAMERA_ID: &str = "refusing";

    /// The synthetic camera, and a second one that refuses to open.
    struct PickyCameras;

    impl PickyCameras {
        fn synthetic() -> SyntheticCameras {
            SyntheticCameras {
                width: 64,
                height: 36,
            }
        }
    }

    impl Cameras for PickyCameras {
        fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
            let mut listed = Self::synthetic().cameras()?;
            listed.push(CameraDescription {
                id: REFUSING_CAMERA_ID.into(),
                name: "Refusing camera".into(),
            });
            Ok(listed)
        }

        fn open(
            &self,
            device_id: Option<&str>,
            frames: VideoInput,
        ) -> Result<Box<dyn RunningCapture>, CameraError> {
            if device_id == Some(REFUSING_CAMERA_ID) {
                return Err(CameraError::PermissionDenied);
            }
            Self::synthetic().open(device_id, frames)
        }
    }

    struct NoCameras;

    impl Cameras for NoCameras {
        fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
            Ok(Vec::new())
        }

        fn open(
            &self,
            _device_id: Option<&str>,
            _frames: VideoInput,
        ) -> Result<Box<dyn RunningCapture>, CameraError> {
            Err(CameraError::NoCamera)
        }
    }
}
