//! A sound played faster without sounding higher (DESIGN.md 5.4, "Sound player": 1×, 1,5×, 2×),
//! as a browser's `playbackRate` keeps the pitch: WSOLA (waveform similarity overlap-add). The
//! sound is cut into overlapping 40 ms windows taken `speed` times further apart than they are laid
//! down, each window shifted by up to 10 ms to where it best continues the one before, so voices
//! stay smooth. Works on interleaved frames as they are decoded; every channel takes the same cuts.

/// A window's half, as a share of a second: the hop between windows laid down.
const HALF_WINDOWS_PER_SECOND: u32 = 50;
/// How far a window may move to match the one before, as a share of a second.
const TOLERANCES_PER_SECOND: u32 = 100;
/// The coarse search compares every `rate / MATCH_STEP_RATE`-th sample at as many candidates
/// apart, then refines around the best one frame by frame: the search costs the same at any
/// rate (every fourth sample at 48 kHz, as finely as needed to find the phase).
const MATCH_STEP_RATE: u32 = 12_000;

/// Stretches interleaved frames of `channels` channels to `1 / speed` of their length.
#[derive(Debug)]
pub(crate) struct TimeStretcher {
    channels: usize,
    /// Input frames between two windows taken, for `half` frames laid down.
    analysis_hop: f64,
    half: usize,
    tolerance: usize,
    /// Frames between compared samples, and between coarse candidates.
    match_step: usize,
    /// The nominal start of window `origin_window`: the speed changes from there on.
    origin_frame: f64,
    origin_window: u64,
    /// A raised cosine over the window's rising half; the falling half mirrors it.
    rising: Vec<f32>,
    /// Interleaved input not yet past every window that needs it.
    input: Vec<f32>,
    /// The absolute frame of `input`'s first frame.
    input_start: u64,
    /// Windows laid down so far.
    windows: u64,
    /// Where the last window was taken from, absolute.
    previous_start: Option<u64>,
    /// The last window's falling half, weighted, waiting for the next window's rising half.
    tail: Vec<f32>,
}

impl TimeStretcher {
    /// `speed` above 0; `sample_rate` the frames' rate.
    pub(crate) fn new(channels: usize, sample_rate: u32, speed: f32) -> Self {
        let channels = channels.max(1);
        let half = usize::try_from((sample_rate / HALF_WINDOWS_PER_SECOND).max(1)).unwrap_or(1);
        let tolerance = usize::try_from(sample_rate / TOLERANCES_PER_SECOND).unwrap_or(0);
        #[allow(clippy::cast_precision_loss)] // A window holds far fewer than 2^24 frames.
        let rising = (0..half)
            .map(|index| {
                let phase = (index as f32 + 0.5) / half as f32 * std::f32::consts::FRAC_PI_2;
                phase.sin().powi(2)
            })
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let analysis_hop = half as f64 * f64::from(speed.max(f32::EPSILON));
        Self {
            channels,
            analysis_hop,
            half,
            tolerance,
            match_step: usize::try_from((sample_rate / MATCH_STEP_RATE).max(1)).unwrap_or(1),
            origin_frame: 0.0,
            origin_window: 0,
            rising,
            input: Vec::new(),
            input_start: 0,
            windows: 0,
            previous_start: None,
            tail: Vec::new(),
        }
    }

    /// Plays `speed` times as fast from the next window on, keeping what waits and the last
    /// window's end, so the sound goes on without a click or a gap.
    pub(crate) fn set_speed(&mut self, speed: f32) {
        #[allow(clippy::cast_precision_loss)] // A frame count far below 2^52.
        let next_nominal = self.nominal_start() as f64;
        self.origin_frame = next_nominal;
        self.origin_window = self.windows;
        #[allow(clippy::cast_precision_loss)] // A window holds far fewer than 2^24 frames.
        let half = self.half as f64;
        self.analysis_hop = half * f64::from(speed.max(f32::EPSILON));
    }

