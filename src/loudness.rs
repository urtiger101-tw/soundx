//! ITU-R BS.1770-4 / EBU R128 loudness measurement.
//!
//! The meter is a streaming accumulator: feed interleaved `f32` samples with
//! [`LoudnessMeter::push`] in any chunk size and call [`LoudnessMeter::finish`]
//! to obtain integrated loudness (gated), loudness range (EBU Tech 3342),
//! momentary/short-term maxima, sample peak and true peak.
//!
//! Memory use is independent of the input length: block energies are stored in
//! fine histograms (0.01 LU bins) that are used for the gating and percentile
//! computations, so a ten hour file needs the same few hundred kilobytes as a
//! ten second one.
//!
//! K-weighting coefficients are derived from the analog prototypes of BS.1770
//! with the bilinear transform for the actual sample rate (the approach used
//! by libebur128), so any sample rate is supported and not only 48 kHz.

use anyhow::{Result, bail};
use serde::Serialize;

/// Human-readable standard name used in reports.
pub const STANDARD: &str = "ITU-R BS.1770-4 / EBU R128";
/// Absolute gate in LUFS.
pub const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Relative gate for integrated loudness in LU.
pub const RELATIVE_GATE_LU: f64 = -10.0;
/// Relative gate for the loudness range in LU.
pub const LRA_RELATIVE_GATE_LU: f64 = -20.0;
/// Gating block length in milliseconds.
pub const BLOCK_MS: u32 = 400;
/// Overlap of consecutive gating blocks.
pub const BLOCK_OVERLAP: f64 = 0.75;

const LOUDNESS_OFFSET: f64 = -0.691;
const SUB_BLOCKS_PER_BLOCK: usize = 4;
const SUB_BLOCKS_PER_SHORT_TERM: usize = 30;
const HISTOGRAM_STEP: f64 = 0.01;
const HISTOGRAM_BINS: usize = 8000; // -70 LUFS .. +10 LUFS
const CHUNK_FRAMES: usize = 8192;

/// Convert a mean-square energy (already channel weighted) to LUFS.
fn energy_to_lufs(energy: f64) -> f64 {
    if energy > 0.0 {
        LOUDNESS_OFFSET + 10.0 * energy.log10()
    } else {
        f64::NEG_INFINITY
    }
}

