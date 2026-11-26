use crate::audio::AudioBuffer;
use crate::util::seconds_to_frames;
use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy)]
pub enum BiquadKind {
    LowPass,
    HighPass,
    BandPass,
    BandPassConstantSkirt,
    BandReject,
    AllPass,
    LowShelf,
    HighShelf,
    Equalizer,
}

#[derive(Debug, Clone, Copy)]
pub enum FilterWidth {
    Hertz(f32),
    Kilohertz(f32),
    HertzNoWarp(f32),
    Octaves(f32),
    Q(f32),
    Slope(f32),
}

#[derive(Debug, Clone, Copy)]
pub enum TempoQuality {
    Quick,
    Music,
    Speech,
    Linear,
}

#[derive(Debug, Clone, Copy)]
struct ReverbParameters {
    wet_only: bool,
    reverberance: f32,
    hf_damping: f32,
    room_scale: f32,
    stereo_depth: f32,
    pre_delay_ms: f32,
    wet_gain_db: f32,
}

#[derive(Debug, Clone)]
pub enum Effect {
    GainDb(f32),
    Normalize {
        target_db: f32,
    },
    Trim {
        start_sec: f32,
        duration_sec: Option<f32>,
    },
    Fade {
        in_sec: f32,
        out_sec: f32,
    },
    Reverse,
    Speed {
        factor: f32,
    },
    Stretch {
        factor: f32,
        window_ms: f32,
        search_ms: f32,
        overlap_ms: f32,
    },
    Tempo {
        factor: f32,
        quality: TempoQuality,
        segment_ms: f32,
        search_ms: f32,
        overlap_ms: f32,
    },
    Dither {
        bits: u16,
    },
    Compand {
        attack_decay: Vec<(f32, f32)>,
        transfer_points: Vec<(f32, f32)>,
        knee_db: f32,
        gain_db: f32,
        initial_volume_db: f32,
        delay_sec: f32,
    },
    Reverb {
        wet_only: bool,
        reverberance: f32,
        hf_damping: f32,
        room_scale: f32,
        stereo_depth: f32,
        pre_delay_ms: f32,
        wet_gain_db: f32,
    },
    Pad {
        start_sec: f32,
        end_sec: f32,
    },
    Silence {
        threshold_db: f32,
        min_duration_sec: f32,
    },
    LowPass {
        hz: f32,
    },
    HighPass {
        hz: f32,
    },
    BiquadFilter {
        kind: BiquadKind,
        hz: f32,
        width: FilterWidth,
        gain_db: f32,
        poles: u8,
    },
    Echo {
        input_gain: f32,
        output_gain: f32,
        taps_ms: Vec<(f32, f32)>,
    },
    Tremolo {
        speed_hz: f32,
        depth_percent: f32,
    },
    Delay {
        delays_sec: Vec<f32>,
    },
    DcShift {
        shift: f32,
        limiter_gain: Option<f32>,
    },
    Downsample {
        factor: u32,
    },
    Upsample {
        factor: u32,
    },
    Repeat {
        count: u32,
    },
    Swap,
    Limiter {
        threshold: f32,
    },
    Rate {
        sample_rate: u32,
    },
    Channels {
        channels: u16,
    },
    Stats,
}

#[derive(Debug, Clone)]
pub struct EffectChain {
    effects: Vec<Effect>,
}

impl EffectChain {
    pub fn new(effects: Vec<Effect>) -> Self {
        Self { effects }
    }

    /// Append an effect and return self (builder-style).
    #[allow(dead_code)]
    pub fn with(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    /// Append effects from an iterator and return self (builder-style).
    #[allow(dead_code)]
    pub fn extend(mut self, effects: impl IntoIterator<Item = Effect>) -> Self {
        self.effects.extend(effects);
        self
    }

    pub fn apply(&self, audio: &mut AudioBuffer) -> Result<()> {
        for effect in &self.effects {
            match effect {
                Effect::GainDb(db) => gain_db(audio, *db)?,
                Effect::Normalize { target_db } => normalize(audio, *target_db)?,
                Effect::Trim {
                    start_sec,
                    duration_sec,
                } => trim(audio, *start_sec, *duration_sec)?,
                Effect::Fade { in_sec, out_sec } => fade(audio, *in_sec, *out_sec)?,
                Effect::Reverse => reverse(audio),
                Effect::Speed { factor } => speed(audio, *factor)?,
                Effect::Stretch {
                    factor,
                    window_ms,
                    search_ms,
                    overlap_ms,
                } => wsola(audio, *factor, *window_ms, *search_ms, *overlap_ms)?,
                Effect::Tempo {
                    factor,
                    quality,
                    segment_ms,
                    search_ms,
                    overlap_ms,
                } => {
                    let tuned_segment = match quality {
                        TempoQuality::Quick => (*segment_ms).min(40.0),
                        TempoQuality::Music => *segment_ms,
                        TempoQuality::Speech => (*segment_ms).min(60.0),
                        TempoQuality::Linear => (*segment_ms).min(30.0),
                    };
                    wsola(audio, 1.0 / *factor, tuned_segment, *search_ms, *overlap_ms)?;
                }
                Effect::Dither { bits } => dither(audio, *bits)?,
                Effect::Compand {
                    attack_decay,
                    transfer_points,
                    knee_db,
                    gain_db,
                    initial_volume_db,
                    delay_sec,
                } => compand(
                    audio,
                    attack_decay,
                    transfer_points,
                    *knee_db,
                    *gain_db,
                    *initial_volume_db,
                    *delay_sec,
                )?,
                Effect::Reverb {
                    wet_only,
                    reverberance,
                    hf_damping,
                    room_scale,
                    stereo_depth,
                    pre_delay_ms,
                    wet_gain_db,
                } => reverb(
                    audio,
                    ReverbParameters {
                        wet_only: *wet_only,
                        reverberance: *reverberance,
                        hf_damping: *hf_damping,
                        room_scale: *room_scale,
                        stereo_depth: *stereo_depth,
                        pre_delay_ms: *pre_delay_ms,
                        wet_gain_db: *wet_gain_db,
                    },
                )?,
                Effect::Pad { start_sec, end_sec } => pad(audio, *start_sec, *end_sec)?,
                Effect::Silence {
                    threshold_db,
                    min_duration_sec,
                } => trim_silence(audio, *threshold_db, *min_duration_sec)?,
                Effect::LowPass { hz } => lowpass(audio, *hz)?,
                Effect::HighPass { hz } => highpass(audio, *hz)?,
                Effect::BiquadFilter {
                    kind,
                    hz,
                    width,
                    gain_db,
                    poles,
                } => biquad_filter(audio, *kind, *hz, *width, *gain_db, *poles)?,
                Effect::Echo {
                    input_gain,
                    output_gain,
                    taps_ms,
                } => echo(audio, *input_gain, *output_gain, taps_ms)?,
                Effect::Tremolo {
                    speed_hz,
                    depth_percent,
                } => tremolo(audio, *speed_hz, *depth_percent)?,
                Effect::Delay { delays_sec } => delay(audio, delays_sec)?,
                Effect::DcShift {
                    shift,
                    limiter_gain,
                } => dcshift(audio, *shift, *limiter_gain)?,
                Effect::Downsample { factor } => downsample(audio, *factor)?,
                Effect::Upsample { factor } => upsample(audio, *factor)?,
                Effect::Repeat { count } => repeat(audio, *count)?,
                Effect::Swap => swap(audio)?,
                Effect::Limiter { threshold } => limiter(audio, *threshold)?,
                Effect::Rate { sample_rate } => rate(audio, *sample_rate)?,
                Effect::Channels { channels } => convert_channels(audio, *channels)?,
                Effect::Stats => {}
            }
        }
        Ok(())
    }

    pub fn wants_stats(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| matches!(effect, Effect::Stats))
    }
}

fn gain_db(audio: &mut AudioBuffer, db: f32) -> Result<()> {
    if !db.is_finite() {
        bail!("gain must be a finite number of dB (got {db})");
    }
    let factor = 10.0_f32.powf(db / 20.0);
    if !factor.is_finite() {
        bail!("gain of {db} dB is too large");
    }
    for sample in &mut audio.samples {
        *sample *= factor;
    }
    Ok(())
}

fn normalize(audio: &mut AudioBuffer, target_db: f32) -> Result<()> {
    if !target_db.is_finite() {
        bail!("normalize target must be a finite number of dB (got {target_db})");
    }
    let peak = audio.peak();
    if peak == 0.0 {
        return Ok(());
    }
    if !peak.is_finite() {
        bail!("normalize requires finite samples");
    }
    let target = 10.0_f32.powf(target_db / 20.0);
    let factor = target / peak;
    if !factor.is_finite() {
        bail!("normalize target of {target_db} dB is too large");
    }
    for sample in &mut audio.samples {
        *sample *= factor;
    }
    Ok(())
}