    /// Takes `interleaved` frames in and appends what can be laid down already to `output`.
    pub(crate) fn process(&mut self, interleaved: &[f32], output: &mut Vec<f32>) {
        self.input.extend_from_slice(interleaved);
        while let Some(start) = self.next_window_start() {
            self.lay_down(start, output);
            self.forget_input_before(start);
        }
    }

    fn frames_in(&self) -> u64 {
        (self.input.len() / self.channels) as u64
    }

    /// Where the next window would be taken from without matching: `speed` hops per hop laid.
    fn nominal_start(&self) -> u64 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        // A frame count far below 2^52, never negative.
        let nominal = (self.origin_frame
            + (self.windows - self.origin_window) as f64 * self.analysis_hop)
            .round() as u64;
        nominal
    }

    /// Where the next window is taken from, once its whole search range has arrived.
    fn next_window_start(&self) -> Option<u64> {
        let nominal = self.nominal_start();
        let (half, tolerance) = (self.half as u64, self.tolerance as u64);
        let input_end = self.input_start + self.frames_in();
        let Some(previous) = self.previous_start else {
            return (input_end >= nominal + 2 * half).then_some(nominal);
        };
        let earliest = nominal.saturating_sub(tolerance).max(self.input_start);
        let latest = nominal + tolerance;
        // The match compares with what followed the last window's first half.
        if input_end < latest + 2 * half || input_end < previous + 2 * half {
            return None;
        }
        let template = previous + half;
        let step = self.match_step as u64;
        let best_of = |candidates: &mut dyn Iterator<Item = u64>, best: (f32, u64)| {
            candidates.fold(best, |best, candidate| {
                let likeness = self.likeness(template, candidate);
                if likeness > best.0 {
                    (likeness, candidate)
                } else {
                    best
                }
            })
        };
        let coarse = best_of(
            &mut (earliest..=latest).step_by(self.match_step),
            (f32::NEG_INFINITY, nominal.max(earliest)),
        );
        let near = coarse.1.saturating_sub(step).max(earliest)..=(coarse.1 + step).min(latest);
        Some(best_of(&mut near.into_iter(), coarse).1)
    }

    /// How alike `half` frames from `first` and from `second` are (their correlation).
    fn likeness(&self, first: u64, second: u64) -> f32 {
        let at = |frame: u64, offset: usize| {
            let index = usize::try_from(frame - self.input_start).unwrap_or(usize::MAX) + offset;
            let samples = &self.input[index * self.channels..(index + 1) * self.channels];
            samples.iter().sum::<f32>()
        };
        (0..self.half)
            .step_by(self.match_step)
            .map(|offset| at(first, offset) * at(second, offset))
            .sum()
    }

    /// Lays down `half` frames: the last window's falling half under this one's rising half
    /// (the very first window starts at full level), then keeps this one's falling half.
    fn lay_down(&mut self, start: u64, output: &mut Vec<f32>) {
        let channels = self.channels;
        let first = usize::try_from(start - self.input_start).unwrap_or(usize::MAX) * channels;
        let window = &self.input[first..first + 2 * self.half * channels];
        let is_first = self.previous_start.is_none();
        for (frame, weight) in self.rising.iter().enumerate() {
            for channel in 0..channels {
                let index = frame * channels + channel;
                let rising = if is_first { 1.0 } else { *weight };
                let laid = window[index] * rising + self.tail.get(index).copied().unwrap_or(0.0);
                output.push(laid);
            }
        }
        self.tail.clear();
        let falling_half = &window[self.half * channels..];
        for (frame, weight) in self.rising.iter().enumerate() {
            for channel in 0..channels {
                // The falling half is the rising one read backwards: the two sum to one.
                self.tail
                    .push(falling_half[frame * channels + channel] * (1.0 - weight));
            }
        }
        self.previous_start = Some(start);
        self.windows += 1;
    }

    /// Drops input no later window or match can reach.
    fn forget_input_before(&mut self, last_start: u64) {
        let next_nominal = self.nominal_start();
        let keep_from = next_nominal
            .saturating_sub(self.tolerance as u64)
            .min(last_start + self.half as u64);
        if keep_from > self.input_start {
            let frames = usize::try_from(keep_from - self.input_start).unwrap_or(usize::MAX);
            let samples = (frames * self.channels).min(self.input.len());
            self.input.drain(..samples);
            self.input_start += (samples / self.channels) as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 8_000;

    #[allow(clippy::cast_precision_loss)]
    fn tone(frames: usize, channels: usize, hertz: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| {
                let sample = (frame as f32 * hertz * std::f32::consts::TAU / RATE as f32).sin();
                std::iter::repeat_n(sample * 0.5, channels)
            })
            .collect()
    }

    fn stretched(input: &[f32], channels: usize, speed: f32, chunk: usize) -> Vec<f32> {
        let mut stretcher = TimeStretcher::new(channels, RATE, speed);
        let mut output = Vec::new();
        for part in input.chunks(chunk * channels) {
            stretcher.process(part, &mut output);
        }
        output
    }

    /// Rising zero crossings per frame of the first channel: the pitch.
    #[allow(clippy::cast_precision_loss)]
    fn crossings_per_frame(samples: &[f32], channels: usize) -> f32 {
        let first: Vec<f32> = samples.iter().step_by(channels).copied().collect();
        let crossings = first
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count();
        crossings as f32 / first.len() as f32
    }

    #[test]
    fn twice_as_fast_lasts_half_as_long_at_the_same_pitch() {
        let input = tone(RATE as usize * 2, 2, 220.0);
        let output = stretched(&input, 2, 2.0, 333);
        let frames = output.len() / 2;
        // Two seconds become one, less the windows still waiting for more input.
        assert!((7_400..=8_000).contains(&frames), "{frames}");
        let pitch = crossings_per_frame(&output, 2) / crossings_per_frame(&input, 2);
        assert!((0.97..=1.03).contains(&pitch), "{pitch}");
    }

    #[test]
    fn one_and_a_half_times_as_fast_keeps_the_level_and_the_pitch() {
        let input = tone(RATE as usize * 3, 1, 330.0);
        let output = stretched(&input, 1, 1.5, 1_000);
        assert!(
            (15_400..=16_000).contains(&output.len()),
            "{}",
            output.len()
        );
        let pitch = crossings_per_frame(&output, 1) / crossings_per_frame(&input, 1);
        assert!((0.97..=1.03).contains(&pitch), "{pitch}");
        let loudest = output
            .iter()
            .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()));
        assert!((0.45..=0.55).contains(&loudest), "{loudest}");
    }

    #[test]
    fn a_speed_change_goes_on_without_a_click_or_lost_sound() {
        let mut stretcher = TimeStretcher::new(1, RATE, 1.5);
        let input = tone(RATE as usize * 2, 1, 220.0);
        let (first, second) = input.split_at(input.len() / 2);
        let mut output = Vec::new();
        for part in first.chunks(160) {
            stretcher.process(part, &mut output);
        }
        let boundary = output.len();
        stretcher.set_speed(2.0);
        for part in second.chunks(160) {
            stretcher.process(part, &mut output);
        }
        let largest_step = |samples: &[f32]| {
            samples
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0_f32, f32::max)
        };
        let steady = largest_step(&output[400..boundary - 10]);
        let across = largest_step(&output[boundary - 5..boundary + 5]);
        assert!(across <= steady * 1.5, "{across} against {steady}");
        // One second at 1.5× then one at 2×, less the windows still waiting for input at the
        // end (a new stretcher at the change would lose as much again).
        let expected = RATE as usize * 2 / 3 + RATE as usize / 2;
        assert!(
            (expected - 400..=expected).contains(&output.len()),
            "{} against {expected}",
            output.len()
        );
    }

    #[test]
    fn the_input_held_stays_bounded() {
        let mut stretcher = TimeStretcher::new(1, RATE, 2.0);
        let mut output = Vec::new();
        for _ in 0..100 {
            stretcher.process(&tone(800, 1, 220.0), &mut output);
            assert!(stretcher.input.len() <= 2_000, "{}", stretcher.input.len());
        }
    }
}