fn linear_to_db(value: f64) -> Option<f64> {
    (value > 0.0).then(|| 20.0 * value.log10())
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct BiquadState {
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    fn run(&self, state: &mut BiquadState, x: f64) -> f64 {
        let y = self.b0 * x + state.z1;
        state.z1 = self.b1 * x - self.a1 * y + state.z2;
        state.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// K-weighting filter pair for the given sample rate: the high-shelf
/// pre-filter followed by the RLB high-pass (BS.1770-4 Annex 1).
fn k_weighting(sample_rate: u32) -> [Biquad; 2] {
    let fs = f64::from(sample_rate);

    // Stage 1: head-related high shelf.
    let f0 = 1_681.974_450_955_533;
    let gain_db = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10.0_f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };

    // Stage 2: revised low-frequency B-curve (high-pass).
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let rlb = Biquad {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };
    [shelf, rlb]
}

/// Channel weights for the loudness sum. Surround channels count +1.5 dB
/// (1.41), the LFE channel is excluded. Layouts follow the WAV channel order.
pub fn channel_weights(channels: usize) -> Vec<f64> {
    const SURROUND: f64 = 1.41;
    match channels {
        4 => vec![1.0, 1.0, SURROUND, SURROUND],
        5 => vec![1.0, 1.0, 1.0, SURROUND, SURROUND],
        6 => vec![1.0, 1.0, 1.0, 0.0, SURROUND, SURROUND],
        8 => vec![1.0, 1.0, 1.0, 0.0, SURROUND, SURROUND, SURROUND, SURROUND],
        _ => vec![1.0; channels],
    }
}

// ---------------------------------------------------------------------------
// True peak
// ---------------------------------------------------------------------------

/// Taps per polyphase branch of the interpolation filter.
const TP_TAPS: usize = 48;
const TP_BLOCK: usize = 8;
const TP_KAISER_BETA: f64 = 7.0;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..60 {
        term *= q / f64::from(k * k);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Oversampling factor per BS.1770-4 Annex 2 (4x), reduced for high rates
/// where inter-sample overshoot is not a practical concern (as libebur128).
pub fn true_peak_oversampling(sample_rate: u32) -> usize {
    match sample_rate {
        0..=95_999 => 4,
        96_000..=191_999 => 2,
        _ => 1,
    }
}

/// Polyphase windowed-sinc interpolator that reports, for every input sample
/// `x[m]`, the largest absolute value among `x[m]` and the interpolated
/// points in `(m, m + 1)`. The result for `x[m]` is available once
/// `x[m + 24]` has been pushed, i.e. the output stream is delayed by
/// [`TruePeakFilter::DELAY`] samples.
#[derive(Debug, Clone)]
pub struct TruePeakFilter {
    phases: Vec<[f32; TP_TAPS]>,
    bound: f32,
    history: [f32; TP_TAPS],
    scratch: Vec<f32>,
    block_max: Vec<f32>,
}

impl TruePeakFilter {
    /// Delay in input samples between a sample and its peak value.
    pub const DELAY: usize = TP_TAPS / 2;

    /// Build a filter for an integer oversampling factor (1 disables
    /// interpolation).
    pub fn new(factor: usize) -> Self {
        let factor = factor.max(1);
        let half = (TP_TAPS / 2) as f64;
        let i0_beta = bessel_i0(TP_KAISER_BETA);
        let mut phases = Vec::new();
        let mut bound = 1.0_f64;
        for phase in 1..factor {
            let offset = phase as f64 / factor as f64;
            let mut taps = [0.0_f64; TP_TAPS];
            for (i, tap) in taps.iter_mut().enumerate() {
                // Coefficient of w[i] (w[47] is the newest sample; the output
                // point lies `offset` after the sample stored in w[23]).
                let u = (TP_TAPS / 2 - 1) as f64 - i as f64 + offset;
                let sinc = if u.abs() < 1e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u)
                };
                let ratio = u / half;
                let window =
                    bessel_i0(TP_KAISER_BETA * (1.0 - ratio * ratio).max(0.0).sqrt()) / i0_beta;
                *tap = sinc * window;
            }
            let sum: f64 = taps.iter().sum();
            let mut coefficients = [0.0_f32; TP_TAPS];
            for (target, tap) in coefficients.iter_mut().zip(taps) {
                *target = (tap / sum) as f32;
            }
            bound = bound.max(taps.iter().map(|tap| (tap / sum).abs()).sum());
            phases.push(coefficients);
        }
        Self {
            phases,
            bound: bound as f32 * 1.0001,
            history: [0.0; TP_TAPS],
            scratch: Vec::new(),
            block_max: Vec::new(),
        }
    }

    /// Push samples and append one peak value per input sample to `out`.
    ///
    /// Interpolation is skipped where a provable bound shows that no point
    /// can exceed `skip_at_or_below`; the reported value is then merely the
    /// sample magnitude, which is irrelevant to any maximum above that bound.
    pub fn run(&mut self, input: &[f32], skip_at_or_below: f32, out: &mut Vec<f32>) {
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.history);
        self.scratch.extend_from_slice(input);
        self.block_max.clear();
        self.block_max.extend(
            self.scratch
                .chunks(TP_BLOCK)
                .map(|block| block.iter().fold(0.0_f32, |max, v| max.max(v.abs()))),
        );
        out.reserve(input.len());
        for k in 0..input.len() {
            // The newest sample of the window is `input[k]`, i.e. scratch[k + TP_TAPS].
            let window = &self.scratch[k + 1..k + 1 + TP_TAPS];
            let mut peak = window[TP_TAPS / 2 - 1].abs();
            if !self.phases.is_empty() {
                let local = self.block_max[(k + 1) / TP_BLOCK..=(k + TP_TAPS) / TP_BLOCK]
                    .iter()
                    .fold(0.0_f32, |max, v| max.max(*v));
                if local * self.bound > skip_at_or_below {
                    for phase in &self.phases {
                        peak = peak.max(dot(phase, window).abs());
                    }
                }
            }
            out.push(peak);
        }
        let start = self.scratch.len() - TP_TAPS;
        self.history.copy_from_slice(&self.scratch[start..]);
    }
}