/// Require a finite, non-negative duration in seconds.
fn ensure_duration(value: f32, what: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        bail!("{what} must be a finite number of seconds >= 0 (got {value})");
    }
    Ok(())
}

fn dither(audio: &mut AudioBuffer, bits: u16) -> Result<()> {
    if !(2..=32).contains(&bits) {
        bail!("dither precision must be between 2 and 32 bits");
    }
    let scale = 2_f64.powi(i32::from(bits) - 1);
    let max = scale - 1.0;
    let mut random = 0x9e37_79b9_7f4a_7c15_u64;
    for sample in &mut audio.samples {
        if !sample.is_finite() {
            bail!("dither requires finite samples");
        }
        let first = next_noise(&mut random);
        let second = next_noise(&mut random);
        let quantized = (f64::from(*sample).clamp(-1.0, 1.0) * scale + first - second)
            .round()
            .clamp(-scale, max);
        *sample = (quantized / scale) as f32;
    }
    Ok(())
}

fn next_noise(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as f64) / (u64::MAX as f64)
}

fn compand(
    audio: &mut AudioBuffer,
    attack_decay: &[(f32, f32)],
    transfer_points: &[(f32, f32)],
    knee_db: f32,
    gain_db: f32,
    initial_volume_db: f32,
    delay_sec: f32,
) -> Result<()> {
    let channels = usize::from(audio.spec.channels);
    if channels == 0 || audio.spec.sample_rate == 0 {
        bail!("compand requires non-zero sample rate and channel count");
    }
    if attack_decay.is_empty()
        || transfer_points.len() < 2
        || !knee_db.is_finite()
        || knee_db < 0.0
        || !gain_db.is_finite()
        || !initial_volume_db.is_finite()
        || !delay_sec.is_finite()
        || delay_sec < 0.0
    {
        bail!("invalid compand parameters");
    }
    if attack_decay.iter().any(|(attack, decay)| {
        !attack.is_finite() || !decay.is_finite() || *attack < 0.0 || *decay < 0.0
    }) {
        bail!("compand attack and decay times must be finite and non-negative");
    }
    for pair in transfer_points.windows(2) {
        if pair[0].0 >= pair[1].0 || pair.iter().any(|point| point.1.is_nan()) {
            bail!("compand input transfer levels must be strictly increasing");
        }
    }
    if transfer_points
        .iter()
        .any(|(input, output)| input.is_nan() || output.is_nan())
    {
        bail!("compand transfer points cannot contain NaN");
    }

    let frames = audio.frames();
    let delay_frames = (f64::from(delay_sec) * f64::from(audio.spec.sample_rate))
        .round()
        .min(usize::MAX as f64) as usize;
    let mut envelope = vec![0.0_f32; audio.samples.len()];
    let mut state = vec![10.0_f32.powf(initial_volume_db / 20.0); channels];
    for frame in 0..frames {
        for channel in 0..channels {
            let index = frame * channels + channel;
            let input_level = audio.samples[index].abs();
            let time = if input_level > state[channel] {
                attack_decay[channel.min(attack_decay.len() - 1)].0
            } else {
                attack_decay[channel.min(attack_decay.len() - 1)].1
            };
            let coefficient = if time <= f32::EPSILON {
                0.0
            } else {
                (-1.0 / (time * audio.spec.sample_rate as f32)).exp()
            };
            state[channel] = input_level + coefficient * (state[channel] - input_level);
            envelope[index] = state[channel].max(1.0e-10);
        }
    }

    for frame in 0..frames {
        let control_frame = frame
            .saturating_add(delay_frames)
            .min(frames.saturating_sub(1));
        for channel in 0..channels {
            let index = frame * channels + channel;
            let level_db = 20.0 * envelope[control_frame * channels + channel].log10();
            let output_db = compand_curve(level_db, transfer_points, knee_db) + gain_db;
            let amplitude_gain = 10.0_f32.powf((output_db - level_db) / 20.0);
            audio.samples[index] *= amplitude_gain;
        }
    }
    Ok(())
}

fn compand_curve(level_db: f32, points: &[(f32, f32)], knee_db: f32) -> f32 {
    let segment = points
        .windows(2)
        .position(|pair| level_db <= pair[1].0)
        .unwrap_or(points.len() - 2);
    let (x0, y0) = points[segment];
    let (x1, y1) = points[segment + 1];
    let slope = compand_slope(x0, y0, x1, y1);
    let mapped = if x0 == f32::NEG_INFINITY && y0 == f32::NEG_INFINITY {
        if x1.is_finite() {
            level_db + (y1 - x1)
        } else {
            level_db
        }
    } else if x1 == f32::INFINITY && y1 == f32::INFINITY {
        if x0.is_finite() {
            level_db + (y0 - x0)
        } else {
            level_db
        }
    } else if level_db <= x0 {
        if x0.is_finite() {
            y0 + slope * (level_db - x0)
        } else {
            y0
        }
    } else if level_db >= x1 {
        if x1.is_finite() {
            y1 + slope * (level_db - x1)
        } else {
            y1
        }
    } else if x0.is_finite() && x1.is_finite() {
        y0 + (level_db - x0) * slope
    } else if x1.is_finite() {
        y1
    } else {
        y0
    };
    if knee_db <= 0.0 || !level_db.is_finite() {
        return mapped;
    }
    for window in points.windows(3) {
        let knee_start = window[1].0 - knee_db * 0.5;
        let knee_end = window[1].0 + knee_db * 0.5;
        if level_db >= knee_start && level_db <= knee_end {
            let t = (level_db - knee_start) / knee_db;
            let smooth = t * t * (3.0 - 2.0 * t);
            let left_slope = compand_slope(window[0].0, window[0].1, window[1].0, window[1].1);
            let right_slope = compand_slope(window[1].0, window[1].1, window[2].0, window[2].1);
            let left = window[1].1 + left_slope * (level_db - window[1].0);
            let right = window[1].1 + right_slope * (level_db - window[1].0);
            return left * (1.0 - smooth) + right * smooth;
        }
    }
    mapped
}

fn compand_slope(x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    if x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite() {
        (y1 - y0) / (x1 - x0).max(f32::EPSILON)
    } else if (x0 == f32::NEG_INFINITY && y0 == f32::NEG_INFINITY)
        || (x1 == f32::INFINITY && y1 == f32::INFINITY)
    {
        1.0
    } else {
        0.0
    }
}

#[derive(Debug)]
struct CombFilter {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
    damping: f32,
    filter_store: f32,
}

impl CombFilter {
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filter_store = output * (1.0 - self.damping) + self.filter_store * self.damping;
        self.buffer[self.index] = input + self.filter_store * self.feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }
}

#[derive(Debug)]
struct AllPassFilter {
    buffer: Vec<f32>,
    index: usize,
}

impl AllPassFilter {
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = buffered - input;
        self.buffer[self.index] = input + buffered * 0.5;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }
}

