use anyhow::Result;
use soundx::audio::{AudioBuffer, AudioSpec};
use soundx::effects::{BiquadKind, Effect, EffectChain, FilterWidth};
use std::f32::consts::TAU;

mod common;
use common::*;

fn constant_buffer(frames: usize, channels: u16, sample_rate: u32, value: f32) -> AudioBuffer {
    AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels,
        },
        samples: vec![value; frames * channels as usize],
    }
}

// ---------------------------------------------------------------------------
// GainDb
// ---------------------------------------------------------------------------

#[test]
fn test_gain_0db_identity() -> Result<()> {
    let mut buf = constant_buffer(10, 1, 100, 0.5);
    EffectChain::new(vec![Effect::GainDb(0.0)]).apply(&mut buf)?;
    assert!(buf.samples.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    Ok(())
}

#[test]
fn test_gain_plus_6db_doubles_amplitude() -> Result<()> {
    let mut buf = constant_buffer(10, 1, 100, 0.5);
    EffectChain::new(vec![Effect::GainDb(6.0)]).apply(&mut buf)?;
    assert!((buf.peak() - 1.0).abs() < 0.01);
    Ok(())
}

#[test]
fn test_gain_minus_6db_halves_amplitude() -> Result<()> {
    let mut buf = constant_buffer(10, 1, 100, 1.0);
    EffectChain::new(vec![Effect::GainDb(-6.0)]).apply(&mut buf)?;
    assert!((buf.peak() - 0.5).abs() < 0.01);
    Ok(())
}

#[test]
fn test_gain_very_negative_approaches_zero() -> Result<()> {
    let mut buf = constant_buffer(10, 1, 100, 1.0);
    EffectChain::new(vec![Effect::GainDb(-100.0)]).apply(&mut buf)?;
    assert!(buf.peak() < 0.0001);
    Ok(())
}

#[test]
fn test_gain_stereo_applies_to_all_channels() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![0.2, 0.4, 0.6, 0.8],
    };
    EffectChain::new(vec![Effect::GainDb(6.0)]).apply(&mut buf)?;
    for (i, &s) in buf.samples.iter().enumerate() {
        let orig = [0.2, 0.4, 0.6, 0.8][i];
        assert!((s - orig * 2.0).abs() < 0.01, "channel {i}: got {s}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Normalize
// ---------------------------------------------------------------------------

#[test]
fn test_normalize_to_target_db() -> Result<()> {
    let mut buf = constant_buffer(100, 2, 44100, 0.5);
    EffectChain::new(vec![Effect::Normalize { target_db: -6.0 }]).apply(&mut buf)?;
    let expected = 10.0_f32.powf(-6.0 / 20.0);
    assert!((buf.peak() - expected).abs() < 0.001);
    Ok(())
}

#[test]
fn test_normalize_silent_audio_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 44100,
            channels: 1,
        },
        samples: vec![0.0; 100],
    };
    EffectChain::new(vec![Effect::Normalize { target_db: -6.0 }]).apply(&mut buf)?;
    assert!(buf.samples.iter().all(|&s| s == 0.0));
    Ok(())
}

#[test]
fn test_normalize_already_normalized_is_unchanged() -> Result<()> {
    let target_db = -6.0;
    let target = 10.0_f32.powf(target_db / 20.0);
    let mut buf = constant_buffer(100, 1, 44100, target);
    let original = buf.clone();
    EffectChain::new(vec![Effect::Normalize { target_db }]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

// ---------------------------------------------------------------------------
// Trim
// ---------------------------------------------------------------------------

#[test]
fn test_trim_from_start() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: (0..10).map(|i| i as f32).collect(),
    };
    EffectChain::new(vec![Effect::Trim {
        start_sec: 0.3,
        duration_sec: None,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    Ok(())
}

#[test]
fn test_trim_with_duration() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: (0..10).map(|i| i as f32).collect(),
    };
    EffectChain::new(vec![Effect::Trim {
        start_sec: 0.2,
        duration_sec: Some(0.4),
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![2.0, 3.0, 4.0, 5.0]);
    Ok(())
}

#[test]
fn test_trim_beyond_end_returns_empty() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: (0..10).map(|i| i as f32).collect(),
    };
    EffectChain::new(vec![Effect::Trim {
        start_sec: 2.0,
        duration_sec: None,
    }])
    .apply(&mut buf)?;
    assert!(buf.samples.is_empty());
    Ok(())
}

#[test]
fn test_trim_zero_duration_returns_empty() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: (0..10).map(|i| i as f32).collect(),
    };
    EffectChain::new(vec![Effect::Trim {
        start_sec: 0.0,
        duration_sec: Some(0.0),
    }])
    .apply(&mut buf)?;
    assert!(buf.samples.is_empty());
    Ok(())
}

#[test]
fn test_trim_negative_start_errors() {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![0.0; 10],
    };
    let result = EffectChain::new(vec![Effect::Trim {
        start_sec: -1.0,
        duration_sec: None,
    }])
    .apply(&mut buf);
    assert!(result.is_err());
}