#[inline]
fn dot(coefficients: &[f32; TP_TAPS], window: &[f32]) -> f32 {
    let mut acc = [0.0_f32; 8];
    for (c, w) in coefficients.chunks_exact(8).zip(window.chunks_exact(8)) {
        for lane in 0..8 {
            acc[lane] += c[lane] * w[lane];
        }
    }
    acc.iter().sum()
}

// ---------------------------------------------------------------------------
// Histogram used for gating and percentiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Histogram {
    counts: Vec<u64>,
    energy: Vec<f64>,
}

impl Histogram {
    fn new() -> Self {
        Self {
            counts: vec![0; HISTOGRAM_BINS],
            energy: vec![0.0; HISTOGRAM_BINS],
        }
    }

    /// Add a block energy; blocks under the absolute gate are dropped.
    fn add(&mut self, energy: f64) {
        let loudness = energy_to_lufs(energy);
        if loudness >= ABSOLUTE_GATE_LUFS {
            let bin = ((loudness - ABSOLUTE_GATE_LUFS) / HISTOGRAM_STEP) as usize;
            let bin = bin.min(HISTOGRAM_BINS - 1);
            self.counts[bin] += 1;
            self.energy[bin] += energy;
        }
    }

    /// Bin index whose lower edge is at or above `lufs`.
    fn bin_at_or_above(lufs: f64) -> usize {
        let bin = ((lufs - ABSOLUTE_GATE_LUFS) / HISTOGRAM_STEP).ceil();
        if bin <= 0.0 {
            0
        } else {
            (bin as usize).min(HISTOGRAM_BINS)
        }
    }

    /// Sum of energies and number of blocks in bins `from..`.
    fn tail(&self, from: usize) -> (f64, u64) {
        let from = from.min(HISTOGRAM_BINS);
        (
            self.energy[from..].iter().sum(),
            self.counts[from..].iter().sum(),
        )
    }

    /// Mean energy of the blocks above the absolute gate and `relative_lu`
    /// below that mean's loudness.
    fn gated_mean(&self, relative_lu: f64) -> Option<(f64, u64)> {
        let (sum, count) = self.tail(0);
        if count == 0 {
            return None;
        }
        let threshold = energy_to_lufs(sum / count as f64) + relative_lu;
        let (gated_sum, gated_count) = self.tail(Self::bin_at_or_above(threshold));
        if gated_count == 0 {
            Some((sum, count))
        } else {
            Some((gated_sum, gated_count))
        }
    }

    /// Loudness of the `rank`-th (0-based) block counted from bin `from`.
    fn value_at_rank(&self, from: usize, rank: u64) -> f64 {
        let mut seen = 0;
        for bin in from..HISTOGRAM_BINS {
            seen += self.counts[bin];
            if seen > rank {
                return energy_to_lufs(self.energy[bin] / self.counts[bin] as f64);
            }
        }
        f64::NEG_INFINITY
    }
}

// ---------------------------------------------------------------------------
// Meter
// ---------------------------------------------------------------------------

/// Result of a loudness measurement.
#[derive(Debug, Clone, Serialize)]
pub struct LoudnessReport {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub duration_seconds: f64,
    /// Gated integrated loudness; `None` when nothing exceeds the -70 LUFS gate.
    pub integrated_lufs: Option<f64>,
    /// EBU Tech 3342 loudness range; 0 when there are no short-term values.
    pub loudness_range_lu: f64,
    pub true_peak_dbtp: Option<f64>,
    pub sample_peak_dbfs: Option<f64>,
    pub momentary_max_lufs: Option<f64>,
    pub short_term_max_lufs: Option<f64>,
}

