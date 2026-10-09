//! True-peak limiter and loudness normalisation built on [`crate::loudness`].
//!
//! The limiter is a lookahead design: the required gain is derived from the
//! 4x oversampled peak of every frame (all channels share one gain), held for
//! the lookahead window, release-smoothed and finally averaged over the
//! lookahead window. The averaging turns the held steps into linear ramps, so
//! the gain never changes abruptly and never exceeds the gain required by any
//! frame inside the window. The output is delayed internally and the delay is
//! compensated: the number of output frames equals the number of input
//! frames and every output frame lines up with its input frame.

use crate::loudness::{LoudnessMeter, LoudnessReport, TruePeakFilter, true_peak_oversampling};
use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::VecDeque;

/// Lookahead (attack) time of the limiter in milliseconds.
pub const LOOKAHEAD_MS: f64 = 2.0;
/// Release time constant of the limiter in milliseconds.
pub const RELEASE_MS: f64 = 100.0;
/// Default true-peak ceiling in dBTP.
pub const DEFAULT_CEILING_DBTP: f64 = -1.0;
/// Accepted deviation of the output from the loudness target in LU.
pub const LOUDNESS_TOLERANCE_LU: f64 = 0.03;
/// Maximum number of gain refinement passes when the limiter is active.
pub const MAX_REFINEMENTS: usize = 4;