#[test]
fn test_trim_stereo_preserves_channels() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 2,
        },
        samples: vec![1.0, 10.0, 2.0, 20.0, 3.0, 30.0, 4.0, 40.0],
    };
    EffectChain::new(vec![Effect::Trim {
        start_sec: 0.1,
        duration_sec: Some(0.2),
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.spec.channels, 2);
    assert_eq!(buf.samples, vec![2.0, 20.0, 3.0, 30.0]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Fade
// ---------------------------------------------------------------------------

#[test]
fn test_fade_in_ramps_correctly() -> Result<()> {
    let mut buf = constant_buffer(10, 1, 10, 1.0);
    EffectChain::new(vec![Effect::Fade {
        in_sec: 0.5,
        out_sec: 0.0,
    }])
    .apply(&mut buf)?;
    let expected: Vec<f32> = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.0, 1.0, 1.0, 1.0];
    assert_buffer_eq(&buf.samples, &expected, 1e-6, "fade in");
    Ok(())
}

#[test]
fn test_fade_out_ramps_correctly() -> Result<()> {
    let mut buf = constant_buffer(10, 1, 10, 1.0);
    EffectChain::new(vec![Effect::Fade {
        in_sec: 0.0,
        out_sec: 0.5,
    }])
    .apply(&mut buf)?;
    let expected: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.8, 0.6, 0.4, 0.2];
    assert_buffer_eq(&buf.samples, &expected, 1e-6, "fade out");
    Ok(())
}

#[test]
fn test_fade_both_ends() -> Result<()> {
    let mut buf = constant_buffer(10, 2, 10, 1.0);
    EffectChain::new(vec![Effect::Fade {
        in_sec: 0.5,
        out_sec: 0.5,
    }])
    .apply(&mut buf)?;
    let expected: Vec<f32> = vec![
        0.0, 0.0, 0.2, 0.2, 0.4, 0.4, 0.6, 0.6, 0.8, 0.8, 1.0, 1.0, 0.8, 0.8, 0.6, 0.6, 0.4, 0.4,
        0.2, 0.2,
    ];
    assert_buffer_eq(&buf.samples, &expected, 1e-6, "fade both");
    Ok(())
}

#[test]
fn test_fade_zero_durations_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![0.3, 0.6, 0.9],
    };
    let original = buf.clone();
    EffectChain::new(vec![Effect::Fade {
        in_sec: 0.0,
        out_sec: 0.0,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

#[test]
fn test_fade_negative_durations_error() {
    let mut buf = constant_buffer(10, 1, 10, 1.0);
    let result = EffectChain::new(vec![Effect::Fade {
        in_sec: -0.1,
        out_sec: 0.0,
    }])
    .apply(&mut buf);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Reverse
// ---------------------------------------------------------------------------

#[test]
fn test_reverse_changes_order() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 2,
        },
        samples: vec![1.0, 10.0, 2.0, 20.0, 3.0, 30.0],
    };
    EffectChain::new(vec![Effect::Reverse]).apply(&mut buf)?;
    assert_eq!(buf.samples, vec![3.0, 30.0, 2.0, 20.0, 1.0, 10.0]);
    Ok(())
}

#[test]
fn test_reverse_twice_is_identity() -> Result<()> {
    let mut buf = test_buffer_with_samples(50, 2, 44100, 0.5);
    let original = buf.clone();
    EffectChain::new(vec![Effect::Reverse, Effect::Reverse]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

#[test]
fn test_reverse_empty_buffer_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 44100,
            channels: 2,
        },
        samples: vec![],
    };
    EffectChain::new(vec![Effect::Reverse]).apply(&mut buf)?;
    assert!(buf.samples.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// Speed
// ---------------------------------------------------------------------------

#[test]
fn test_speed_half_doubles_frames() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 1, 100, 0.5);
    EffectChain::new(vec![Effect::Speed { factor: 0.5 }]).apply(&mut buf)?;
    assert_eq!(buf.frames(), 200);
    Ok(())
}

#[test]
fn test_speed_double_halves_frames() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 1, 100, 0.5);
    EffectChain::new(vec![Effect::Speed { factor: 2.0 }]).apply(&mut buf)?;
    assert_eq!(buf.frames(), 50);
    Ok(())
}

#[test]
fn test_speed_one_is_identity() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 44100, 0.5);
    let original = buf.clone();
    EffectChain::new(vec![Effect::Speed { factor: 1.0 }]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

#[test]
fn test_speed_negative_factor_errors() {
    let mut buf = test_buffer_with_samples(100, 1, 100, 0.5);
    let result = EffectChain::new(vec![Effect::Speed { factor: -1.0 }]).apply(&mut buf);
    assert!(result.is_err());
}

#[test]
fn test_speed_zero_factor_errors() {
    let mut buf = test_buffer_with_samples(100, 1, 100, 0.5);
    let result = EffectChain::new(vec![Effect::Speed { factor: 0.0 }]).apply(&mut buf);
    assert!(result.is_err());
}

#[test]
fn test_speed_stereo_preserves_channels() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 100, 0.5);
    EffectChain::new(vec![Effect::Speed { factor: 0.5 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.channels, 2);
    assert_eq!(buf.frames(), 200);
    assert_eq!(buf.samples.len(), 400);
    Ok(())
}

#[test]
fn dither_adds_tpdf_noise_and_quantizes_to_requested_precision() -> Result<()> {
    let mut buf = constant_buffer(2_000, 1, 48_000, 0.0);
    EffectChain::new(vec![Effect::Dither { bits: 16 }]).apply(&mut buf)?;
    assert!(buf.samples.iter().any(|sample| *sample != 0.0));
    assert!(
        buf.samples
            .iter()
            .all(|sample| ((*sample * 32768.0).round() / 32768.0 - *sample).abs() < 1.0e-7)
    );
    Ok(())
}

#[test]
fn compand_applies_the_configured_transfer_curve() -> Result<()> {
    let mut buf = constant_buffer(4_800, 1, 48_000, 0.5);
    EffectChain::new(vec![Effect::Compand {
        attack_decay: vec![(0.0, 0.0)],
        transfer_points: vec![(-80.0, -80.0), (-20.0, -20.0), (0.0, -10.0)],
        knee_db: 0.0,
        gain_db: 0.0,
        initial_volume_db: 0.0,
        delay_sec: 0.0,
    }])
    .apply(&mut buf)?;
    assert!(
        buf.peak() > 0.18 && buf.peak() < 0.28,
        "peak was {}",
        buf.peak()
    );
    Ok(())
}

#[test]
fn reverb_adds_a_tail_and_produces_a_wet_signal() -> Result<()> {
    let mut buf = constant_buffer(500, 1, 8_000, 0.5);
    EffectChain::new(vec![Effect::Reverb {
        wet_only: true,
        reverberance: 70.0,
        hf_damping: 40.0,
        room_scale: 80.0,
        stereo_depth: 100.0,
        pre_delay_ms: 5.0,
        wet_gain_db: 0.0,
    }])
    .apply(&mut buf)?;
    assert!(buf.frames() > 500);
    assert!(buf.samples.iter().any(|sample| sample.abs() > 1.0e-5));
    Ok(())
}

#[test]
fn stretch_and_tempo_change_duration_without_resampling_pitch() -> Result<()> {
    let sample_rate = 8_000;
    let mut stretched = AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels: 1,
        },
        samples: (0..sample_rate as usize)
            .map(|frame| (TAU * 440.0 * frame as f32 / sample_rate as f32).sin() * 0.5)
            .collect(),
    };
    EffectChain::new(vec![Effect::Stretch {
        factor: 1.5,
        window_ms: 40.0,
        search_ms: 8.0,
        overlap_ms: 10.0,
    }])
    .apply(&mut stretched)?;
    assert_eq!(stretched.frames(), 12_000);
    let crossings = stretched
        .samples
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let measured_hz = crossings as f32 / stretched.duration_seconds() as f32;
    assert!(
        (400.0..480.0).contains(&measured_hz),
        "measured pitch {measured_hz}"
    );

    let mut tempo = stretched.clone();
    EffectChain::new(vec![Effect::Tempo {
        factor: 2.0,
        quality: soundx::effects::TempoQuality::Quick,
        segment_ms: 30.0,
        search_ms: 5.0,
        overlap_ms: 10.0,
    }])
    .apply(&mut tempo)?;
    assert_eq!(tempo.frames(), 6_000);
    let tempo_crossings = tempo
        .samples
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let tempo_hz = tempo_crossings as f32 / tempo.duration_seconds() as f32;
    assert!(
        (400.0..480.0).contains(&tempo_hz),
        "measured tempo pitch {tempo_hz}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Pad
// ---------------------------------------------------------------------------

#[test]
fn test_pad_start_adds_silence() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![1.0, 2.0, 3.0],
    };
    EffectChain::new(vec![Effect::Pad {
        start_sec: 0.2,
        end_sec: 0.0,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![0.0, 0.0, 1.0, 2.0, 3.0]);
    Ok(())
}

#[test]
fn test_pad_end_adds_silence() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 2,
        },
        samples: vec![1.0, 2.0, 3.0, 4.0],
    };
    EffectChain::new(vec![Effect::Pad {
        start_sec: 0.0,
        end_sec: 0.1,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![1.0, 2.0, 3.0, 4.0, 0.0, 0.0]);
    Ok(())
}

#[test]
fn test_pad_both_sides() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![5.0],
    };
    EffectChain::new(vec![Effect::Pad {
        start_sec: 0.1,
        end_sec: 0.2,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![0.0, 5.0, 0.0, 0.0]);
    Ok(())
}

#[test]
fn test_pad_zero_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 2,
        },
        samples: vec![0.5, -0.5],
    };
    let original = buf.clone();
    EffectChain::new(vec![Effect::Pad {
        start_sec: 0.0,
        end_sec: 0.0,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

// ---------------------------------------------------------------------------
// Silence (trim_silence)
// ---------------------------------------------------------------------------

#[test]
fn test_trim_silence_removes_leading_silence() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0],
    };
    EffectChain::new(vec![Effect::Silence {
        threshold_db: -60.0,
        min_duration_sec: 0.1,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![1.0, 2.0, 3.0]);
    Ok(())
}

#[test]
fn test_trim_silence_removes_trailing_silence() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0],
    };
    EffectChain::new(vec![Effect::Silence {
        threshold_db: -60.0,
        min_duration_sec: 0.2,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![1.0, 2.0, 3.0]);
    Ok(())
}

#[test]
fn test_trim_silence_no_silence_is_noop() -> Result<()> {
    let samples: Vec<f32> = (0..10).map(|i| (i as f32 + 1.0) * 0.1).collect();
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: samples.clone(),
    };
    EffectChain::new(vec![Effect::Silence {
        threshold_db: -60.0,
        min_duration_sec: 0.1,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, samples);
    Ok(())
}

#[test]
fn test_trim_silence_all_silent_returns_empty() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![0.0; 10],
    };
    EffectChain::new(vec![Effect::Silence {
        threshold_db: -60.0,
        min_duration_sec: 0.1,
    }])
    .apply(&mut buf)?;
    assert!(buf.samples.is_empty());
    Ok(())
}