/// Streaming BS.1770-4 / EBU R128 meter.
#[derive(Debug, Clone)]
pub struct LoudnessMeter {
    sample_rate: u32,
    channels: usize,
    weights: Vec<f64>,
    k_filters: [Biquad; 2],
    states: Vec<[BiquadState; 2]>,
    true_peak: Vec<TruePeakFilter>,
    true_peak_max: f32,
    sample_peak: f32,
    sub_len: usize,
    sub_pos: usize,
    sub_energy: f64,
    sub_ring: [f64; SUB_BLOCKS_PER_SHORT_TERM],
    sub_count: u64,
    gating: Histogram,
    short_term: Histogram,
    momentary_max: f64,
    short_term_max: f64,
    frames: u64,
    carry: Vec<f32>,
    // scratch buffers
    frame_energy: Vec<f64>,
    channel_buf: Vec<f32>,
    peak_buf: Vec<f32>,
}

impl LoudnessMeter {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if sample_rate < 1000 {
            bail!("loudness measurement needs a sample rate of at least 1000 Hz");
        }
        if channels == 0 {
            bail!("loudness measurement needs at least one channel");
        }
        let count = usize::from(channels);
        let factor = true_peak_oversampling(sample_rate);
        Ok(Self {
            sample_rate,
            channels: count,
            weights: channel_weights(count),
            k_filters: k_weighting(sample_rate),
            states: vec![[BiquadState::default(); 2]; count],
            true_peak: vec![TruePeakFilter::new(factor); count],
            true_peak_max: 0.0,
            sample_peak: 0.0,
            sub_len: (sample_rate as usize).div_ceil(10).max(1),
            sub_pos: 0,
            sub_energy: 0.0,
            sub_ring: [0.0; SUB_BLOCKS_PER_SHORT_TERM],
            sub_count: 0,
            gating: Histogram::new(),
            short_term: Histogram::new(),
            momentary_max: 0.0,
            short_term_max: 0.0,
            frames: 0,
            carry: Vec::new(),
            frame_energy: Vec::new(),
            channel_buf: Vec::new(),
            peak_buf: Vec::new(),
        })
    }

    /// Feed interleaved samples. Chunks need not be multiples of the channel
    /// count; a trailing partial frame is completed by the next call.
    pub fn push(&mut self, interleaved: &[f32]) {
        let channels = self.channels;
        let mut data = interleaved;
        if !self.carry.is_empty() {
            let take = (channels - self.carry.len()).min(data.len());
            self.carry.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.carry.len() < channels {
                return;
            }
            let frame = std::mem::take(&mut self.carry);
            self.process(&frame);
        }
        let whole = data.len() / channels * channels;
        for chunk in data[..whole].chunks(CHUNK_FRAMES * channels) {
            self.process(chunk);
        }
        self.carry.extend_from_slice(&data[whole..]);
    }

    fn process(&mut self, chunk: &[f32]) {
        let channels = self.channels;
        let frames = chunk.len() / channels;
        self.frame_energy.clear();
        self.frame_energy.resize(frames, 0.0);
        let [shelf, rlb] = self.k_filters;
        for channel in 0..channels {
            self.channel_buf.clear();
            self.channel_buf
                .extend(chunk.iter().skip(channel).step_by(channels).copied());
            self.sample_peak = self
                .channel_buf
                .iter()
                .fold(self.sample_peak, |max, v| max.max(v.abs()));
            let weight = self.weights[channel];
            if weight > 0.0 {
                let [shelf_state, rlb_state] = &mut self.states[channel];
                for (energy, &x) in self.frame_energy.iter_mut().zip(&self.channel_buf) {
                    let y = rlb.run(rlb_state, shelf.run(shelf_state, f64::from(x)));
                    *energy += weight * y * y;
                }
            }
            self.peak_buf.clear();
            self.true_peak[channel].run(&self.channel_buf, self.true_peak_max, &mut self.peak_buf);
            self.true_peak_max = self
                .peak_buf
                .iter()
                .fold(self.true_peak_max, |m, v| m.max(*v));
        }
        for index in 0..frames {
            self.sub_energy += self.frame_energy[index];
            self.sub_pos += 1;
            if self.sub_pos == self.sub_len {
                let energy = self.sub_energy / self.sub_len as f64;
                self.sub_pos = 0;
                self.sub_energy = 0.0;
                self.finish_sub_block(energy);
            }
        }
        self.frames += frames as u64;
    }

    fn window_energy(&self, blocks: usize) -> f64 {
        let newest = self.sub_count - 1;
        (0..blocks as u64)
            .map(|back| {
                self.sub_ring[((newest - back) % SUB_BLOCKS_PER_SHORT_TERM as u64) as usize]
            })
            .sum::<f64>()
            / blocks as f64
    }

    fn finish_sub_block(&mut self, energy: f64) {
        self.sub_ring[(self.sub_count % SUB_BLOCKS_PER_SHORT_TERM as u64) as usize] = energy;
        self.sub_count += 1;
        if self.sub_count >= SUB_BLOCKS_PER_BLOCK as u64 {
            let momentary = self.window_energy(SUB_BLOCKS_PER_BLOCK);
            self.gating.add(momentary);
            self.momentary_max = self.momentary_max.max(momentary);
        }
        if self.sub_count >= SUB_BLOCKS_PER_SHORT_TERM as u64 {
            let short_term = self.window_energy(SUB_BLOCKS_PER_SHORT_TERM);
            self.short_term.add(short_term);
            self.short_term_max = self.short_term_max.max(short_term);
        }
    }

    /// Finish the measurement and produce the report.
    pub fn finish(mut self) -> LoudnessReport {
        // Drain the true-peak filters (excluding the interval after the
        // final sample).
        let tail = [0.0_f32; TruePeakFilter::DELAY - 1];
        for filter in &mut self.true_peak {
            self.peak_buf.clear();
            filter.run(&tail, self.true_peak_max, &mut self.peak_buf);
            self.true_peak_max = self
                .peak_buf
                .iter()
                .fold(self.true_peak_max, |m, v| m.max(*v));
        }
        let true_peak = self.true_peak_max.max(self.sample_peak);

        let integrated_lufs = self
            .gating
            .gated_mean(RELATIVE_GATE_LU)
            .map(|(sum, count)| energy_to_lufs(sum / count as f64));

        let loudness_range_lu = self.loudness_range();

        LoudnessReport {
            sample_rate: self.sample_rate,
            channels: self.channels as u16,
            frames: self.frames,
            duration_seconds: self.frames as f64 / f64::from(self.sample_rate),
            integrated_lufs,
            loudness_range_lu,
            true_peak_dbtp: linear_to_db(f64::from(true_peak)),
            sample_peak_dbfs: linear_to_db(f64::from(self.sample_peak)),
            momentary_max_lufs: finite(energy_to_lufs(self.momentary_max)),
            short_term_max_lufs: finite(energy_to_lufs(self.short_term_max)),
        }
    }

    fn loudness_range(&self) -> f64 {
        let histogram = &self.short_term;
        let (sum, count) = histogram.tail(0);
        if count == 0 {
            return 0.0;
        }
        let threshold = energy_to_lufs(sum / count as f64) + LRA_RELATIVE_GATE_LU;
        let from = Histogram::bin_at_or_above(threshold);
        let (_, gated) = histogram.tail(from);
        if gated == 0 {
            return 0.0;
        }
        let low = ((gated - 1) as f64 * 0.10 + 0.5) as u64;
        let high = ((gated - 1) as f64 * 0.95 + 0.5) as u64;
        let low = histogram.value_at_rank(from, low);
        let high = histogram.value_at_rank(from, high);
        (high - low).max(0.0)
    }
}