fn reverb(audio: &mut AudioBuffer, params: ReverbParameters) -> Result<()> {
    let ReverbParameters {
        wet_only,
        reverberance,
        hf_damping,
        room_scale,
        stereo_depth,
        pre_delay_ms,
        wet_gain_db,
    } = params;
    if audio.spec.sample_rate == 0 || audio.spec.channels == 0 {
        bail!("reverb requires non-zero sample rate and channel count");
    }
    if audio.samples.is_empty() {
        return Ok(());
    }
    if [reverberance, hf_damping, room_scale, stereo_depth]
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=100.0).contains(value))
        || !pre_delay_ms.is_finite()
        || !(0.0..=500.0).contains(&pre_delay_ms)
        || !wet_gain_db.is_finite()
    {
        bail!("reverb parameters are outside their supported ranges");
    }
    let channels = usize::from(audio.spec.channels);
    let input_frames = audio.frames();
    let rate = audio.spec.sample_rate as f32;
    let tail_seconds = (0.25 + 2.5 * room_scale / 100.0 * reverberance / 100.0).clamp(0.25, 3.0);
    let tail_frames = (tail_seconds * rate).round() as usize;
    let predelay = (pre_delay_ms * rate / 1000.0).round() as usize;
    let total_frames = input_frames
        .checked_add(predelay)
        .and_then(|frames| frames.checked_add(tail_frames))
        .ok_or_else(|| anyhow::anyhow!("reverb output size overflow"))?;
    let total_samples = total_frames
        .checked_mul(channels)
        .ok_or_else(|| anyhow::anyhow!("reverb output size overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(total_samples)?;
    output.resize(total_samples, 0.0);
    let wet_gain = 10.0_f32.powf(wet_gain_db / 20.0);
    let feedback = 0.2 + 0.007 * reverberance;
    let damping = hf_damping / 100.0 * 0.72;
    let room_scale = 0.5 + room_scale / 200.0;
    const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
    let mut combs: Vec<Vec<CombFilter>> = Vec::with_capacity(channels);
    let mut allpasses: Vec<Vec<AllPassFilter>> = Vec::with_capacity(channels);
    for channel in 0..channels {
        let side = if channel % 2 == 1 {
            stereo_depth / 100.0
        } else {
            0.0
        };
        let spread = side * 23.0 * rate / 44_100.0;
        combs.push(
            COMB_TUNINGS
                .iter()
                .map(|tuning| {
                    let len = ((*tuning as f32 * rate / 44_100.0 * room_scale + spread).round()
                        as usize)
                        .clamp(1, (audio.spec.sample_rate as usize * 2).max(1));
                    CombFilter {
                        buffer: vec![0.0; len],
                        index: 0,
                        feedback: feedback.min(0.9),
                        damping,
                        filter_store: 0.0,
                    }
                })
                .collect(),
        );
        allpasses.push(
            ALLPASS_TUNINGS
                .iter()
                .map(|tuning| {
                    let len = ((*tuning as f32 * rate / 44_100.0 * room_scale + spread).round()
                        as usize)
                        .clamp(1, (audio.spec.sample_rate as usize).max(1));
                    AllPassFilter {
                        buffer: vec![0.0; len],
                        index: 0,
                    }
                })
                .collect(),
        );
    }

    for frame in 0..total_frames {
        for channel in 0..channels {
            let source_frame = frame.checked_sub(predelay);
            let dry = source_frame
                .filter(|source_frame| *source_frame < input_frames)
                .map(|source_frame| audio.samples[source_frame * channels + channel])
                .unwrap_or(0.0);
            let mut wet = 0.0;
            for comb in &mut combs[channel] {
                wet += comb.process(dry * 0.015);
            }
            wet /= COMB_TUNINGS.len() as f32;
            for allpass in &mut allpasses[channel] {
                wet = allpass.process(wet);
            }
            let audible_dry = if frame < input_frames {
                audio.samples[frame * channels + channel]
            } else {
                0.0
            };
            output[frame * channels + channel] =
                wet * wet_gain + if wet_only { 0.0 } else { audible_dry };
        }
    }
    audio.samples = output;
    Ok(())
}

fn wsola(
    audio: &mut AudioBuffer,
    duration_factor: f32,
    window_ms: f32,
    search_ms: f32,
    overlap_ms: f32,
) -> Result<()> {
    if !duration_factor.is_finite() || duration_factor <= 0.0 {
        bail!("stretch/tempo factor must be finite and greater than zero");
    }
    if [window_ms, search_ms, overlap_ms]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || window_ms <= 0.0
        || overlap_ms <= 0.0
        || overlap_ms >= window_ms
    {
        bail!("stretch window/overlap parameters are invalid");
    }
    let channels = usize::from(audio.spec.channels);
    if channels == 0 || audio.spec.sample_rate == 0 {
        bail!("stretch/tempo requires non-zero sample rate and channel count");
    }
    if (duration_factor - 1.0).abs() <= f32::EPSILON || audio.samples.is_empty() {
        return Ok(());
    }
    let input_frames = audio.frames();
    let target_frames_f = input_frames as f64 * f64::from(duration_factor);
    if !target_frames_f.is_finite() || target_frames_f > usize::MAX as f64 {
        bail!("stretch/tempo output length is too large");
    }
    let target_frames = (target_frames_f.round() as usize).max(1);
    let target_samples = target_frames
        .checked_mul(channels)
        .ok_or_else(|| anyhow::anyhow!("stretch/tempo output size overflow"))?;
    if input_frames == 1 {
        let sample = audio.samples.clone();
        audio.samples = sample.repeat(target_frames);
        return Ok(());
    }
    let window = ((window_ms * audio.spec.sample_rate as f32 / 1000.0).round() as usize)
        .clamp(2, input_frames);
    let overlap = ((overlap_ms * audio.spec.sample_rate as f32 / 1000.0).round() as usize)
        .clamp(1, window.saturating_sub(1));
    let hop = window - overlap;
    let search = (search_ms * audio.spec.sample_rate as f32 / 1000.0).round() as usize;
    let max_source_start = input_frames.saturating_sub(window);
    let mut output = Vec::new();
    output.try_reserve_exact(target_samples)?;
    output.resize(target_samples, 0.0);
    let initial = window.min(target_frames);
    output[..initial * channels].copy_from_slice(&audio.samples[..initial * channels]);
    let mut output_start = hop;
    while output_start < target_frames {
        let predicted = ((output_start as f64 / f64::from(duration_factor)).round() as usize)
            .min(max_source_start);
        let low = predicted.saturating_sub(search);
        let high = predicted.saturating_add(search).min(max_source_start);
        let reference_end = (output_start + overlap).min(target_frames);
        let compare_frames = reference_end.saturating_sub(output_start);
        let mut best_source = predicted;
        let mut best_score = f64::NEG_INFINITY;
        for candidate in low..=high {
            let mut dot = 0.0_f64;
            let mut ref_energy = 0.0_f64;
            let mut src_energy = 0.0_f64;
            for frame in (0..compare_frames).step_by(4) {
                let out_index = (output_start + frame) * channels;
                let src_index = (candidate + frame).min(input_frames - 1) * channels;
                for channel in 0..channels {
                    let left = f64::from(output[out_index + channel]);
                    let right = f64::from(audio.samples[src_index + channel]);
                    dot += left * right;
                    ref_energy += left * left;
                    src_energy += right * right;
                }
            }
            let score = if ref_energy <= f64::EPSILON || src_energy <= f64::EPSILON {
                if candidate == predicted { 0.0 } else { -1.0 }
            } else {
                dot / (ref_energy * src_energy).sqrt()
            };
            if score > best_score {
                best_score = score;
                best_source = candidate;
            }
        }
        let segment_frames = window.min(target_frames - output_start);
        let blend_frames = overlap.min(segment_frames);
        for frame in 0..blend_frames {
            let alpha = (frame + 1) as f32 / (blend_frames + 1) as f32;
            let out_index = (output_start + frame) * channels;
            let src_index = (best_source + frame).min(input_frames - 1) * channels;
            for channel in 0..channels {
                output[out_index + channel] = output[out_index + channel] * (1.0 - alpha)
                    + audio.samples[src_index + channel] * alpha;
            }
        }
        for frame in blend_frames..segment_frames {
            let out_index = (output_start + frame) * channels;
            let src_index = (best_source + frame).min(input_frames - 1) * channels;
            output[out_index..out_index + channels]
                .copy_from_slice(&audio.samples[src_index..src_index + channels]);
        }
        output_start = output_start.saturating_add(hop);
    }
    audio.samples = output;
    Ok(())
}

fn trim(audio: &mut AudioBuffer, start_sec: f32, duration_sec: Option<f32>) -> Result<()> {
    ensure_duration(start_sec, "trim start")?;
    if let Some(duration) = duration_sec {
        ensure_duration(duration, "trim duration")?;
    }

    let channels = usize::from(audio.spec.channels);
    let start_frame = seconds_to_frames(start_sec, audio.spec.sample_rate);
    // `seconds_to_frames` saturates at usize::MAX for huge durations, so the end
    // must saturate too: `trim 0.2 1e30` keeps everything after 0.2 s.
    let end_frame = duration_sec
        .map(|duration| {
            start_frame.saturating_add(seconds_to_frames(duration, audio.spec.sample_rate))
        })
        .unwrap_or_else(|| audio.frames())
        .min(audio.frames());

    if start_frame >= audio.frames() || start_frame >= end_frame {
        audio.samples.clear();
        return Ok(());
    }

    let start = start_frame * channels;
    let end = end_frame * channels;
    audio.samples = audio.samples[start..end].to_vec();
    Ok(())
}

fn fade(audio: &mut AudioBuffer, in_sec: f32, out_sec: f32) -> Result<()> {
    ensure_duration(in_sec, "fade-in duration")?;
    ensure_duration(out_sec, "fade-out duration")?;
    let channels = usize::from(audio.spec.channels);
    let frames = audio.frames();
    let fade_in_frames = seconds_to_frames(in_sec, audio.spec.sample_rate).min(frames);
    let fade_out_frames = seconds_to_frames(out_sec, audio.spec.sample_rate).min(frames);

    for frame in 0..fade_in_frames {
        let factor = frame as f32 / fade_in_frames.max(1) as f32;
        scale_frame(audio, frame, channels, factor);
    }

    for n in 0..fade_out_frames {
        let frame = frames - fade_out_frames + n;
        let factor = 1.0 - (n as f32 / fade_out_frames.max(1) as f32);
        scale_frame(audio, frame, channels, factor);
    }

    Ok(())
}

fn reverse(audio: &mut AudioBuffer) {
    let channels = usize::from(audio.spec.channels);
    let mut reversed = Vec::with_capacity(audio.samples.len());
    for frame in audio.samples.chunks_exact(channels).rev() {
        reversed.extend_from_slice(frame);
    }
    audio.samples = reversed;
}

/// Play the audio `factor` times faster, changing pitch and duration together
/// (the sample rate is unchanged). This is a band-limited resampling with a
/// step of `factor` input frames per output frame; when speeding up, the
/// anti-aliasing cutoff follows the output Nyquist frequency.
fn speed(audio: &mut AudioBuffer, factor: f32) -> Result<()> {
    if !factor.is_finite() || factor <= 0.0 {
        bail!("speed factor must be finite and > 0 (got {factor})");
    }
    let channels = usize::from(audio.spec.channels);
    if factor == 1.0 || audio.samples.is_empty() || channels == 0 {
        return Ok(());
    }

    let frames = audio.frames();
    let out_frames_f = (frames as f64 / f64::from(factor)).round().max(1.0);
    if out_frames_f > (isize::MAX as usize / channels / std::mem::size_of::<f32>()) as f64 {
        bail!("speed {factor} would produce an output that is too large");
    }
    let out_frames = out_frames_f as usize;
    let step = match exact_ratio(f64::from(factor), RESAMPLE_MAX_EXACT_PHASES) {
        Some((num, den)) => ResampleStep::Ratio { num, den },
        None => ResampleStep::Real(f64::from(factor)),
    };
    audio.samples = resample_interleaved(&audio.samples, channels, step, out_frames)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Band-limited resampling (shared by `rate` and `speed`)
// ---------------------------------------------------------------------------

/// Kaiser-window design target: stopband attenuation in dB.
const RESAMPLE_STOPBAND_DB: f64 = 100.0;
/// Passband edge as a fraction of the lower of the two Nyquist frequencies.
/// The stopband starts at that Nyquist frequency, so nothing above it aliases.
const RESAMPLE_PASSBAND: f64 = 0.90;
/// Kernel table resolution (points per low-rate sample) for arbitrary phases.
const RESAMPLE_TABLE_DENSITY: usize = 4096;
/// Largest phase count for which an exact polyphase bank is precomputed.
const RESAMPLE_MAX_EXACT_PHASES: u64 = 1024;
/// Largest exact polyphase bank, in coefficients (16 MiB of f32).
const RESAMPLE_MAX_BANK: usize = 1 << 22;
/// Largest filter half-length, in input frames (limits extreme ratios).
const RESAMPLE_MAX_RADIUS: f64 = (1 << 22) as f64;

/// How far the read position advances in the input per output frame.
#[derive(Debug, Clone, Copy)]
enum ResampleStep {
    /// Exactly `num / den` input frames per output frame.
    Ratio { num: u64, den: u64 },
    /// An arbitrary step, tracked in f64.
    Real(f64),
}

impl ResampleStep {
    fn value(self) -> f64 {
        match self {
            Self::Ratio { num, den } => num as f64 / den as f64,
            Self::Real(step) => step,
        }
    }
}

/// Windowed-sinc low-pass kernel. Distances are measured in input frames;
/// `scale` (= min(1, output rate / input rate)) stretches the kernel when
/// downsampling so that the cutoff tracks the output Nyquist frequency.
struct SincKernel {
    /// Cutoff in cycles per low-rate sample (between passband and Nyquist).
    cutoff: f64,
    /// Half-width of the window in low-rate samples.
    half_width: f64,
    beta: f64,
    i0_beta: f64,
    scale: f64,
    /// Half-length of the filter in input frames; the filter has 2 * radius taps.
    radius: usize,
}

impl SincKernel {
    fn new(step: f64) -> Result<Self> {
        if !step.is_finite() || step <= 0.0 {
            bail!("resampling step must be finite and > 0");
        }
        let scale = (1.0 / step).min(1.0);
        // Kaiser's design formulas: transition width as a fraction of the
        // low sample rate, then the filter length for the target attenuation.
        let transition = (1.0 - RESAMPLE_PASSBAND) * 0.5;
        let length = ((RESAMPLE_STOPBAND_DB - 7.95) / (14.36 * transition)).ceil() + 1.0;
        let half_width = (length / 2.0).ceil();
        let beta = 0.1102 * (RESAMPLE_STOPBAND_DB - 8.7);
        let radius = (half_width / scale).ceil();
        if radius > RESAMPLE_MAX_RADIUS {
            bail!("resampling ratio 1:{step} is too extreme");
        }
        Ok(Self {
            cutoff: (1.0 + RESAMPLE_PASSBAND) * 0.25,
            half_width,
            beta,
            i0_beta: bessel_i0(beta),
            scale,
            radius: radius as usize,
        })
    }

    /// Kernel value (without the DC normalisation) at `u` low-rate samples.
    fn shape(&self, u: f64) -> f64 {
        let ratio = u / self.half_width;
        if ratio.abs() >= 1.0 {
            return 0.0;
        }
        let x = 2.0 * self.cutoff * u;
        let sinc = if x.abs() < 1.0e-12 {
            1.0
        } else {
            (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
        };
        2.0 * self.cutoff * sinc * bessel_i0(self.beta * (1.0 - ratio * ratio).sqrt())
            / self.i0_beta
    }

    fn taps(&self) -> usize {
        2 * self.radius
    }

    /// Coefficients for an output frame that sits `phase` (0 <= phase < 1)
    /// input frames after input frame `i`. Tap `k` multiplies input frame
    /// `i + 1 - radius + k`, at distance `phase + radius - 1 - k`.
    fn fill_exact(&self, phase: f64, coefficients: &mut [f32]) {
        let offset = phase + self.radius as f64 - 1.0;
        let mut sum = 0.0;
        let mut values = Vec::with_capacity(coefficients.len());
        for k in 0..coefficients.len() {
            let value = self.shape((offset - k as f64) * self.scale);
            sum += value;
            values.push(value);
        }
        normalise_into(&values, sum, coefficients);
    }

    /// Kernel sampled every 1/RESAMPLE_TABLE_DENSITY low-rate samples.
    fn table(&self) -> Vec<f32> {
        let points = (self.half_width * RESAMPLE_TABLE_DENSITY as f64).ceil() as usize + 2;
        (0..points)
            .map(|index| self.shape(index as f64 / RESAMPLE_TABLE_DENSITY as f64) as f32)
            .collect()
    }

    /// Like `fill_exact`, but reads the kernel from `table` with linear
    /// interpolation (used for phases that are not on a precomputed grid).
    fn fill_from_table(&self, table: &[f32], phase: f64, coefficients: &mut [f32]) {
        let offset = phase + self.radius as f64 - 1.0;
        let density = RESAMPLE_TABLE_DENSITY as f64;
        let mut sum = 0.0_f64;
        for (k, coefficient) in coefficients.iter_mut().enumerate() {
            let position = ((offset - k as f64) * self.scale).abs() * density;
            let index = position as usize;
            let value = if index + 1 < table.len() {
                let fraction = (position - index as f64) as f32;
                table[index] + (table[index + 1] - table[index]) * fraction
            } else {
                0.0
            };
            *coefficient = value;
            sum += f64::from(value);
        }
        if sum.abs() > f64::EPSILON {
            let gain = (1.0 / sum) as f32;
            for coefficient in coefficients.iter_mut() {
                *coefficient *= gain;
            }
        }
    }
}

/// Scale `values` to unit sum (unity DC gain for every phase) into `out`.
fn normalise_into(values: &[f64], sum: f64, out: &mut [f32]) {
    let gain = if sum.abs() > f64::EPSILON {
        1.0 / sum
    } else {
        1.0
    };
    for (destination, value) in out.iter_mut().zip(values) {
        *destination = (value * gain) as f32;
    }
}

/// Zeroth-order modified Bessel function of the first kind (power series).
fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..200 {
        let factor = half / k as f64;
        term *= factor * factor;
        sum += term;
        if term < sum * 1.0e-17 {
            break;
        }
    }
    sum
}

/// Best rational approximation `num / den` of `value` with `den <= max_den`,
/// returned only when it equals `value` to within f64 rounding.
fn exact_ratio(value: f64, max_den: u64) -> Option<(u64, u64)> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let (mut h0, mut h1) = (0_u64, 1_u64);
    let (mut k0, mut k1) = (1_u64, 0_u64);
    let mut remainder = value;
    for _ in 0..64 {
        let whole = remainder.floor();
        if whole > 1.0e15 {
            return None;
        }
        let a = whole as u64;
        let h2 = a.checked_mul(h1)?.checked_add(h0)?;
        let k2 = a.checked_mul(k1)?.checked_add(k0)?;
        if k2 > max_den {
            return None;
        }
        if ((h2 as f64 / k2 as f64) - value).abs() <= value * 4.0 * f64::EPSILON {
            return (h2 > 0).then_some((h2, k2));
        }
        let fraction = remainder - whole;
        if fraction <= 0.0 {
            return None;
        }
        remainder = 1.0 / fraction;
        (h0, h1) = (h1, h2);
        (k0, k1) = (k1, k2);
    }
    None
}

/// Unrolled dot product (independent accumulators so it vectorises).
fn dot(coefficients: &[f32], input: &[f32]) -> f32 {
    let mut lanes = [0.0_f32; 8];
    let coefficient_chunks = coefficients.chunks_exact(8);
    let input_chunks = input.chunks_exact(8);
    let tail = coefficient_chunks
        .remainder()
        .iter()
        .zip(input_chunks.remainder())
        .map(|(c, x)| c * x)
        .sum::<f32>();
    for (c, x) in coefficient_chunks.zip(input_chunks) {
        for ((lane, &coefficient), &sample) in lanes.iter_mut().zip(c).zip(x) {
            *lane += coefficient * sample;
        }
    }
    lanes.iter().sum::<f32>() + tail
}

/// Band-limited resampling of interleaved audio with a Kaiser-windowed sinc
/// filter (~100 dB stopband, passband to 90% of the lower Nyquist frequency).
///
/// Output frame `n` is centred exactly on input position `n * step` (the
/// kernel is symmetric, so there is no group delay to compensate); input
/// outside the buffer is treated as silence. Exact rational steps with up to
/// `RESAMPLE_MAX_EXACT_PHASES` phases use a precomputed polyphase bank;
/// other steps interpolate a finely sampled kernel table.
fn resample_interleaved(
    input: &[f32],
    channels: usize,
    step: ResampleStep,
    out_frames: usize,
) -> Result<Vec<f32>> {
    let frames = input.len() / channels;
    let kernel = SincKernel::new(step.value())?;
    let radius = kernel.radius;
    let taps = kernel.taps();

    // Deinterleave into zero-padded planes so every tap window is in bounds:
    // input frame `f` lives at plane index `f + radius`.
    let too_large = || anyhow::anyhow!("resampled audio is too large");
    let plane_len = frames
        .checked_add(radius.checked_mul(2).ok_or_else(too_large)?)
        .and_then(|len| len.checked_add(2))
        .ok_or_else(too_large)?;
    let mut planes = Vec::with_capacity(channels);
    for _ in 0..channels {
        planes.push(zeroed_samples(plane_len, "resampler input")?);
    }
    for (frame_index, frame) in input.chunks_exact(channels).enumerate() {
        for (plane, &sample) in planes.iter_mut().zip(frame) {
            plane[radius + frame_index] = sample;
        }
    }

    let output_len = out_frames.checked_mul(channels).ok_or_else(too_large)?;
    let mut output = zeroed_samples(output_len, "resampled output")?;

    let bank = match step {
        ResampleStep::Ratio { num: _, den }
            if den <= RESAMPLE_MAX_EXACT_PHASES
                && (den as usize)
                    .checked_mul(taps)
                    .is_some_and(|size| size <= RESAMPLE_MAX_BANK) =>
        {
            let phases = den as usize;
            let mut bank = zeroed_samples(phases * taps, "resampler filter bank")?;
            for (phase, coefficients) in bank.chunks_exact_mut(taps).enumerate() {
                kernel.fill_exact(phase as f64 / den as f64, coefficients);
            }
            Some(bank)
        }
        _ => None,
    };
    let table = if bank.is_none() {
        kernel.table()
    } else {
        Vec::new()
    };
    let mut scratch = vec![0.0_f32; taps];

    // Exact position tracking for rational steps: input frame `index` plus
    // `numerator / den` of a frame.
    let (step_whole, step_fraction, den) = match step {
        ResampleStep::Ratio { num, den } => ((num / den) as usize, num % den, den),
        ResampleStep::Real(_) => (0, 0, 1),
    };
    let mut index = 0_usize;
    let mut numerator = 0_u64;

    for (out_index, out_frame) in output.chunks_exact_mut(channels).enumerate() {
        let (frame, coefficients): (usize, &[f32]) = match (step, &bank) {
            (ResampleStep::Ratio { .. }, Some(bank)) => {
                let phase = numerator as usize;
                (index, &bank[phase * taps..(phase + 1) * taps])
            }
            (ResampleStep::Ratio { .. }, None) => {
                kernel.fill_from_table(&table, numerator as f64 / den as f64, &mut scratch);
                (index, &scratch)
            }
            (ResampleStep::Real(step), _) => {
                let position = out_index as f64 * step;
                let whole = position.floor();
                kernel.fill_from_table(&table, position - whole, &mut scratch);
                (whole as usize, &scratch)
            }
        };
        // Positions past the end only see padding; clamp to stay in bounds.
        let start = frame.min(frames) + 1;
        for (sample, plane) in out_frame.iter_mut().zip(&planes) {
            *sample = dot(coefficients, &plane[start..start + taps]);
        }

        numerator += step_fraction;
        if numerator >= den {
            numerator -= den;
            index = index.saturating_add(1);
        }
        index = index.saturating_add(step_whole);
    }
    Ok(output)
}

fn pad(audio: &mut AudioBuffer, start_sec: f32, end_sec: f32) -> Result<()> {
    ensure_duration(start_sec, "pad start")?;
    ensure_duration(end_sec, "pad end")?;

    let channels = usize::from(audio.spec.channels);
    let too_large = || anyhow::anyhow!("padded output is too large");
    let start_samples = seconds_to_frames(start_sec, audio.spec.sample_rate)
        .checked_mul(channels)
        .ok_or_else(too_large)?;
    let end_samples = seconds_to_frames(end_sec, audio.spec.sample_rate)
        .checked_mul(channels)
        .ok_or_else(too_large)?;
    let total = start_samples
        .checked_add(audio.samples.len())
        .and_then(|total| total.checked_add(end_samples))
        .ok_or_else(too_large)?;
    let mut output = zeroed_samples(total, "padded output")?;
    output[start_samples..start_samples + audio.samples.len()].copy_from_slice(&audio.samples);
    audio.samples = output;
    Ok(())
}

fn trim_silence(audio: &mut AudioBuffer, threshold_db: f32, min_duration_sec: f32) -> Result<()> {
    ensure_duration(min_duration_sec, "silence minimum duration")?;
    if !threshold_db.is_finite() {
        bail!("silence threshold must be a finite number of dB (got {threshold_db})");
    }
    if audio.samples.is_empty() {
        return Ok(());
    }

    let channels = usize::from(audio.spec.channels);
    let threshold = 10.0_f32.powf(threshold_db / 20.0);
    let min_frames = seconds_to_frames(min_duration_sec, audio.spec.sample_rate);
    let silent = |frame: &[f32]| frame.iter().all(|sample| sample.abs() <= threshold);

    let frames: Vec<&[f32]> = audio.samples.chunks_exact(channels).collect();
    let leading = contiguous_silence(&frames, min_frames, &silent, false);
    let trailing = contiguous_silence(&frames, min_frames, &silent, true);
    let keep_start = leading.min(frames.len());
    let keep_end = frames.len().saturating_sub(trailing);

    if keep_start >= keep_end {
        audio.samples.clear();
        return Ok(());
    }

    audio.samples = audio.samples[keep_start * channels..keep_end * channels].to_vec();
    Ok(())
}

fn lowpass(audio: &mut AudioBuffer, hz: f32) -> Result<()> {
    if !hz.is_finite() || hz <= 0.0 {
        bail!("lowpass frequency must be finite and > 0 (got {hz})");
    }
    if audio.spec.sample_rate == 0 {
        bail!("lowpass requires a non-zero sample rate");
    }
    let channels = usize::from(audio.spec.channels);
    let dt = 1.0 / audio.spec.sample_rate as f32;
    let rc = 1.0 / (std::f32::consts::TAU * hz);
    let alpha = dt / (rc + dt);
    let mut state = vec![0.0; channels];

    for frame in audio.samples.chunks_exact_mut(channels) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            state[channel] += alpha * (*sample - state[channel]);
            *sample = state[channel];
        }
    }

    Ok(())
}