#[test]
fn test_trim_silence_below_min_duration_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![0.0, 0.0, 1.0],
    };
    EffectChain::new(vec![Effect::Silence {
        threshold_db: -60.0,
        min_duration_sec: 1.0,
    }])
    .apply(&mut buf)?;
    assert_eq!(buf.samples, vec![0.0, 0.0, 1.0]);
    Ok(())
}

// ---------------------------------------------------------------------------
// LowPass
// ---------------------------------------------------------------------------

#[test]
fn test_lowpass_dc_preserved() -> Result<()> {
    let mut buf = constant_buffer(1000, 1, 100, 1.0);
    EffectChain::new(vec![Effect::LowPass { hz: 20.0 }]).apply(&mut buf)?;
    let last = buf.samples[buf.samples.len() - 1];
    assert!((last - 1.0).abs() < 0.01, "DC not preserved: {last}");
    Ok(())
}

#[test]
fn test_lowpass_high_freq_attenuated() -> Result<()> {
    let sample_rate = 1000;
    let signal_hz = 100.0;
    let mut samples = Vec::with_capacity(sample_rate as usize);
    for i in 0..sample_rate {
        let t = i as f32 / sample_rate as f32;
        samples.push((t * signal_hz * TAU).sin());
    }
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels: 1,
        },
        samples,
    };
    let peak_before = buf.peak();
    EffectChain::new(vec![Effect::LowPass { hz: 20.0 }]).apply(&mut buf)?;
    assert!(buf.peak() < peak_before * 0.5);
    Ok(())
}

#[test]
fn test_lowpass_zero_hz_errors() {
    let mut buf = constant_buffer(10, 1, 100, 1.0);
    let result = EffectChain::new(vec![Effect::LowPass { hz: 0.0 }]).apply(&mut buf);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// HighPass
// ---------------------------------------------------------------------------

#[test]
fn test_highpass_dc_removed() -> Result<()> {
    let mut buf = constant_buffer(1000, 1, 100, 1.0);
    EffectChain::new(vec![Effect::HighPass { hz: 20.0 }]).apply(&mut buf)?;
    let last = buf.samples[buf.samples.len() - 1];
    assert!(last.abs() < 0.01, "DC not removed: {last}");
    Ok(())
}

#[test]
fn test_highpass_high_freq_preserved() -> Result<()> {
    let sample_rate = 1000;
    let signal_hz = 100.0;
    let mut samples = Vec::with_capacity(sample_rate as usize);
    for i in 0..sample_rate {
        let t = i as f32 / sample_rate as f32;
        samples.push((t * signal_hz * TAU).sin());
    }
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels: 1,
        },
        samples,
    };
    let peak_before = buf.peak();
    EffectChain::new(vec![Effect::HighPass { hz: 5.0 }]).apply(&mut buf)?;
    assert!(buf.peak() > peak_before * 0.8);
    Ok(())
}

#[test]
fn test_echo_extends_audio_and_applies_multiple_taps() -> Result<()> {
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 1000,
            channels: 1,
        },
        samples: vec![1.0, 0.0, 0.0, 0.0],
    };
    EffectChain::new(vec![Effect::Echo {
        input_gain: 0.5,
        output_gain: 1.0,
        taps_ms: vec![(2.0, 0.25), (4.0, 0.125)],
    }])
    .apply(&mut audio)?;

    assert_eq!(audio.samples.len(), 8);
    assert_eq!(audio.samples[0], 0.5);
    assert_eq!(audio.samples[2], 0.25);
    assert_eq!(audio.samples[4], 0.125);
    Ok(())
}

#[test]
fn test_tremolo_applies_stereo_linked_lfo() -> Result<()> {
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![1.0; 8],
    };
    EffectChain::new(vec![Effect::Tremolo {
        speed_hz: 25.0,
        depth_percent: 100.0,
    }])
    .apply(&mut audio)?;

    assert!((audio.samples[0] - 0.5).abs() < 1e-6);
    assert!((audio.samples[2] - 1.0).abs() < 1e-6);
    assert!((audio.samples[4] - 0.5).abs() < 1e-6);
    assert!(audio.samples[6].abs() < 1e-6);
    assert_eq!(audio.samples[0], audio.samples[1]);
    assert_eq!(audio.samples[2], audio.samples[3]);
    Ok(())
}

#[test]
fn test_tremolo_rejects_invalid_depth() {
    let mut audio = constant_buffer(4, 1, 100, 1.0);
    let result = EffectChain::new(vec![Effect::Tremolo {
        speed_hz: 2.0,
        depth_percent: 101.0,
    }])
    .apply(&mut audio);
    assert!(result.is_err());
}

