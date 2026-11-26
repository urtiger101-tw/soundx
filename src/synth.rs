use crate::{
    audio::{AudioBuffer, AudioSpec},
    cli::{SynthArgs, Waveform},
};
use anyhow::{Context, Result, anyhow, bail};

pub fn render(args: &SynthArgs) -> Result<AudioBuffer> {
    if !args.duration.is_finite() || args.duration < 0.0 {
        bail!(
            "synth duration must be a finite number of seconds >= 0 (got {})",
            args.duration
        );
    }
    if !args.freq.is_finite() || args.freq < 0.0 {
        bail!(
            "synth frequency must be finite and >= 0 (got {})",
            args.freq
        );
    }
    if args.rate == 0 {
        bail!("synth sample rate must be > 0");
    }
    if args.channels == 0 {
        bail!("synth channel count must be > 0");
    }
    if !(0.0..=1.0).contains(&args.amplitude) {
        bail!("synth amplitude must be between 0 and 1");
    }

    let frames_f = (f64::from(args.duration) * f64::from(args.rate)).round();
    let channels = usize::from(args.channels);
    let too_long = || anyhow!("synth duration of {} s is too long", args.duration);
    if frames_f >= usize::MAX as f64 {
        return Err(too_long());
    }
    let frames = frames_f as usize;
    let sample_count = frames.checked_mul(channels).ok_or_else(too_long)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(sample_count)
        .map_err(|_| too_long())
        .context("not enough memory to synthesize audio")?;
    let mut noise = Noise::new(0x5255_5354_534f_5821);

    let freq = f64::from(args.freq);
    for frame in 0..frames {
        let phase = phase_at(frame as u64, freq, args.rate);
        let sample = waveform_sample(args.waveform, phase, &mut noise) * args.amplitude;
        samples.extend(std::iter::repeat_n(sample, channels));
    }

    Ok(AudioBuffer {
        spec: AudioSpec {
            sample_rate: args.rate,
            channels: args.channels,
        },
        samples,
    })
}

/// Oscillator phase in cycles, in [0, 1), at `frame` for a tone of `freq` Hz.
///
/// Computed in f64 from the absolute frame index (no accumulated error), so
/// the phase stays accurate for very long renders: an f32 `t * freq` loses
/// about 6 dB of SNR per doubling of the elapsed time.
fn phase_at(frame: u64, freq: f64, rate: u32) -> f64 {
    let rate = u64::from(rate);
    // Split the frame into whole seconds and a remainder so the products stay
    // small: frac(freq * (s + r / rate)) = frac(frac(freq * s) + freq * r / rate).
    let seconds = frame / rate;
    let remainder = frame % rate;
    let whole = (freq * seconds as f64).fract();
    (whole + freq * remainder as f64 / rate as f64).fract()
}

fn waveform_sample(waveform: Waveform, phase: f64, noise: &mut Noise) -> f32 {
    match waveform {
        Waveform::Sine => (phase * std::f64::consts::TAU).sin() as f32,
        Waveform::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        Waveform::Triangle => (4.0 * (phase - 0.5).abs() - 1.0) as f32,
        Waveform::Saw => (2.0 * phase - 1.0) as f32,
        Waveform::Noise => noise.next(),
        Waveform::Silence => 0.0,
    }
}

struct Noise {
    state: u64,
}

impl Noise {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let value = (self.state >> 32) as u32;
        (value as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args(duration: f32, freq: f32, rate: u32) -> SynthArgs {
        SynthArgs {
            output: PathBuf::from("unused.wav"),
            duration,
            freq,
            rate,
            channels: 1,
            amplitude: 1.0,
            waveform: Waveform::Sine,
            gain_db: None,
            normalize: false,
            fade: None,
            stat_json: false,
        }
    }

    /// SNR in dB of `actual` against `reference`.
    fn snr_db(actual: &[f32], reference: &[f64]) -> f64 {
        let signal: f64 = reference.iter().map(|value| value * value).sum();
        let noise: f64 = actual
            .iter()
            .zip(reference)
            .map(|(a, r)| (f64::from(*a) - r).powi(2))
            .sum();
        10.0 * (signal / noise.max(1.0e-300)).log10()
    }

    #[test]
    fn long_sine_keeps_phase_precision_in_the_last_second() {
        // 400 s at a low sample rate keeps the test fast; the old f32 phase
        // computation drops below 20 dB SNR here.
        let rate = 4_000;
        let freq = 997.0_f32;
        let audio = render(&args(400.0, freq, rate)).unwrap();
        let frames = audio.frames();
        assert_eq!(frames, 400 * rate as usize);
        let first = frames - rate as usize;
        // Exact reference: the phase of an integer frequency is
        // (frame * freq mod rate) / rate, computed in integers.
        let reference: Vec<f64> = (first..frames)
            .map(|frame| {
                let cycles = (frame as u64 * 997) % u64::from(rate);
                (std::f64::consts::TAU * cycles as f64 / f64::from(rate)).sin()
            })
            .collect();
        let snr = snr_db(&audio.samples[first..], &reference);
        assert!(snr > 80.0, "SNR of the last second is {snr:.1} dB");
    }

    /// Distance between two phases on the unit circle (in cycles).
    fn phase_distance(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(1.0);
        d.min(1.0 - d)
    }

    #[test]
    fn phase_matches_exact_rational_phase_for_huge_frame_indices() {
        let rate = 48_000_u32;
        for frame in [
            0_u64,
            1,
            47_999,
            48_000,
            2_880_000_000,
            86_400 * 48_000 * 7 + 123,
        ] {
            let exact = ((frame * 1_001) % u64::from(rate)) as f64 / f64::from(rate);
            let phase = phase_at(frame, 1_001.0, rate);
            assert!((0.0..1.0).contains(&phase));
            assert!(
                phase_distance(phase, exact) < 1.0e-9,
                "frame {frame}: {phase} vs {exact}"
            );
        }
        // Non-integer frequency (440.25 Hz = 44025/100) at frame 1e9 + 7.
        let frame = 1_000_000_007_u64;
        let phase = phase_at(frame, 440.25, rate);
        let exact = ((frame * 44_025) % (100 * u64::from(rate))) as f64 / (100.0 * f64::from(rate));
        assert!(phase_distance(phase, exact) < 1.0e-9, "{phase} vs {exact}");
    }

    #[test]
    fn rejects_non_finite_and_huge_parameters() {
        assert!(render(&args(f32::NAN, 440.0, 8_000)).is_err());
        assert!(render(&args(f32::INFINITY, 440.0, 8_000)).is_err());
        assert!(render(&args(1.0, f32::NAN, 8_000)).is_err());
        let error = render(&args(1.0e30, 440.0, 8_000)).unwrap_err();
        assert!(format!("{error:#}").contains("too long"), "{error:#}");
    }
}