fn highpass(audio: &mut AudioBuffer, hz: f32) -> Result<()> {
    if !hz.is_finite() || hz <= 0.0 {
        bail!("highpass frequency must be finite and > 0 (got {hz})");
    }
    if audio.spec.sample_rate == 0 {
        bail!("highpass requires a non-zero sample rate");
    }
    let channels = usize::from(audio.spec.channels);
    let dt = 1.0 / audio.spec.sample_rate as f32;
    let rc = 1.0 / (std::f32::consts::TAU * hz);
    let alpha = rc / (rc + dt);
    let mut prev_input = vec![0.0; channels];
    let mut prev_output = vec![0.0; channels];

    for frame in audio.samples.chunks_exact_mut(channels) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let input = *sample;
            let output = alpha * (prev_output[channel] + input - prev_input[channel]);
            prev_input[channel] = input;
            prev_output[channel] = output;
            *sample = output;
        }
    }

    Ok(())
}

fn biquad_filter(
    audio: &mut AudioBuffer,
    kind: BiquadKind,
    hz: f32,
    width: FilterWidth,
    gain_db: f32,
    poles: u8,
) -> Result<()> {
    if audio.spec.sample_rate == 0 || audio.spec.channels == 0 {
        bail!("biquad filters require a non-zero sample rate and channel count");
    }
    let nyquist = audio.spec.sample_rate as f32 / 2.0;
    if !hz.is_finite() || hz <= 0.0 || hz >= nyquist {
        bail!("filter frequency must be finite and between zero and Nyquist ({nyquist} Hz)");
    }
    if !gain_db.is_finite() {
        bail!("filter gain must be finite");
    }
    if poles == 1 {
        return match kind {
            BiquadKind::LowPass => lowpass(audio, hz),
            BiquadKind::HighPass => highpass(audio, hz),
            _ => bail!("the -1 pole mode is only supported for lowpass and highpass"),
        };
    }
    if poles != 2 {
        bail!("filter pole count must be 1 or 2");
    }

    let q = width_to_q(width, hz)?;
    let w0 = std::f64::consts::TAU * f64::from(hz) / f64::from(audio.spec.sample_rate);
    let (sin_w0, cos_w0) = w0.sin_cos();
    let alpha = sin_w0 / (2.0 * f64::from(q));
    let gain_a = 10.0_f64.powf(f64::from(gain_db) / 40.0);
    let (b0, b1, b2, a0, a1, a2) = match kind {
        BiquadKind::LowPass => (
            (1.0 - cos_w0) / 2.0,
            1.0 - cos_w0,
            (1.0 - cos_w0) / 2.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        ),
        BiquadKind::HighPass => (
            (1.0 + cos_w0) / 2.0,
            -(1.0 + cos_w0),
            (1.0 + cos_w0) / 2.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        ),
        BiquadKind::BandPass => {
            let peak_gain = alpha;
            (
                peak_gain,
                0.0,
                -peak_gain,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            )
        }
        BiquadKind::BandPassConstantSkirt => (
            sin_w0 / 2.0,
            0.0,
            -sin_w0 / 2.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        ),
        BiquadKind::BandReject => (
            1.0,
            -2.0 * cos_w0,
            1.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        ),
        BiquadKind::AllPass => (
            1.0 - alpha,
            -2.0 * cos_w0,
            1.0 + alpha,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        ),
        BiquadKind::Equalizer => (
            1.0 + alpha * gain_a,
            -2.0 * cos_w0,
            1.0 - alpha * gain_a,
            1.0 + alpha / gain_a,
            -2.0 * cos_w0,
            1.0 - alpha / gain_a,
        ),
        BiquadKind::LowShelf | BiquadKind::HighShelf => {
            let slope = width_to_shelf_slope(width, hz, audio.spec.sample_rate)?;
            if slope <= 0.0
                || !slope.is_finite()
                || (matches!(width, FilterWidth::Slope(_)) && slope > 1.0)
            {
                bail!(
                    "shelf slope must be finite and greater than zero; explicit s widths may not exceed one"
                );
            }
            let shelf_alpha_argument =
                (gain_a + 1.0 / gain_a) * (1.0 / f64::from(slope) - 1.0) + 2.0;
            if shelf_alpha_argument < 0.0 || !shelf_alpha_argument.is_finite() {
                bail!("shelf width and gain produce invalid filter coefficients");
            }
            let shelf_alpha = sin_w0 / 2.0 * shelf_alpha_argument.sqrt();
            let beta = 2.0 * gain_a.sqrt() * shelf_alpha;
            if matches!(kind, BiquadKind::LowShelf) {
                (
                    gain_a * ((gain_a + 1.0) - (gain_a - 1.0) * cos_w0 + beta),
                    2.0 * gain_a * ((gain_a - 1.0) - (gain_a + 1.0) * cos_w0),
                    gain_a * ((gain_a + 1.0) - (gain_a - 1.0) * cos_w0 - beta),
                    (gain_a + 1.0) + (gain_a - 1.0) * cos_w0 + beta,
                    -2.0 * ((gain_a - 1.0) + (gain_a + 1.0) * cos_w0),
                    (gain_a + 1.0) + (gain_a - 1.0) * cos_w0 - beta,
                )
            } else {
                (
                    gain_a * ((gain_a + 1.0) + (gain_a - 1.0) * cos_w0 + beta),
                    -2.0 * gain_a * ((gain_a - 1.0) + (gain_a + 1.0) * cos_w0),
                    gain_a * ((gain_a + 1.0) + (gain_a - 1.0) * cos_w0 - beta),
                    (gain_a + 1.0) - (gain_a - 1.0) * cos_w0 + beta,
                    2.0 * ((gain_a - 1.0) - (gain_a + 1.0) * cos_w0),
                    (gain_a + 1.0) - (gain_a - 1.0) * cos_w0 - beta,
                )
            }
        }
    };
    if !a0.is_finite() || a0.abs() <= f64::EPSILON {
        bail!("filter coefficients are not finite");
    }
    let (b0, b1, b2, a1, a2) = (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
    if [b0, b1, b2, a1, a2]
        .iter()
        .any(|coefficient| !coefficient.is_finite())
    {
        bail!("filter coefficients are not finite");
    }

    let channels = usize::from(audio.spec.channels);
    let mut in1 = vec![0.0_f64; channels];
    let mut in2 = vec![0.0_f64; channels];
    let mut out1 = vec![0.0_f64; channels];
    let mut out2 = vec![0.0_f64; channels];
    for frame in audio.samples.chunks_exact_mut(channels) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let input = f64::from(*sample);
            let output = b0 * input + b1 * in1[channel] + b2 * in2[channel]
                - a1 * out1[channel]
                - a2 * out2[channel];
            in2[channel] = in1[channel];
            in1[channel] = input;
            out2[channel] = out1[channel];
            out1[channel] = output;
            *sample = output as f32;
        }
    }
    Ok(())
}