#[test]
fn test_delay_offsets_samples_and_extends_the_buffer() -> Result<()> {
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 1000,
            channels: 1,
        },
        samples: vec![1.0, 2.0],
    };
    EffectChain::new(vec![Effect::Delay {
        delays_sec: vec![0.002],
    }])
    .apply(&mut audio)?;
    assert_eq!(audio.samples, vec![0.0, 0.0, 1.0, 2.0]);
    Ok(())
}

#[test]
fn test_dcshift_applies_shift_and_limits_clipping() -> Result<()> {
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 1000,
            channels: 1,
        },
        samples: vec![-0.5, 0.5, 1.0],
    };
    EffectChain::new(vec![Effect::DcShift {
        shift: 0.25,
        limiter_gain: Some(0.1),
    }])
    .apply(&mut audio)?;
    assert!((audio.samples[0] + 0.25).abs() < 1e-6);
    assert!((audio.samples[1] - 0.75).abs() < 1e-6);
    assert_eq!(audio.samples[2], 1.0);
    Ok(())
}

#[test]
fn test_downsample_and_upsample_match_sample_insertion_semantics() -> Result<()> {
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 1000,
            channels: 1,
        },
        samples: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
    };
    EffectChain::new(vec![Effect::Downsample { factor: 2 }]).apply(&mut audio)?;
    assert_eq!(audio.spec.sample_rate, 500);
    assert_eq!(audio.samples, vec![1.0, 3.0, 5.0]);

    EffectChain::new(vec![Effect::Upsample { factor: 2 }]).apply(&mut audio)?;
    assert_eq!(audio.spec.sample_rate, 1000);
    assert_eq!(audio.samples, vec![1.0, 0.0, 3.0, 0.0, 5.0, 0.0]);
    Ok(())
}

#[test]
fn test_repeat_and_swap_preserve_channel_frames() -> Result<()> {
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 1000,
            channels: 2,
        },
        samples: vec![1.0, 10.0, 2.0, 20.0],
    };
    EffectChain::new(vec![Effect::Swap, Effect::Repeat { count: 1 }]).apply(&mut audio)?;
    assert_eq!(
        audio.samples,
        vec![10.0, 1.0, 20.0, 2.0, 10.0, 1.0, 20.0, 2.0]
    );
    Ok(())
}

#[test]
fn test_highpass_zero_hz_errors() {
    let mut buf = constant_buffer(10, 1, 100, 1.0);
    let result = EffectChain::new(vec![Effect::HighPass { hz: 0.0 }]).apply(&mut buf);
    assert!(result.is_err());
}

#[test]
fn test_biquad_lowpass_and_highpass_have_expected_dc_response() -> Result<()> {
    let width = FilterWidth::Q(0.707);
    let mut low = constant_buffer(4096, 1, 48_000, 0.25);
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::LowPass,
        hz: 1000.0,
        width,
        gain_db: 0.0,
        poles: 2,
    }])
    .apply(&mut low)?;
    assert!((low.samples.last().copied().unwrap() - 0.25).abs() < 1e-4);

    let mut high = constant_buffer(4096, 1, 48_000, 0.25);
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::HighPass,
        hz: 1000.0,
        width,
        gain_db: 0.0,
        poles: 2,
    }])
    .apply(&mut high)?;
    assert!(high.samples.last().copied().unwrap().abs() < 1e-4);
    Ok(())
}

#[test]
fn test_bass_shelf_boosts_dc_and_treble_shelf_preserves_it() -> Result<()> {
    let mut bass = constant_buffer(8192, 1, 48_000, 0.1);
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::LowShelf,
        hz: 100.0,
        width: FilterWidth::Slope(0.5),
        gain_db: 6.0,
        poles: 2,
    }])
    .apply(&mut bass)?;
    let expected = 0.1 * 10.0_f32.powf(6.0 / 20.0);
    assert!((bass.samples.last().copied().unwrap() - expected).abs() < 1e-3);

    let mut treble = constant_buffer(8192, 1, 48_000, 0.1);
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::HighShelf,
        hz: 3000.0,
        width: FilterWidth::Slope(0.5),
        gain_db: 6.0,
        poles: 2,
    }])
    .apply(&mut treble)?;
    assert!((treble.samples.last().copied().unwrap() - 0.1).abs() < 1e-3);
    Ok(())
}

#[test]
fn test_biquad_equalizer_boosts_the_center_frequency() -> Result<()> {
    let sample_rate = 48_000;
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels: 1,
        },
        samples: (0..sample_rate)
            .map(|index| (TAU * 1000.0 * index as f32 / sample_rate as f32).sin() * 0.1)
            .collect(),
    };
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::Equalizer,
        hz: 1000.0,
        width: FilterWidth::Q(1.0),
        gain_db: 6.0,
        poles: 2,
    }])
    .apply(&mut audio)?;
    let before_rms = ((0..sample_rate as usize)
        .map(|index| {
            (TAU * 1000.0 * index as f32 / sample_rate as f32)
                .sin()
                .mul_add(0.1, 0.0)
                .powi(2)
        })
        .sum::<f32>()
        / sample_rate as f32)
        .sqrt();
    let after_rms = (audio
        .samples
        .iter()
        .map(|sample| sample * sample)
        .sum::<f32>()
        / sample_rate as f32)
        .sqrt();
    assert!(after_rms > before_rms * 1.7);
    Ok(())
}

#[test]
fn test_bandpass_modes_follow_peak_and_constant_skirt_gain() -> Result<()> {
    let sample_rate = 48_000;
    let make_tone = || AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels: 1,
        },
        samples: (0..sample_rate)
            .map(|index| (TAU * 1000.0 * index as f32 / sample_rate as f32).sin() * 0.1)
            .collect(),
    };
    let rms = |samples: &[f32]| {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    };
    let input_rms = rms(&make_tone().samples);
    let mut peak = make_tone();
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::BandPass,
        hz: 1000.0,
        width: FilterWidth::Q(3.0),
        gain_db: 0.0,
        poles: 2,
    }])
    .apply(&mut peak)?;
    let peak_gain = rms(&peak.samples) / input_rms;

    let mut skirt = make_tone();
    EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::BandPassConstantSkirt,
        hz: 1000.0,
        width: FilterWidth::Q(3.0),
        gain_db: 0.0,
        poles: 2,
    }])
    .apply(&mut skirt)?;
    let skirt_gain = rms(&skirt.samples) / input_rms;
    assert!((peak_gain - 1.0).abs() < 0.05);
    assert!((skirt_gain - 3.0).abs() < 0.1);
    Ok(())
}

