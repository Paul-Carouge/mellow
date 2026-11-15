//! The sound player's rules (`web/src/components/audio-player.ts`, DESIGN.md 5.4, "Sound
//! player"): the waveform's bars, how keys move the playhead, the speeds. Clients draw them.

use super::format::Locale;

/// Arrow keys move the playhead this far, as most players do.
pub const SEEK_STEP_SECONDS: f64 = 5.0;
/// Page Up and Page Down move it a tenth of the sound.
const SEEK_PAGE_FRACTION: f64 = 0.1;
/// Before the sound's length is known, an arrow moves it a twentieth.
const SEEK_STEP_FRACTION_WITHOUT_DURATION: f64 = 0.05;

/// The speeds the speed key cycles through: voice notes at 1.5× and 2×, music at 1×.
pub const PLAYBACK_SPEEDS: [f32; 3] = [1.0, 1.5, 2.0];

/// A key on the focused waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformKey {
    Back,
    Forward,
    PageBack,
    PageForward,
    Start,
    End,
}

impl WaveformKey {
    /// GPUI's key names (`left`, `pageup`…); `None` for keys the waveform leaves alone. Left and
    /// Down go back, Right and Up forward, as every slider.
    #[must_use]
    pub fn from_key_name(key: &str) -> Option<Self> {
        Some(match key {
            "left" | "down" => Self::Back,
            "right" | "up" => Self::Forward,
            "pagedown" => Self::PageBack,
            "pageup" => Self::PageForward,
            "home" => Self::Start,
            "end" => Self::End,
            _ => return None,
        })
    }
}

/// The flat line drawn while the sound's shape is unknown.
#[must_use]
pub fn placeholder_levels(bar_count: usize, level: f32) -> Vec<f32> {
    vec![level; bar_count]
}

/// How many bars are behind the playhead: a bar counts once the playhead passed its middle.
#[must_use]
pub fn played_bar_count(fraction: f64, bar_count: usize) -> usize {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    // A share of a few dozen bars, clamped to 0 and their count.
    let played = (fraction.clamp(0.0, 1.0) * bar_count as f64).round() as usize;
    played.min(bar_count)
}

/// Where the sound is, 0 to 1; 0 while its length is unknown.
#[must_use]
pub fn playback_fraction(current_seconds: f64, duration_seconds: Option<f64>) -> f64 {
    match duration_seconds {
        Some(duration) if duration.is_finite() && duration > 0.0 => {
            (current_seconds / duration).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Where `key` moves the playhead, as a fraction of the sound.
#[must_use]
pub fn seek_fraction_after_key(
    key: WaveformKey,
    fraction: f64,
    duration_seconds: Option<f64>,
) -> f64 {
    let step = match duration_seconds {
        Some(duration) if duration.is_finite() && duration > 0.0 => SEEK_STEP_SECONDS / duration,
        _ => SEEK_STEP_FRACTION_WITHOUT_DURATION,
    };
    let moved = match key {
        WaveformKey::Back => fraction - step,
        WaveformKey::Forward => fraction + step,
        WaveformKey::PageBack => fraction - SEEK_PAGE_FRACTION,
        WaveformKey::PageForward => fraction + SEEK_PAGE_FRACTION,
        WaveformKey::Start => 0.0,
        WaveformKey::End => 1.0,
    };
    moved.clamp(0.0, 1.0)
}

/// The speed after `speed` in the cycle; back to 1× after the last or from any other speed.
#[must_use]
pub fn next_playback_speed(speed: f32) -> f32 {
    #[allow(clippy::float_cmp)] // Speeds are only ever taken from the list.
    let index = PLAYBACK_SPEEDS.iter().position(|known| *known == speed);
    index
        .and_then(|index| PLAYBACK_SPEEDS.get(index + 1))
        .copied()
        .unwrap_or(PLAYBACK_SPEEDS[0])
}

/// "1,5" in French, "1.5" in English: the speed key shows it with "×".
#[must_use]
pub fn playback_speed_number(speed: f32, locale: Locale) -> String {
    let text = format!("{}", (speed * 100.0).round() / 100.0);
    match locale {
        Locale::English => text,
        Locale::French => text.replace('.', ","),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from `audio-player.test.ts`.

    #[test]
    fn a_bar_counts_as_played_once_the_playhead_passed_its_middle() {
        assert_eq!(played_bar_count(0.0, 48), 0);
        assert_eq!(played_bar_count(0.5, 48), 24);
        assert_eq!(played_bar_count(0.01, 48), 0);
        assert_eq!(played_bar_count(0.011, 48), 1);
        assert_eq!(played_bar_count(1.0, 48), 48);
        assert_eq!(played_bar_count(1.4, 48), 48);
        assert_eq!(played_bar_count(-1.0, 48), 0);
        assert_eq!(placeholder_levels(3, 0.16), [0.16, 0.16, 0.16]);
    }

    #[test]
    fn the_fraction_is_zero_until_the_length_is_known() {
        assert!((playback_fraction(5.0, Some(20.0)) - 0.25).abs() < 1e-12);
        assert!((playback_fraction(30.0, Some(20.0)) - 1.0).abs() < 1e-12);
        assert!(playback_fraction(5.0, None).abs() < 1e-12);
        assert!(playback_fraction(5.0, Some(0.0)).abs() < 1e-12);
        assert!(playback_fraction(5.0, Some(f64::INFINITY)).abs() < 1e-12);
    }

    #[test]
    fn arrows_move_five_seconds_pages_a_tenth_and_home_and_end_to_the_ends() {
        let near = |actual: f64, expected: f64| (actual - expected).abs() < 1e-12;
        assert!(near(
            seek_fraction_after_key(WaveformKey::Forward, 0.5, Some(100.0)),
            0.55
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::Back, 0.5, Some(100.0)),
            0.45
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::Back, 0.01, Some(100.0)),
            0.0
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::Forward, 0.99, Some(100.0)),
            1.0
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::PageForward, 0.5, Some(100.0)),
            0.6
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::PageBack, 0.5, None),
            0.4
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::Forward, 0.5, None),
            0.55
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::Start, 0.5, None),
            0.0
        ));
        assert!(near(
            seek_fraction_after_key(WaveformKey::End, 0.5, None),
            1.0
        ));
    }

    #[test]
    fn keys_are_read_from_gpui_s_names() {
        assert_eq!(WaveformKey::from_key_name("left"), Some(WaveformKey::Back));
        assert_eq!(WaveformKey::from_key_name("down"), Some(WaveformKey::Back));
        assert_eq!(WaveformKey::from_key_name("up"), Some(WaveformKey::Forward));
        assert_eq!(
            WaveformKey::from_key_name("pageup"),
            Some(WaveformKey::PageForward)
        );
        assert_eq!(WaveformKey::from_key_name("home"), Some(WaveformKey::Start));
        assert_eq!(WaveformKey::from_key_name("space"), None);
    }

    #[test]
    fn the_speed_cycles_back_to_one_and_reads_in_the_locale() {
        assert!((next_playback_speed(1.0) - 1.5).abs() < f32::EPSILON);
        assert!((next_playback_speed(1.5) - 2.0).abs() < f32::EPSILON);
        assert!((next_playback_speed(2.0) - 1.0).abs() < f32::EPSILON);
        assert!((next_playback_speed(1.25) - 1.0).abs() < f32::EPSILON);
        assert_eq!(playback_speed_number(1.5, Locale::French), "1,5");
        assert_eq!(playback_speed_number(1.5, Locale::English), "1.5");
        assert_eq!(playback_speed_number(2.0, Locale::French), "2");
    }
}