fn width_to_q(width: FilterWidth, center_hz: f32) -> Result<f32> {
    let q = match width {
        FilterWidth::Hertz(bandwidth) | FilterWidth::HertzNoWarp(bandwidth) => {
            center_hz / bandwidth
        }
        FilterWidth::Kilohertz(bandwidth) => center_hz / (bandwidth * 1000.0),
        FilterWidth::Octaves(bandwidth) => {
            let octave_ratio = 2.0_f64.powf(f64::from(bandwidth));
            (octave_ratio.sqrt() / (octave_ratio - 1.0)) as f32
        }
        FilterWidth::Q(q) => q,
        FilterWidth::Slope(slope) => 1.0 / slope,
    };
    if !q.is_finite() || q <= 0.0 {
        bail!("filter width must be finite and greater than zero");
    }
    Ok(q)
}

fn width_to_shelf_slope(width: FilterWidth, center_hz: f32, sample_rate: u32) -> Result<f32> {
    if let FilterWidth::Slope(slope) = width {
        return Ok(slope);
    }
    let bandwidth_octaves = match width {
        FilterWidth::Hertz(bandwidth) | FilterWidth::HertzNoWarp(bandwidth) => {
            if bandwidth >= 2.0 * center_hz {
                bail!("shelf bandwidth in Hz must be less than twice its center frequency");
            }
            ((center_hz + bandwidth / 2.0) / (center_hz - bandwidth / 2.0)).log2()
        }
        FilterWidth::Kilohertz(bandwidth) => {
            return width_to_shelf_slope(
                FilterWidth::Hertz(bandwidth * 1000.0),
                center_hz,
                sample_rate,
            );
        }
        FilterWidth::Octaves(octaves) => octaves,
        FilterWidth::Q(q) => {
            (2.0 * (1.0 / (2.0 * f64::from(q))).asinh() / std::f64::consts::LN_2) as f32
        }
        FilterWidth::Slope(_) => unreachable!(),
    };
    if !bandwidth_octaves.is_finite() || bandwidth_octaves <= 0.0 {
        bail!("filter width must be finite and greater than zero");
    }
    let omega = std::f64::consts::TAU * f64::from(center_hz) / f64::from(sample_rate);
    let warped =
        (std::f64::consts::LN_2 / 2.0) * f64::from(bandwidth_octaves) * omega / omega.sin();
    let slope = (1.0 / warped.sinh()) as f32;
    Ok(slope)
}