#[test]
fn test_biquad_rejects_frequencies_at_or_above_nyquist() {
    let mut audio = constant_buffer(32, 1, 1000, 0.5);
    let result = EffectChain::new(vec![Effect::BiquadFilter {
        kind: BiquadKind::LowPass,
        hz: 500.0,
        width: FilterWidth::Q(0.707),
        gain_db: 0.0,
        poles: 2,
    }])
    .apply(&mut audio);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Limiter
// ---------------------------------------------------------------------------

#[test]
fn test_limiter_clips_above_threshold() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 1,
        },
        samples: vec![0.0, 0.3, 0.6, 0.9, 1.0, -0.7, -1.0],
    };
    EffectChain::new(vec![Effect::Limiter { threshold: 0.5 }]).apply(&mut buf)?;
    assert_eq!(buf.samples, vec![0.0, 0.3, 0.5, 0.5, 0.5, -0.5, -0.5]);
    Ok(())
}

#[test]
fn test_limiter_below_threshold_unchanged() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![0.1, -0.2, 0.3, -0.4],
    };
    let original = buf.clone();
    EffectChain::new(vec![Effect::Limiter { threshold: 0.5 }]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

#[test]
fn test_limiter_threshold_zero_silences_all() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![0.5, -0.5, 0.3, -0.3],
    };
    EffectChain::new(vec![Effect::Limiter { threshold: 0.0 }]).apply(&mut buf)?;
    assert!(buf.samples.iter().all(|&s| s == 0.0));
    Ok(())
}

#[test]
fn test_limiter_threshold_one_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 1,
        },
        samples: vec![-0.5, 0.0, 0.5],
    };
    let original = buf.clone();
    EffectChain::new(vec![Effect::Limiter { threshold: 1.0 }]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

#[test]
fn test_limiter_invalid_threshold_errors() {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 1,
        },
        samples: vec![0.5],
    };
    let result = EffectChain::new(vec![Effect::Limiter { threshold: 1.5 }]).apply(&mut buf);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Rate
// ---------------------------------------------------------------------------

#[test]
fn test_rate_half_reduces_frames() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 44100, 0.5);
    EffectChain::new(vec![Effect::Rate { sample_rate: 22050 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.sample_rate, 22050);
    assert_eq!(buf.frames(), 50);
    Ok(())
}

#[test]
fn test_rate_double_increases_frames() -> Result<()> {
    let mut buf = test_buffer_with_samples(50, 1, 22050, 0.5);
    EffectChain::new(vec![Effect::Rate { sample_rate: 44100 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.sample_rate, 44100);
    assert_eq!(buf.frames(), 100);
    Ok(())
}

#[test]
fn test_rate_same_is_noop() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 44100, 0.5);
    let original = buf.clone();
    EffectChain::new(vec![Effect::Rate { sample_rate: 44100 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.sample_rate, original.spec.sample_rate);
    assert_eq!(buf.spec.channels, original.spec.channels);
    assert_eq!(buf.samples, original.samples);
    Ok(())
}

#[test]
fn test_rate_zero_errors() {
    let mut buf = test_buffer_with_samples(10, 1, 44100, 0.5);
    let result = EffectChain::new(vec![Effect::Rate { sample_rate: 0 }]).apply(&mut buf);
    assert!(result.is_err());
}

#[test]
fn test_rate_stereo_preserves_channels() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 44100, 0.5);
    EffectChain::new(vec![Effect::Rate { sample_rate: 22050 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.channels, 2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------------

#[test]
fn test_channels_stereo_to_mono() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![1.0, 3.0, 2.0, 4.0, 5.0, 7.0],
    };
    EffectChain::new(vec![Effect::Channels { channels: 1 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.channels, 1);
    assert_eq!(buf.samples, vec![2.0, 3.0, 6.0]);
    Ok(())
}

#[test]
fn test_channels_mono_to_stereo() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 1,
        },
        samples: vec![1.0, 2.0, 3.0],
    };
    EffectChain::new(vec![Effect::Channels { channels: 2 }]).apply(&mut buf)?;
    assert_eq!(buf.spec.channels, 2);
    assert_eq!(buf.samples, vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
    Ok(())
}

#[test]
fn test_channels_same_is_noop() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![1.0, -1.0, 0.5, -0.5],
    };
    let original = buf.clone();
    EffectChain::new(vec![Effect::Channels { channels: 2 }]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    assert_eq!(buf.spec.channels, 2);
    Ok(())
}

#[test]
fn test_channels_zero_errors() {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 2,
        },
        samples: vec![1.0, 2.0],
    };
    let result = EffectChain::new(vec![Effect::Channels { channels: 0 }]).apply(&mut buf);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

#[test]
fn test_stats_is_noop() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 44100, 0.5);
    let original = buf.clone();
    EffectChain::new(vec![Effect::Stats]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    assert_eq!(buf.spec.sample_rate, original.spec.sample_rate);
    assert_eq!(buf.spec.channels, original.spec.channels);
    Ok(())
}

// ---------------------------------------------------------------------------
// EffectChain
// ---------------------------------------------------------------------------

#[test]
fn test_chain_multiple_effects_apply_in_order() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![0.0, 0.5, 1.0],
    };
    EffectChain::new(vec![Effect::GainDb(6.0), Effect::Reverse]).apply(&mut buf)?;
    let factor = 10.0_f32.powf(6.0 / 20.0);
    assert!((buf.samples[0] - 1.0 * factor).abs() < 1e-6);
    assert!((buf.samples[1] - 0.5 * factor).abs() < 1e-6);
    assert_eq!(buf.samples[2], 0.0);
    Ok(())
}

#[test]
fn test_chain_empty_is_noop() -> Result<()> {
    let mut buf = test_buffer_with_samples(100, 2, 44100, 0.5);
    let original = buf.clone();
    EffectChain::new(vec![]).apply(&mut buf)?;
    assert_eq!(buf.samples, original.samples);
    assert_eq!(buf.spec.sample_rate, original.spec.sample_rate);
    assert_eq!(buf.spec.channels, original.spec.channels);
    Ok(())
}

#[test]
fn test_chain_builder_with_appends_effects() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: vec![1.0, 2.0, 3.0],
    };
    EffectChain::new(vec![Effect::Reverse])
        .with(Effect::GainDb(-6.0))
        .apply(&mut buf)?;
    // First reverse: [3.0, 2.0, 1.0], then gain -6dB: multiply by ~0.5
    assert!((buf.samples[0] - 1.5).abs() < 0.01);
    assert!((buf.samples[1] - 1.0).abs() < 0.01);
    assert!((buf.samples[2] - 0.5).abs() < 0.01);
    Ok(())
}

#[test]
fn test_wants_stats_true_with_stats() {
    let chain = EffectChain::new(vec![Effect::GainDb(3.0), Effect::Stats]);
    assert!(chain.wants_stats());
}

#[test]
fn test_wants_stats_false_without_stats() {
    let chain = EffectChain::new(vec![Effect::GainDb(3.0), Effect::Reverse]);
    assert!(!chain.wants_stats());
}

// ---------------------------------------------------------------------------
// Edge cases across effects
// ---------------------------------------------------------------------------