/// Gating parameters echoed in JSON reports.
#[derive(Debug, Clone, Serialize)]
pub struct GatingInfo {
    pub absolute_lufs: f64,
    pub relative_lu: f64,
    pub block_ms: u32,
    pub overlap: f64,
}

/// Per-file report as printed by `soundx loudness`.
#[derive(Debug, Clone, Serialize)]
pub struct FileReport {
    pub path: std::path::PathBuf,
    #[serde(flatten)]
    pub report: LoudnessReport,
    pub standard: &'static str,
    pub gating: GatingInfo,
}

impl FileReport {
    pub fn new(path: &std::path::Path, report: LoudnessReport) -> Self {
        Self {
            path: path.to_path_buf(),
            report,
            standard: STANDARD,
            gating: GatingInfo {
                absolute_lufs: ABSOLUTE_GATE_LUFS,
                relative_lu: RELATIVE_GATE_LU,
                block_ms: BLOCK_MS,
                overlap: BLOCK_OVERLAP,
            },
        }
    }
}

impl std::fmt::Display for FileReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let report = &self.report;
        let show = |value: Option<f64>, unit: &str| match value {
            Some(value) => format!("{value:.1} {unit}"),
            None => "n/a".to_string(),
        };
        writeln!(f, "File: {}", self.path.display())?;
        writeln!(f, "Standard: {}", self.standard)?;
        writeln!(f, "Channels: {}", report.channels)?;
        writeln!(f, "Sample Rate: {} Hz", report.sample_rate)?;
        writeln!(f, "Duration: {:.3} s", report.duration_seconds)?;
        writeln!(
            f,
            "Integrated Loudness: {}",
            show(report.integrated_lufs, "LUFS")
        )?;
        writeln!(f, "Loudness Range: {:.1} LU", report.loudness_range_lu)?;
        writeln!(f, "True Peak: {}", show(report.true_peak_dbtp, "dBTP"))?;
        writeln!(f, "Sample Peak: {}", show(report.sample_peak_dbfs, "dBFS"))?;
        writeln!(
            f,
            "Momentary Max: {}",
            show(report.momentary_max_lufs, "LUFS")
        )?;
        write!(
            f,
            "Short-term Max: {}",
            show(report.short_term_max_lufs, "LUFS")
        )
    }
}