fn echo(
    audio: &mut AudioBuffer,
    input_gain: f32,
    output_gain: f32,
    taps_ms: &[(f32, f32)],
) -> Result<()> {
    if !input_gain.is_finite() || !output_gain.is_finite() {
        bail!("echo gains must be finite");
    }
    if taps_ms.is_empty() {
        bail!("echo requires at least one delay/decay pair");
    }
    if audio.spec.channels == 0 || audio.spec.sample_rate == 0 {
        bail!("echo requires a non-zero channel count and sample rate");
    }

    let mut taps = Vec::with_capacity(taps_ms.len());
    for &(delay_ms, decay) in taps_ms {
        if !delay_ms.is_finite() || delay_ms <= 0.0 {
            bail!("echo delay must be finite and greater than zero milliseconds");
        }
        if !decay.is_finite() || !(0.0..=1.0).contains(&decay) {
            bail!("echo decay must be between zero and one");
        }
        let delay_frames = seconds_to_frames(delay_ms / 1000.0, audio.spec.sample_rate);
        if delay_frames == 0 {
            bail!("echo delay is shorter than one sample at this sample rate");
        }
        taps.push((delay_frames, decay));
    }

    let channels = usize::from(audio.spec.channels);
    let frames = audio.frames();
    let max_delay = taps.iter().map(|(frames, _)| *frames).max().unwrap_or(0);
    let output_frames = frames
        .checked_add(max_delay)
        .ok_or_else(|| anyhow::anyhow!("echo output is too large"))?;
    let output_len = output_frames
        .checked_mul(channels)
        .ok_or_else(|| anyhow::anyhow!("echo output is too large"))?;
    let mut output = zeroed_samples(output_len, "echo output")?;

    for (frame_index, frame) in audio.samples.chunks_exact(channels).enumerate() {
        let dry_index = frame_index * channels;
        for (channel, &sample) in frame.iter().enumerate() {
            output[dry_index + channel] += input_gain * sample;
        }
        for &(delay_frames, decay) in &taps {
            let delayed_index = (frame_index + delay_frames) * channels;
            for (channel, &sample) in frame.iter().enumerate() {
                output[delayed_index + channel] += decay * sample;
            }
        }
    }

    for sample in &mut output {
        *sample *= output_gain;
    }
    audio.samples = output;
    Ok(())
}