#[test]
fn test_empty_buffer_through_all_effects() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 44100,
            channels: 2,
        },
        samples: vec![],
    };
    EffectChain::new(vec![
        Effect::GainDb(6.0),
        Effect::Normalize { target_db: -3.0 },
        Effect::Trim {
            start_sec: 0.0,
            duration_sec: None,
        },
        Effect::Reverse,
        Effect::Speed { factor: 1.5 },
        Effect::Pad {
            start_sec: 0.0,
            end_sec: 0.0,
        },
        Effect::Silence {
            threshold_db: -60.0,
            min_duration_sec: 0.1,
        },
        Effect::LowPass { hz: 100.0 },
        Effect::HighPass { hz: 20.0 },
        Effect::Limiter { threshold: 0.9 },
        Effect::Rate { sample_rate: 22050 },
        Effect::Channels { channels: 1 },
    ])
    .apply(&mut buf)?;
    assert!(buf.samples.is_empty());
    Ok(())
}

#[test]
fn test_single_frame_behaviors() -> Result<()> {
    let mut buf = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 100,
            channels: 1,
        },
        samples: vec![0.5],
    };
    EffectChain::new(vec![
        Effect::GainDb(6.0),
        Effect::Speed { factor: 2.0 },
        Effect::Rate { sample_rate: 200 },
    ])
    .apply(&mut buf)?;
    // speed 2: round(1 / 2) = 1 frame (never fewer than one for non-empty
    // input); rate 100 -> 200: round(1 * 2) = 2 frames.
    assert_eq!(buf.frames(), 2);
    assert_eq!(buf.spec.sample_rate, 200);
    assert!(buf.samples.iter().all(|sample| sample.is_finite()));
    Ok(())
}

// ---------------------------------------------------------------------------
// Parameter validation (NaN / infinity / out of domain)
// ---------------------------------------------------------------------------

fn apply_one(effect: Effect) -> Result<AudioBuffer> {
    let mut buf = constant_buffer(100, 1, 100, 0.5);
    EffectChain::new(vec![effect]).apply(&mut buf)?;
    Ok(buf)
}

#[test]
fn test_effects_reject_non_finite_parameters() {
    let nan = f32::NAN;
    let inf = f32::INFINITY;
    let cases: Vec<(&str, Effect)> = vec![
        ("gain nan", Effect::GainDb(nan)),
        ("gain inf", Effect::GainDb(inf)),
        ("gain -inf", Effect::GainDb(-inf)),
        ("gain overflow", Effect::GainDb(1.0e4)),
        ("normalize nan", Effect::Normalize { target_db: nan }),
        ("normalize inf", Effect::Normalize { target_db: inf }),
        (
            "trim start nan",
            Effect::Trim {
                start_sec: nan,
                duration_sec: None,
            },
        ),
        (
            "trim start inf",
            Effect::Trim {
                start_sec: inf,
                duration_sec: None,
            },
        ),
        (
            "trim duration nan",
            Effect::Trim {
                start_sec: 0.0,
                duration_sec: Some(nan),
            },
        ),
        (
            "trim duration negative",
            Effect::Trim {
                start_sec: 0.0,
                duration_sec: Some(-1.0),
            },
        ),
        (
            "fade in nan",
            Effect::Fade {
                in_sec: nan,
                out_sec: 0.0,
            },
        ),
        (
            "fade out nan",
            Effect::Fade {
                in_sec: 0.0,
                out_sec: nan,
            },
        ),
        (
            "pad start nan",
            Effect::Pad {
                start_sec: nan,
                end_sec: 0.0,
            },
        ),
        (
            "pad end inf",
            Effect::Pad {
                start_sec: 0.0,
                end_sec: inf,
            },
        ),
        ("speed nan", Effect::Speed { factor: nan }),
        ("speed inf", Effect::Speed { factor: inf }),
        ("lowpass nan", Effect::LowPass { hz: nan }),
        ("lowpass inf", Effect::LowPass { hz: inf }),
        ("highpass nan", Effect::HighPass { hz: nan }),
        ("highpass inf", Effect::HighPass { hz: inf }),
        ("limiter nan", Effect::Limiter { threshold: nan }),
        (
            "silence min duration nan",
            Effect::Silence {
                threshold_db: -60.0,
                min_duration_sec: nan,
            },
        ),
        (
            "silence threshold nan",
            Effect::Silence {
                threshold_db: nan,
                min_duration_sec: 0.1,
            },
        ),
    ];
    for (name, effect) in cases {
        assert!(apply_one(effect).is_err(), "{name} was accepted");
    }
}

// ---------------------------------------------------------------------------
// Overflow / huge allocations
// ---------------------------------------------------------------------------

#[test]
fn test_trim_with_huge_duration_behaves_like_open_ended_trim() -> Result<()> {
    let make = || AudioBuffer {
        spec: AudioSpec {
            sample_rate: 10,
            channels: 1,
        },
        samples: (0..10).map(|i| i as f32).collect(),
    };
    for duration in [1.0e30, f32::MAX] {
        let mut huge = make();
        EffectChain::new(vec![Effect::Trim {
            start_sec: 0.2,
            duration_sec: Some(duration),
        }])
        .apply(&mut huge)?;
        let mut open = make();
        EffectChain::new(vec![Effect::Trim {
            start_sec: 0.2,
            duration_sec: None,
        }])
        .apply(&mut open)?;
        assert_eq!(huge.samples, open.samples, "duration {duration}");
        assert_eq!(huge.samples, (2..10).map(|i| i as f32).collect::<Vec<_>>());
    }
    Ok(())
}

#[test]
fn test_pad_with_huge_duration_is_a_readable_error() {
    for (start_sec, end_sec) in [(1.0e30, 0.0), (0.0, 1.0e30), (f32::MAX, f32::MAX)] {
        for channels in [1, 2] {
            let mut buf = constant_buffer(4, channels, 44_100, 0.5);
            let error = EffectChain::new(vec![Effect::Pad { start_sec, end_sec }])
                .apply(&mut buf)
                .unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("too large") || message.contains("not enough memory"),
                "{message}"
            );
        }
    }
}

#[test]
fn test_speed_with_tiny_factor_is_a_readable_error() {
    let mut buf = constant_buffer(100, 2, 44_100, 0.5);
    let error = EffectChain::new(vec![Effect::Speed { factor: 1.0e-30 }])
        .apply(&mut buf)
        .unwrap_err();
    assert!(format!("{error:#}").contains("too large"), "{error:#}");
}

// ---------------------------------------------------------------------------
// Channel downmix (ITU-R BS.775)
// ---------------------------------------------------------------------------

const MINUS_3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

fn one_frame(frame: &[f32]) -> AudioBuffer {
    AudioBuffer {
        spec: AudioSpec {
            sample_rate: 48_000,
            channels: frame.len() as u16,
        },
        samples: frame.to_vec(),
    }
}