fn db_to_linear(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

/// Streaming lookahead true-peak limiter for interleaved audio.
#[derive(Debug)]
pub struct TruePeakLimiter {
    channels: usize,
    ceiling: f64,
    lookahead: usize,
    release_coefficient: f64,
    filters: Vec<TruePeakFilter>,
    pending: VecDeque<f32>,
    // per-chunk scratch
    channel_buf: Vec<f32>,
    peak_buf: Vec<f32>,
    frame_peak: Vec<f32>,
    // gain computation state
    frames_in: u64,
    window_min: VecDeque<(u64, f64)>,
    released: f64,
    average_ring: Vec<f64>,
    average_sum: f64,
    held_count: u64,
    min_gain: f64,
}

impl TruePeakLimiter {
    pub fn new(sample_rate: u32, channels: u16, ceiling_dbtp: f64) -> Result<Self> {
        if channels == 0 || sample_rate == 0 {
            bail!("limiter needs a positive sample rate and channel count");
        }
        let rate = f64::from(sample_rate);
        let lookahead = ((LOOKAHEAD_MS / 1000.0 * rate).ceil() as usize).max(2);
        Ok(Self {
            channels: usize::from(channels),
            ceiling: db_to_linear(ceiling_dbtp),
            lookahead,
            release_coefficient: 1.0 - (-1.0 / (RELEASE_MS / 1000.0 * rate)).exp(),
            filters: vec![
                TruePeakFilter::new(true_peak_oversampling(sample_rate));
                usize::from(channels)
            ],
            pending: VecDeque::new(),
            channel_buf: Vec::new(),
            peak_buf: Vec::new(),
            frame_peak: Vec::new(),
            frames_in: 0,
            window_min: VecDeque::new(),
            released: 1.0,
            average_ring: vec![1.0; lookahead],
            average_sum: lookahead as f64,
            held_count: 0,
            min_gain: 1.0,
        })
    }

    /// Total latency in frames between an input frame and the moment its
    /// output is produced (compensated internally).
    pub fn latency_frames(&self) -> usize {
        TruePeakFilter::DELAY + self.lookahead - 1
    }

    /// Largest gain reduction applied so far, in dB (>= 0).
    pub fn max_gain_reduction_db(&self) -> f64 {
        -20.0 * self.min_gain.log10()
    }

    /// Process whole interleaved frames, appending the aligned output that is
    /// ready to `out` (nothing is produced until the lookahead has filled).
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let channels = self.channels;
        let frames = input.len() / channels;
        if frames == 0 {
            return;
        }
        self.pending.extend(&input[..frames * channels]);
        self.frame_peak.clear();
        self.frame_peak.resize(frames, 0.0);
        for channel in 0..channels {
            self.channel_buf.clear();
            self.channel_buf.extend(
                input[..frames * channels]
                    .iter()
                    .skip(channel)
                    .step_by(channels)
                    .copied(),
            );
            self.peak_buf.clear();
            self.filters[channel].run(&self.channel_buf, self.ceiling as f32, &mut self.peak_buf);
            for (peak, &value) in self.frame_peak.iter_mut().zip(&self.peak_buf) {
                *peak = peak.max(value);
            }
        }
        out.reserve(frames * channels);
        for index in 0..frames {
            let arrival = self.frames_in + index as u64;
            // The peak of frame `arrival - DELAY` is available now.
            if arrival >= TruePeakFilter::DELAY as u64 {
                let peak = f64::from(self.frame_peak[index]);
                self.advance(peak, out);
            }
        }
        self.frames_in += frames as u64;
    }

    /// Flush the remaining frames so that exactly as many frames have been
    /// produced as were consumed. The limiter must not be used afterwards.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        // Feeding `latency` frames of silence releases every pending real
        // frame; the padding itself stays queued and is discarded.
        let padding = vec![0.0_f32; self.latency_frames() * self.channels];
        for chunk in padding.chunks(4096 * self.channels) {
            self.process(chunk, out);
        }
        self.pending.clear();
    }

    fn advance(&mut self, peak: f64, out: &mut Vec<f32>) {
        let required = if peak > self.ceiling {
            self.ceiling / peak
        } else {
            1.0
        };
        let index = self.held_count;
        self.held_count += 1;

        // Minimum of the required gain over the last `lookahead` frames.
        while self.window_min.back().is_some_and(|&(_, v)| v >= required) {
            self.window_min.pop_back();
        }
        self.window_min.push_back((index, required));
        while self
            .window_min
            .front()
            .is_some_and(|&(i, _)| i + self.lookahead as u64 <= index)
        {
            self.window_min.pop_front();
        }
        let held = self.window_min.front().map_or(1.0, |&(_, v)| v);

        // Fast fall, exponential release.
        let recovering = self.released + (1.0 - self.released) * self.release_coefficient;
        self.released = held.min(recovering);
        if 1.0 - self.released < 1e-9 {
            self.released = 1.0;
        }

        // Moving average over the lookahead window.
        let slot = (index % self.lookahead as u64) as usize;
        self.average_sum += self.released - self.average_ring[slot];
        self.average_ring[slot] = self.released;

        // The average covers released[index - L + 1 ..= index]; it is the gain
        // for frame `index - (L - 1)`.
        if index + 1 >= self.lookahead as u64 {
            let gain = (self.average_sum / self.lookahead as f64).clamp(0.0, 1.0);
            self.min_gain = self.min_gain.min(gain);
            let gain = gain as f32;
            for _ in 0..self.channels {
                if let Some(sample) = self.pending.pop_front() {
                    out.push(sample * gain);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Loudness normalisation
// ---------------------------------------------------------------------------

/// What the normaliser should achieve.
#[derive(Debug, Clone, Copy)]
pub struct LoudnessGoal {
    /// Integrated loudness target in LUFS; `None` only limits peaks.
    pub target_lufs: Option<f64>,
    /// True-peak ceiling in dBTP.
    pub ceiling_dbtp: f64,
}

impl LoudnessGoal {
    pub fn validate(&self) -> Result<()> {
        if let Some(target) = self.target_lufs
            && !(-70.0..=0.0).contains(&target)
        {
            bail!("loudness target must be between -70 and 0 LUFS");
        }
        if !(-40.0..=0.0).contains(&self.ceiling_dbtp) {
            bail!("true-peak ceiling must be between -40 and 0 dBTP");
        }
        Ok(())
    }
}

/// Outcome of [`normalize`].
#[derive(Debug, Clone, Serialize)]
pub struct LoudnessOutcome {
    pub input: LoudnessReport,
    /// Present when the output was measured (always when limiting was needed).
    pub output: Option<LoudnessReport>,
    /// Linear gain applied before the limiter, in dB.
    pub gain_db: f64,
    pub limited: bool,
    pub limiter_gain_reduction_max_db: f64,
}

/// Receives the processed output as chunks of whole interleaved frames.
pub type Sink<'a> = dyn FnMut(&[f32]) -> Result<()> + 'a;

/// Replays the (unchanged) input as chunks of whole interleaved frames.
pub type Source<'a> = dyn FnMut(&mut dyn FnMut(&[f32]) -> Result<()>) -> Result<()> + 'a;

struct PassResult {
    reduction_db: f64,
    report: Option<LoudnessReport>,
}

/// One pass: input -> gain -> optional limiter -> optional meter / sink.
fn run_pass(
    source: &mut Source<'_>,
    sample_rate: u32,
    channels: u16,
    gain_db: f64,
    ceiling_dbtp: Option<f64>,
    measure: bool,
    mut sink: Option<&mut Sink<'_>>,
) -> Result<PassResult> {
    let gain = db_to_linear(gain_db) as f32;
    let mut limiter = ceiling_dbtp
        .map(|ceiling| TruePeakLimiter::new(sample_rate, channels, ceiling))
        .transpose()?;
    let mut meter = measure
        .then(|| LoudnessMeter::new(sample_rate, channels))
        .transpose()?;
    let mut scaled = Vec::new();
    let mut limited = Vec::new();

    let mut emit = |samples: &[f32]| -> Result<()> {
        if let Some(meter) = meter.as_mut() {
            meter.push(samples);
        }
        if let Some(sink) = sink.as_mut() {
            sink(samples)?;
        }
        Ok(())
    };

    source(&mut |chunk: &[f32]| {
        scaled.clear();
        scaled.extend(chunk.iter().map(|sample| sample * gain));
        match limiter.as_mut() {
            Some(limiter) => {
                limited.clear();
                limiter.process(&scaled, &mut limited);
                emit(&limited)
            }
            None => emit(&scaled),
        }
    })?;
    if let Some(limiter) = limiter.as_mut() {
        limited.clear();
        limiter.finish(&mut limited);
        emit(&limited)?;
    }
    Ok(PassResult {
        reduction_db: limiter
            .as_ref()
            .map_or(0.0, TruePeakLimiter::max_gain_reduction_db),
        report: meter.map(LoudnessMeter::finish),
    })
}

/// Measure, apply the gain needed for the goal and limit true peaks.
///
/// `source` must be replayable (each call streams the whole input). The final
/// output is delivered to `sink` in chunks of whole frames. When `want_output`
/// is set the output loudness is measured as well (this always happens when
/// the limiter is engaged, because its gain reduction lowers the loudness and
/// the gain is refined to compensate).
pub fn normalize(
    source: &mut Source<'_>,
    sample_rate: u32,
    channels: u16,
    goal: LoudnessGoal,
    want_output: bool,
    sink: &mut Sink<'_>,
) -> Result<LoudnessOutcome> {
    goal.validate()?;
    let input = run_pass(source, sample_rate, channels, 0.0, None, true, None)?
        .report
        .expect("measurement requested");

    let mut gain_db = match (goal.target_lufs, input.integrated_lufs) {
        (Some(target), Some(integrated)) => target - integrated,
        _ => 0.0,
    };
    let needs_limit = |gain_db: f64| {
        input
            .true_peak_dbtp
            .is_some_and(|peak| peak + gain_db > goal.ceiling_dbtp + 1e-9)
    };

    if !needs_limit(gain_db) {
        let result = run_pass(
            source,
            sample_rate,
            channels,
            gain_db,
            None,
            want_output,
            Some(sink),
        )?;
        return Ok(LoudnessOutcome {
            input,
            output: result.report,
            gain_db,
            limited: false,
            limiter_gain_reduction_max_db: 0.0,
        });
    }

    // The limiter lowers the loudness, so iterate: measure the limited
    // result and raise the gain by the shortfall.
    let mut output = None;
    let mut reduction = 0.0;
    if let Some(target) = goal.target_lufs {
        for attempt in 0..=MAX_REFINEMENTS {
            let trial = run_pass(
                source,
                sample_rate,
                channels,
                gain_db,
                Some(goal.ceiling_dbtp),
                true,
                None,
            )?;
            reduction = trial.reduction_db;
            output = trial.report;
            let Some(measured) = output.as_ref().and_then(|r| r.integrated_lufs) else {
                break;
            };
            let error = target - measured;
            if error.abs() <= LOUDNESS_TOLERANCE_LU || attempt == MAX_REFINEMENTS {
                break;
            }
            gain_db += error;
        }
    }
    let result = run_pass(
        source,
        sample_rate,
        channels,
        gain_db,
        Some(goal.ceiling_dbtp),
        want_output && output.is_none(),
        Some(sink),
    )?;
    if output.is_none() {
        output = result.report;
        reduction = result.reduction_db;
    }
    Ok(LoudnessOutcome {
        input,
        output,
        gain_db,
        limited: true,
        limiter_gain_reduction_max_db: reduction,
    })
}

/// In-memory convenience wrapper around [`normalize`]; replaces `samples`.
pub fn normalize_buffer(
    samples: &mut Vec<f32>,
    sample_rate: u32,
    channels: u16,
    goal: LoudnessGoal,
    want_output: bool,
) -> Result<LoudnessOutcome> {
    let frame_chunk = 16_384 * usize::from(channels);
    let mut output = Vec::with_capacity(samples.len());
    let outcome = {
        let original: &[f32] = samples;
        let mut source = |emit: &mut dyn FnMut(&[f32]) -> Result<()>| -> Result<()> {
            for chunk in original.chunks(frame_chunk) {
                emit(chunk)?;
            }
            Ok(())
        };
        let mut sink = |chunk: &[f32]| -> Result<()> {
            output.extend_from_slice(chunk);
            Ok(())
        };
        normalize(
            &mut source,
            sample_rate,
            channels,
            goal,
            want_output,
            &mut sink,
        )?
    };
    *samples = output;
    Ok(outcome)
}