fn tremolo(audio: &mut AudioBuffer, speed_hz: f32, depth_percent: f32) -> Result<()> {
    if !speed_hz.is_finite() || speed_hz <= 0.0 {
        bail!("tremolo speed must be finite and greater than zero");
    }
    if !depth_percent.is_finite() || !(0.0..=100.0).contains(&depth_percent) {
        bail!("tremolo depth must be between zero and 100 percent");
    }
    if audio.spec.sample_rate == 0 {
        bail!("tremolo requires a non-zero sample rate");
    }

    let depth = depth_percent / 100.0;
    let sample_rate = audio.spec.sample_rate as f32;
    let channels = usize::from(audio.spec.channels);
    for (frame_index, frame) in audio.samples.chunks_exact_mut(channels).enumerate() {
        let phase = std::f32::consts::TAU * speed_hz * frame_index as f32 / sample_rate;
        let amplitude = 1.0 - depth + depth * (0.5 + 0.5 * phase.sin());
        for sample in frame {
            *sample *= amplitude;
        }
    }
    Ok(())
}

fn delay(audio: &mut AudioBuffer, delays_sec: &[f32]) -> Result<()> {
    if delays_sec.is_empty() {
        bail!("delay requires at least one channel position");
    }
    let channels = usize::from(audio.spec.channels);
    if channels == 0 || audio.spec.sample_rate == 0 {
        bail!("delay requires a non-zero channel count and sample rate");
    }
    if delays_sec.len() > 1 && delays_sec.len() > channels {
        bail!("delay has more positions than audio channels");
    }
    let mut channel_delays = Vec::with_capacity(channels);
    for channel in 0..channels {
        let delay_sec = if delays_sec.len() == 1 {
            delays_sec[0]
        } else {
            delays_sec.get(channel).copied().unwrap_or(0.0)
        };
        if !delay_sec.is_finite() || delay_sec < 0.0 {
            bail!("delay positions must be finite and non-negative");
        }
        channel_delays.push(seconds_to_frames(delay_sec, audio.spec.sample_rate));
    }

    let frames = audio.frames();
    let max_delay = channel_delays.iter().copied().max().unwrap_or(0);
    let output_frames = frames
        .checked_add(max_delay)
        .ok_or_else(|| anyhow::anyhow!("delayed output is too large"))?;
    let output_len = output_frames
        .checked_mul(channels)
        .ok_or_else(|| anyhow::anyhow!("delayed output is too large"))?;
    let mut output = zeroed_samples(output_len, "delayed output")?;
    for (frame_index, frame) in audio.samples.chunks_exact(channels).enumerate() {
        for (channel, &sample) in frame.iter().enumerate() {
            let target = (frame_index + channel_delays[channel]) * channels + channel;
            output[target] = sample;
        }
    }
    audio.samples = output;
    Ok(())
}

fn dcshift(audio: &mut AudioBuffer, shift: f32, limiter_gain: Option<f32>) -> Result<()> {
    if !shift.is_finite() {
        bail!("dcshift value must be finite");
    }
    if limiter_gain.is_some_and(|gain| !gain.is_finite() || !(0.0..1.0).contains(&gain)) {
        bail!("dcshift limiter gain must be greater than zero and less than one");
    }
    let limiter_threshold = limiter_gain.map(|gain| 1.0 - (shift.abs() - gain));
    for sample in &mut audio.samples {
        let input = *sample;
        let shifted = match (shift, limiter_gain, limiter_threshold) {
            (shift, Some(gain), Some(threshold)) if shift > 0.0 && input > threshold => {
                let range = 1.0 - threshold;
                if range > f32::EPSILON {
                    (input - threshold) * gain / range + threshold + shift
                } else {
                    input + shift
                }
            }
            (shift, Some(gain), Some(threshold)) if shift < 0.0 && input < -threshold => {
                let range = 1.0 - threshold;
                if range > f32::EPSILON {
                    (input + threshold) * gain / range - threshold + shift
                } else {
                    input + shift
                }
            }
            _ => input + shift,
        };
        *sample = shifted.clamp(-1.0, 1.0);
        if !sample.is_finite() {
            bail!("dcshift produced a non-finite sample");
        }
    }
    Ok(())
}

fn downsample(audio: &mut AudioBuffer, factor: u32) -> Result<()> {
    if factor == 0 {
        bail!("downsample factor must be greater than zero");
    }
    if audio.spec.sample_rate == 0 || factor > audio.spec.sample_rate {
        bail!("downsample factor must not exceed a non-zero sample rate");
    }
    if factor == 1 || audio.samples.is_empty() {
        if factor > 1 {
            audio.spec.sample_rate = (audio.spec.sample_rate / factor).max(1);
        }
        return Ok(());
    }
    let channels = usize::from(audio.spec.channels);
    let step = usize::try_from(factor).context("downsample factor is too large")?;
    let output_len = audio
        .frames()
        .div_ceil(step)
        .checked_mul(channels)
        .ok_or_else(|| anyhow::anyhow!("downsample output is too large"))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .context("not enough memory to downsample audio")?;
    for frame in audio.samples.chunks_exact(channels).step_by(step) {
        output.extend_from_slice(frame);
    }
    audio.samples = output;
    audio.spec.sample_rate = (audio.spec.sample_rate / factor).max(1);
    Ok(())
}

fn upsample(audio: &mut AudioBuffer, factor: u32) -> Result<()> {
    if factor == 0 {
        bail!("upsample factor must be greater than zero");
    }
    if audio.spec.sample_rate == 0 {
        bail!("upsample requires a non-zero sample rate");
    }
    let output_rate = audio
        .spec
        .sample_rate
        .checked_mul(factor)
        .ok_or_else(|| anyhow::anyhow!("upsampled sample rate is too large"))?;
    if factor == 1 || audio.samples.is_empty() {
        if factor > 1 {
            audio.spec.sample_rate = output_rate;
        }
        return Ok(());
    }
    let channels = usize::from(audio.spec.channels);
    let factor = usize::try_from(factor).context("upsample factor is too large")?;
    let output_len = audio
        .samples
        .len()
        .checked_mul(factor)
        .ok_or_else(|| anyhow::anyhow!("upsample output is too large"))?;
    let mut output = zeroed_samples(output_len, "upsampled output")?;
    for (frame_index, frame) in audio.samples.chunks_exact(channels).enumerate() {
        let target = frame_index * factor * channels;
        output[target..target + channels].copy_from_slice(frame);
    }
    audio.samples = output;
    audio.spec.sample_rate = output_rate;
    Ok(())
}