fn convert(frame: &[f32], channels: u16) -> Result<Vec<f32>> {
    let mut buf = one_frame(frame);
    EffectChain::new(vec![Effect::Channels { channels }]).apply(&mut buf)?;
    assert_eq!(buf.spec.channels, channels);
    Ok(buf.samples)
}

#[test]
fn test_5_1_centre_only_downmixes_to_both_stereo_channels() -> Result<()> {
    // L R C LFE Ls Rs
    let stereo = convert(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 2)?;
    assert_buffer_eq(&stereo, &[MINUS_3DB, MINUS_3DB], 1e-6, "5.1 centre");
    let mono = convert(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 1)?;
    assert_buffer_eq(&mono, &[MINUS_3DB], 1e-6, "5.1 centre mono");
    Ok(())
}

#[test]
fn test_5_1_downmix_uses_bs775_coefficients_and_drops_lfe() -> Result<()> {
    assert_buffer_eq(
        &convert(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 2)?,
        &[1.0, 0.0],
        1e-6,
        "left",
    );
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0], 2)?,
        &[0.0, 0.0],
        1e-6,
        "lfe",
    );
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0], 2)?,
        &[MINUS_3DB, 0.0],
        1e-6,
        "left surround",
    );
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0], 2)?,
        &[0.0, MINUS_3DB],
        1e-6,
        "right surround",
    );
    // Mono is the average of the stereo downmix.
    assert_buffer_eq(
        &convert(&[0.2, 0.4, 0.5, 0.9, 0.3, 0.1], 1)?,
        &[(0.2 + 0.4 + 2.0 * MINUS_3DB * 0.5 + MINUS_3DB * (0.3 + 0.1)) / 2.0],
        1e-6,
        "5.1 mono",
    );
    Ok(())
}

#[test]
fn test_quad_5_0_and_7_1_downmix_to_stereo() -> Result<()> {
    // quad: L R Ls Rs
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 1.0, 0.5], 2)?,
        &[MINUS_3DB, 0.5 * MINUS_3DB],
        1e-6,
        "quad",
    );
    // 5.0: L R C Ls Rs
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 1.0, 0.0, 0.0], 2)?,
        &[MINUS_3DB, MINUS_3DB],
        1e-6,
        "5.0 centre",
    );
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 0.0, 0.0, 1.0], 2)?,
        &[0.0, MINUS_3DB],
        1e-6,
        "5.0 right surround",
    );
    // 7.1: L R C LFE Lb Rb Ls Rs
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0], 2)?,
        &[2.0 * MINUS_3DB, 0.0],
        1e-6,
        "7.1 left back + side",
    );
    assert_buffer_eq(
        &convert(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 1)?,
        &[MINUS_3DB],
        1e-6,
        "7.1 centre mono",
    );
    Ok(())
}