/// Measure an interleaved in-memory buffer.
pub fn measure(samples: &[f32], sample_rate: u32, channels: u16) -> Result<LoudnessReport> {
    let mut meter = LoudnessMeter::new(sample_rate, channels)?;
    meter.push(samples);
    Ok(meter.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_weighting_matches_bs1770_reference_at_48khz() {
        let [shelf, rlb] = k_weighting(48_000);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(shelf.b0, 1.53512485958697));
        assert!(close(shelf.b1, -2.69169618940638));
        assert!(close(shelf.b2, 1.19839281085285));
        assert!(close(shelf.a1, -1.69065929318241));
        assert!(close(shelf.a2, 0.73248077421585));
        assert!(close(rlb.a1, -1.99004745483398));
        assert!(close(rlb.a2, 0.99007225036621));
    }

    #[test]
    fn interpolation_filter_has_unity_dc_gain_and_enough_taps() {
        let filter = TruePeakFilter::new(4);
        assert_eq!(filter.phases.len(), 3);
        for phase in &filter.phases {
            assert!(phase.len() >= 48);
            let sum: f32 = phase.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn chunking_does_not_change_the_result() {
        let samples: Vec<f32> = (0..96_000 * 2)
            .map(|i| ((i / 2) as f32 * 0.05).sin() * 0.3)
            .collect();
        let whole = measure(&samples, 48_000, 2).unwrap();
        let mut meter = LoudnessMeter::new(48_000, 2).unwrap();
        for chunk in samples.chunks(1001) {
            meter.push(chunk);
        }
        let chunked = meter.finish();
        assert_eq!(whole.integrated_lufs, chunked.integrated_lufs);
        assert_eq!(whole.true_peak_dbtp, chunked.true_peak_dbtp);
        assert_eq!(whole.frames, chunked.frames);
    }
}