fn repeat(audio: &mut AudioBuffer, count: u32) -> Result<()> {
    let repetitions = usize::try_from(count)
        .context("repeat count is too large")?
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("repeat count is too large"))?;
    let initial_len = audio.samples.len();
    let final_len = initial_len
        .checked_mul(repetitions)
        .ok_or_else(|| anyhow::anyhow!("repeated audio is too large"))?;
    audio
        .samples
        .try_reserve_exact(final_len.saturating_sub(initial_len))
        .context("not enough memory to repeat audio")?;
    for _ in 1..repetitions {
        audio.samples.extend_from_within(..initial_len);
    }
    Ok(())
}

fn swap(audio: &mut AudioBuffer) -> Result<()> {
    if audio.spec.channels < 2 {
        bail!("swap requires at least two audio channels");
    }
    let channels = usize::from(audio.spec.channels);
    for frame in audio.samples.chunks_exact_mut(channels) {
        frame.swap(0, 1);
    }
    Ok(())
}

fn zeroed_samples(length: usize, description: &str) -> Result<Vec<f32>> {
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(length)
        .with_context(|| format!("not enough memory for {description}"))?;
    samples.resize(length, 0.0);
    Ok(samples)
}

fn limiter(audio: &mut AudioBuffer, threshold: f32) -> Result<()> {
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        bail!("limiter threshold must be between 0 and 1 (got {threshold})");
    }
    for sample in &mut audio.samples {
        *sample = sample.clamp(-threshold, threshold);
    }
    Ok(())
}

/// Convert to `sample_rate` with the band-limited resampler. The output has
/// `round(frames * new_rate / old_rate)` frames (at least one for non-empty
/// input), is aligned sample-for-sample in time with the input, and is
/// low-pass filtered at the lower of the two Nyquist frequencies.
fn rate(audio: &mut AudioBuffer, sample_rate: u32) -> Result<()> {
    if sample_rate == 0 {
        bail!("sample rate must be > 0");
    }
    let channels = usize::from(audio.spec.channels);
    if sample_rate == audio.spec.sample_rate || audio.samples.is_empty() || channels == 0 {
        audio.spec.sample_rate = sample_rate;
        return Ok(());
    }
    if audio.spec.sample_rate == 0 {
        bail!("cannot resample audio whose sample rate is zero");
    }

    let divisor = gcd(u64::from(audio.spec.sample_rate), u64::from(sample_rate));
    let num = u64::from(audio.spec.sample_rate) / divisor;
    let den = u64::from(sample_rate) / divisor;
    let frames = audio.frames();
    // round(frames * den / num) in exact integer arithmetic.
    let out_frames =
        (2 * frames as u128 * u128::from(den) + u128::from(num)) / (2 * u128::from(num));
    let out_frames = usize::try_from(out_frames.max(1))
        .map_err(|_| anyhow::anyhow!("resampled audio is too large"))?;
    audio.samples = resample_interleaved(
        &audio.samples,
        channels,
        ResampleStep::Ratio { num, den },
        out_frames,
    )?;
    audio.spec.sample_rate = sample_rate;
    Ok(())
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Gain for the centre and surround channels in the ITU-R BS.775 stereo
/// downmix: 1/sqrt(2), i.e. -3 dB.
const DOWNMIX_MINUS_3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Mixing matrix used to convert `source` channels into `target` channels:
/// `matrix[t][s]` is the gain of source channel `s` in target channel `t`.
///
/// Source layouts follow the default WAVE_FORMAT_EXTENSIBLE channel orders:
///
/// | channels | layout | order                         |
/// |----------|--------|-------------------------------|
/// | 4        | quad   | L R Ls Rs                     |
/// | 5        | 5.0    | L R C Ls Rs                   |
/// | 6        | 5.1    | L R C LFE Ls Rs               |
/// | 8        | 7.1    | L R C LFE Lb Rb Ls Rs         |
///
/// Downmix to stereo uses the ITU-R BS.775 coefficients
/// (`g = 1/sqrt(2)`, -3 dB; the LFE channel is dropped):
///
/// ```text
/// L' = L + g*C + g*Ls (+ g*Lb for 7.1)
/// R' = R + g*C + g*Rs (+ g*Rb for 7.1)
/// ```
///
/// Downmix to mono is the average of that stereo downmix, `(L' + R') / 2`,
/// which for stereo sources is the familiar `(L + R) / 2`. The coefficients
/// are not renormalised, so a loud multichannel mix can exceed full scale
/// (it is preserved in float and clamped when quantised to integers).
///
/// Other conversions keep the previous rules: mono is copied to every
/// output channel, a downmix to mono from a layout without a defined mapping
/// (3, 7 or more than 8 channels) averages all channels, and any other
/// conversion maps each output channel to the source channel with the same
/// index (the last source channel for indices past the end).
pub(crate) fn channel_matrix(source: usize, target: usize) -> Vec<Vec<f32>> {
    let g = DOWNMIX_MINUS_3DB;
    let stereo: Option<[Vec<f32>; 2]> = match source {
        2 => Some([vec![1.0, 0.0], vec![0.0, 1.0]]),
        4 => Some([vec![1.0, 0.0, g, 0.0], vec![0.0, 1.0, 0.0, g]]),
        5 => Some([vec![1.0, 0.0, g, g, 0.0], vec![0.0, 1.0, g, 0.0, g]]),
        6 => Some([
            vec![1.0, 0.0, g, 0.0, g, 0.0],
            vec![0.0, 1.0, g, 0.0, 0.0, g],
        ]),
        8 => Some([
            vec![1.0, 0.0, g, 0.0, g, 0.0, g, 0.0],
            vec![0.0, 1.0, g, 0.0, 0.0, g, 0.0, g],
        ]),
        _ => None,
    };
    if source == target {
        return (0..target)
            .map(|t| {
                (0..source)
                    .map(|s| if s == t { 1.0 } else { 0.0 })
                    .collect()
            })
            .collect();
    }
    if source == 1 {
        return vec![vec![1.0]; target];
    }
    match (target, stereo) {
        (1, Some([left, right])) => {
            vec![
                left.iter()
                    .zip(&right)
                    .map(|(l, r)| (l + r) * 0.5)
                    .collect(),
            ]
        }
        (1, None) => vec![vec![1.0 / source as f32; source]],
        (2, Some(stereo)) => stereo.into(),
        _ => (0..target)
            .map(|t| {
                let nearest = t.min(source - 1);
                (0..source)
                    .map(|s| if s == nearest { 1.0 } else { 0.0 })
                    .collect()
            })
            .collect(),
    }
}

fn convert_channels(audio: &mut AudioBuffer, target_channels: u16) -> Result<()> {
    if target_channels == 0 {
        bail!("channel count must be > 0");
    }
    if target_channels == audio.spec.channels {
        return Ok(());
    }

    let source_channels = usize::from(audio.spec.channels);
    if source_channels == 0 {
        bail!("cannot convert audio with zero channels");
    }
    let target_channels_usize = usize::from(target_channels);
    let frames = audio.frames();
    let output_len = frames
        .checked_mul(target_channels_usize)
        .ok_or_else(|| anyhow::anyhow!("channel-converted audio is too large"))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .context("not enough memory to convert channels")?;

    let matrix = channel_matrix(source_channels, target_channels_usize);
    for frame in audio.samples.chunks_exact(source_channels) {
        for row in &matrix {
            output.push(
                row.iter()
                    .zip(frame)
                    .filter(|(gain, _)| **gain != 0.0)
                    .map(|(gain, sample)| gain * sample)
                    .sum::<f32>(),
            );
        }
    }

    audio.spec.channels = target_channels;
    audio.samples = output;
    Ok(())
}

fn scale_frame(audio: &mut AudioBuffer, frame: usize, channels: usize, factor: f32) {
    let start = frame * channels;
    for sample in &mut audio.samples[start..start + channels] {
        *sample *= factor;
    }
}

fn contiguous_silence(
    frames: &[&[f32]],
    min_frames: usize,
    silent: &dyn Fn(&[f32]) -> bool,
    reverse: bool,
) -> usize {
    let mut count = 0;
    let iter: Box<dyn Iterator<Item = &&[f32]>> = if reverse {
        Box::new(frames.iter().rev())
    } else {
        Box::new(frames.iter())
    };

    for frame in iter {
        if silent(frame) {
            count += 1;
        } else {
            break;
        }
    }

    if count >= min_frames { count } else { 0 }
}