#[test]
fn test_channel_layouts_without_a_mapping_keep_previous_rules() -> Result<()> {
    // 3 channels -> mono: plain average.
    assert_buffer_eq(&convert(&[0.3, 0.6, 0.9], 1)?, &[0.6], 1e-6, "3 -> 1");
    // 3 channels -> stereo: nearest channel.
    assert_eq!(convert(&[0.3, 0.6, 0.9], 2)?, vec![0.3, 0.6]);
    // Mono -> 5.1 copies the signal everywhere.
    assert_eq!(convert(&[0.25], 6)?, vec![0.25; 6]);
    // Stereo -> 4: nearest channel.
    assert_eq!(convert(&[0.1, 0.2], 4)?, vec![0.1, 0.2, 0.2, 0.2]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Band-limited resampling (rate / speed)
// ---------------------------------------------------------------------------

fn tone(sample_rate: u32, channels: u16, freq: f64, amplitude: f64, frames: usize) -> AudioBuffer {
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for n in 0..frames {
        let value = (amplitude
            * (std::f64::consts::TAU * freq * n as f64 / f64::from(sample_rate)).sin())
            as f32;
        samples.extend(std::iter::repeat_n(value, usize::from(channels)));
    }
    AudioBuffer {
        spec: AudioSpec {
            sample_rate,
            channels,
        },
        samples,
    }
}

/// Channel `channel` of frames `skip..frames - skip`.
fn middle(audio: &AudioBuffer, channel: usize, skip: usize) -> Vec<f64> {
    let channels = usize::from(audio.spec.channels);
    (skip..audio.frames() - skip)
        .map(|frame| f64::from(audio.samples[frame * channels + channel]))
        .collect()
}

fn rms(values: &[f64]) -> f64 {
    (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt()
}

/// Least-squares amplitude of a `freq` Hz sinusoid in `values` (which start at
/// output frame `first`), and the RMS of what is left after removing the
/// expected zero-phase sine `amplitude * sin(2 pi f n / rate)`.
fn fit(values: &[f64], first: usize, rate: u32, freq: f64, amplitude: f64) -> (f64, f64) {
    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    let mut residual = Vec::with_capacity(values.len());
    for (offset, value) in values.iter().enumerate() {
        let phase = std::f64::consts::TAU * freq * (first + offset) as f64 / f64::from(rate);
        sin_sum += value * phase.sin();
        cos_sum += value * phase.cos();
        residual.push(value - amplitude * phase.sin());
    }
    let n = values.len() as f64;
    let fitted = 2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / n;
    (fitted, rms(&residual))
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

#[test]
fn test_rate_48k_to_8k_removes_6khz_tone_instead_of_aliasing_it() -> Result<()> {
    let mut audio = tone(48_000, 1, 6_000.0, 0.5, 24_000);
    let input_rms = rms(&middle(&audio, 0, 0));
    EffectChain::new(vec![Effect::Rate { sample_rate: 8_000 }]).apply(&mut audio)?;
    assert_eq!(audio.spec.sample_rate, 8_000);
    assert_eq!(audio.frames(), 4_000);
    // Skip the edges, where the abrupt start/end of the tone has in-band energy.
    let output_rms = rms(&middle(&audio, 0, 200));
    let attenuation = db(output_rms / input_rms);
    assert!(
        attenuation < -60.0,
        "6 kHz tone only attenuated by {attenuation:.1} dB (it would alias to 2 kHz)"
    );
    Ok(())
}

#[test]
fn test_rate_stopband_attenuation_is_at_least_80_db() -> Result<()> {
    // The stopband starts at the output Nyquist frequency (4 kHz here).
    for freq in [4_100.0, 5_000.0, 7_900.0, 12_000.0, 23_000.0] {
        let mut audio = tone(48_000, 1, freq, 0.5, 24_000);
        let input_rms = rms(&middle(&audio, 0, 0));
        EffectChain::new(vec![Effect::Rate { sample_rate: 8_000 }]).apply(&mut audio)?;
        let attenuation = db(rms(&middle(&audio, 0, 200)) / input_rms);
        assert!(
            attenuation < -80.0,
            "{freq} Hz attenuated by only {attenuation:.1} dB"
        );
    }
    // Upsampling: the images of a 19 kHz tone (inside the 0.9 * 22.05 kHz
    // passband) above 22.05 kHz must be removed, leaving a clean sine at 96 kHz.
    let amplitude = 0.5;
    let mut audio = tone(44_100, 1, 19_000.0, amplitude, 8_820);
    EffectChain::new(vec![Effect::Rate {
        sample_rate: 96_000,
    }])
    .apply(&mut audio)?;
    let values = middle(&audio, 0, 600);
    let (fitted, residual) = fit(&values, 600, 96_000, 19_000.0, amplitude);
    assert!(db(fitted / amplitude).abs() < 0.1);
    let residual_db = db(residual / (amplitude / std::f64::consts::SQRT_2));
    assert!(residual_db < -80.0, "imaging residual {residual_db:.1} dB");
    Ok(())
}

#[test]
fn test_rate_44k1_to_48k_keeps_amplitude_frequency_and_timing() -> Result<()> {
    let amplitude = 0.5;
    let mut audio = tone(44_100, 1, 1_000.0, amplitude, 22_050);
    EffectChain::new(vec![Effect::Rate {
        sample_rate: 48_000,
    }])
    .apply(&mut audio)?;
    assert_eq!(audio.spec.sample_rate, 48_000);
    assert_eq!(audio.frames(), 24_000);
    let skip = 300;
    let values = middle(&audio, 0, skip);
    let (fitted, residual) = fit(&values, skip, 48_000, 1_000.0, amplitude);
    let gain = db(fitted / amplitude);
    assert!(gain.abs() < 0.1, "1 kHz gain changed by {gain:.3} dB");
    // The residual against the expected, zero-phase 1 kHz sine at 48 kHz is
    // tiny only if the frequency is right and there is no time shift.
    let residual_db = db(residual / (amplitude / std::f64::consts::SQRT_2));
    assert!(residual_db < -80.0, "residual {residual_db:.1} dB");
    Ok(())
}

#[test]
fn test_rate_with_many_phases_uses_accurate_fallback() -> Result<()> {
    // 44100 -> 48001 has 48001 phases, so the interpolated kernel table is used.
    let amplitude = 0.5;
    let mut audio = tone(44_100, 1, 1_000.0, amplitude, 8_820);
    EffectChain::new(vec![Effect::Rate {
        sample_rate: 48_001,
    }])
    .apply(&mut audio)?;
    assert_eq!(
        audio.frames(),
        (8_820.0_f64 * 48_001.0 / 44_100.0).round() as usize
    );
    let skip = 300;
    let values = middle(&audio, 0, skip);
    let (fitted, residual) = fit(&values, skip, 48_001, 1_000.0, amplitude);
    assert!(db(fitted / amplitude).abs() < 0.1);
    let residual_db = db(residual / (amplitude / std::f64::consts::SQRT_2));
    assert!(residual_db < -80.0, "residual {residual_db:.1} dB");
    Ok(())
}

#[test]
fn test_rate_downsampling_keeps_passband_tone() -> Result<()> {
    let amplitude = 0.25;
    let mut audio = tone(48_000, 2, 1_000.0, amplitude, 24_000);
    EffectChain::new(vec![Effect::Rate { sample_rate: 8_000 }]).apply(&mut audio)?;
    for channel in 0..2 {
        let values = middle(&audio, channel, 200);
        let (fitted, residual) = fit(&values, 200, 8_000, 1_000.0, amplitude);
        assert!(db(fitted / amplitude).abs() < 0.1, "channel {channel}");
        assert!(db(residual / amplitude) < -70.0, "channel {channel}");
    }
    Ok(())
}

#[test]
fn test_rate_output_lengths_round_frames_times_ratio() -> Result<()> {
    for (frames, from, to) in [
        (1_001_usize, 44_100_u32, 48_000_u32),
        (1_000, 48_000, 44_100),
        (7, 48_000, 8_000),
        (333, 8_000, 22_050),
        (10, 96_000, 8_000),
    ] {
        let mut audio = tone(from, 2, 100.0, 0.5, frames);
        EffectChain::new(vec![Effect::Rate { sample_rate: to }]).apply(&mut audio)?;
        let expected = ((frames as f64 * f64::from(to) / f64::from(from)).round() as usize).max(1);
        assert_eq!(audio.frames(), expected, "{frames} frames {from} -> {to}");
        assert_eq!(audio.samples.len(), expected * 2);
    }
    Ok(())
}

#[test]
fn test_rate_keeps_channels_independent() -> Result<()> {
    // Left: 1 kHz tone, right: silence.
    let source = tone(44_100, 1, 1_000.0, 0.5, 4_410);
    let mut audio = AudioBuffer {
        spec: AudioSpec {
            sample_rate: 44_100,
            channels: 2,
        },
        samples: source.samples.iter().flat_map(|&s| [s, 0.0]).collect(),
    };
    EffectChain::new(vec![Effect::Rate {
        sample_rate: 48_000,
    }])
    .apply(&mut audio)?;
    assert!(middle(&audio, 1, 0).iter().all(|value| *value == 0.0));
    assert!(rms(&middle(&audio, 0, 100)) > 0.3);
    Ok(())
}

#[test]
fn test_speed_up_does_not_alias() -> Result<()> {
    // Speeding a 3 kHz tone up 2x moves it to 6 kHz, above the 4 kHz Nyquist
    // of an 8 kHz signal: it must be filtered out, not folded down to 2 kHz.
    let mut audio = tone(8_000, 1, 3_000.0, 0.5, 8_000);
    let input_rms = rms(&middle(&audio, 0, 0));
    EffectChain::new(vec![Effect::Speed { factor: 2.0 }]).apply(&mut audio)?;
    assert_eq!(audio.spec.sample_rate, 8_000);
    assert_eq!(audio.frames(), 4_000);
    let attenuation = db(rms(&middle(&audio, 0, 200)) / input_rms);
    assert!(attenuation < -60.0, "attenuation {attenuation:.1} dB");
    Ok(())
}

#[test]
fn test_speed_changes_pitch_accurately() -> Result<()> {
    for factor in [0.5_f32, 1.5, 1.1, 0.9] {
        let amplitude = 0.5;
        let mut audio = tone(48_000, 1, 1_000.0, amplitude, 12_000);
        EffectChain::new(vec![Effect::Speed { factor }]).apply(&mut audio)?;
        assert_eq!(audio.spec.sample_rate, 48_000);
        assert_eq!(
            audio.frames(),
            (12_000.0 / f64::from(factor)).round() as usize,
            "speed {factor}"
        );
        let skip = 300;
        let values = middle(&audio, 0, skip);
        let freq = 1_000.0 * f64::from(factor);
        let (fitted, residual) = fit(&values, skip, 48_000, freq, amplitude);
        assert!(db(fitted / amplitude).abs() < 0.1, "speed {factor}");
        let residual_db = db(residual / (amplitude / std::f64::consts::SQRT_2));
        assert!(
            residual_db < -80.0,
            "speed {factor}: residual {residual_db:.1} dB"
        );
    }
    Ok(())
}
