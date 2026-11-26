//! Reader/writer sample-range contract: floating-point input is never clamped
//! (NaN/Inf is rejected), every writer validates, integer outputs clamp when
//! quantizing, and float outputs keep values above full scale.

use hound::{SampleFormat, WavSpec, WavWriter};
use soundx::audio::{AudioBuffer, AudioSpec, RawEncoding};
use soundx::encode::{AuEncoding, EncodeOptions, write_audio_with_options};
use soundx::loudness::measure;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soundx-audio-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sine(rate: u32, freq: f64, amplitude: f64, seconds: f64) -> Vec<f32> {
    let frames = (f64::from(rate) * seconds) as usize;
    (0..frames)
        .map(|n| {
            (amplitude * (std::f64::consts::TAU * freq * n as f64 / f64::from(rate)).sin()) as f32
        })
        .collect()
}

fn write_float_wav(path: &Path, samples: &[f32], rate: u32, channels: u16) {
    let mut writer = WavWriter::create(
        path,
        WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        },
    )
    .unwrap();
    for sample in samples {
        writer.write_sample(*sample).unwrap();
    }
    writer.finalize().unwrap();
}

fn buffer(samples: Vec<f32>, rate: u32, channels: u16) -> AudioBuffer {
    AudioBuffer {
        spec: AudioSpec {
            sample_rate: rate,
            channels,
        },
        samples,
    }
}

#[test]
fn float_wav_above_full_scale_keeps_its_peak_and_loudness() {
    let dir = temp_dir("over-full-scale");
    let path = dir.join("hot.wav");
    let rate = 48_000;
    // A 997 Hz sine at amplitude 2.0 (+6.02 dBFS).
    write_float_wav(&path, &sine(rate, 997.0, 2.0, 5.0), rate, 1);

    let audio = AudioBuffer::read(&path).unwrap();
    assert!(
        (audio.peak() - 2.0).abs() < 1.0e-3,
        "peak was clamped to {}",
        audio.peak()
    );

    let report = measure(&audio.samples, rate, 1).unwrap();
    let sample_peak = report.sample_peak_dbfs.unwrap();
    let true_peak = report.true_peak_dbtp.unwrap();
    let integrated = report.integrated_lufs.unwrap();
    let expected_peak = 20.0 * 2.0_f64.log10();
    assert!(
        (sample_peak - expected_peak).abs() < 0.05,
        "sample peak {sample_peak} dBFS"
    );
    assert!(
        (true_peak - expected_peak).abs() < 0.3,
        "true peak {true_peak} dBTP"
    );
    // A full-scale mono sine near 1 kHz reads -3.01 LUFS; +6.02 dB gives +3.01.
    assert!(
        (integrated - 3.01).abs() < 0.1,
        "integrated {integrated} LUFS"
    );

    // Headroom is recoverable: -6.02 dB of gain brings it back to full scale.
    let mut quieter = audio.clone();
    soundx::effects::EffectChain::new(vec![soundx::effects::Effect::GainDb(-6.0206)])
        .apply(&mut quieter)
        .unwrap();
    assert!((quieter.peak() - 1.0).abs() < 1.0e-3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn float_outputs_round_trip_values_above_full_scale() {
    let dir = temp_dir("float-round-trip");
    let samples = vec![2.5, -3.0, 0.25, 1.5, -1.25, 0.0, 7.75, -0.5];
    let audio = buffer(samples.clone(), 44_100, 2);

    let cases: Vec<(&str, EncodeOptions, Option<RawEncoding>)> = vec![
        (
            "f32.wav",
            EncodeOptions {
                wav_bits: Some(32),
                wav_float: true,
                ..EncodeOptions::default()
            },
            None,
        ),
        (
            "f64.wav",
            EncodeOptions {
                wav_bits: Some(64),
                wav_float: true,
                ..EncodeOptions::default()
            },
            None,
        ),
        (
            "f32.au",
            EncodeOptions {
                au_encoding: Some(AuEncoding::Float32),
                ..EncodeOptions::default()
            },
            None,
        ),
        (
            "f64.au",
            EncodeOptions {
                au_encoding: Some(AuEncoding::Float64),
                ..EncodeOptions::default()
            },
            None,
        ),
        (
            "f32le.raw",
            EncodeOptions {
                raw_encoding: Some(RawEncoding::Float32le),
                ..EncodeOptions::default()
            },
            Some(RawEncoding::Float32le),
        ),
        (
            "f32be.raw",
            EncodeOptions {
                raw_encoding: Some(RawEncoding::Float32be),
                ..EncodeOptions::default()
            },
            Some(RawEncoding::Float32be),
        ),
        (
            "f64le.raw",
            EncodeOptions {
                raw_encoding: Some(RawEncoding::Float64le),
                ..EncodeOptions::default()
            },
            Some(RawEncoding::Float64le),
        ),
        (
            "f64be.raw",
            EncodeOptions {
                raw_encoding: Some(RawEncoding::Float64be),
                ..EncodeOptions::default()
            },
            Some(RawEncoding::Float64be),
        ),
        ("float.wv", EncodeOptions::default(), None),
    ];
    for (name, options, raw) in cases {
        let path = dir.join(name);
        write_audio_with_options(&audio, &path, options).unwrap();
        let decoded = match raw {
            Some(encoding) => {
                AudioBuffer::read_with_raw_options(&path, Some(44_100), Some(2), Some(encoding))
                    .unwrap()
            }
            None => AudioBuffer::read(&path).unwrap(),
        };
        assert_eq!(decoded.spec.channels, 2, "{name}");
        assert_eq!(decoded.spec.sample_rate, 44_100, "{name}");
        assert_eq!(decoded.samples, samples, "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn integer_outputs_clamp_when_quantizing() {
    let dir = temp_dir("int-clamp");
    let audio = buffer(vec![2.0, -2.0, 0.5], 8_000, 1);
    for (name, options) in [
        ("pcm16.wav", EncodeOptions::default()),
        (
            "pcm24.wav",
            EncodeOptions {
                wav_bits: Some(24),
                ..EncodeOptions::default()
            },
        ),
        (
            "pcm16.au",
            EncodeOptions {
                au_encoding: Some(AuEncoding::Pcm16),
                ..EncodeOptions::default()
            },
        ),
        ("pcm16.aiff", EncodeOptions::default()),
        ("pcm.flac", EncodeOptions::default()),
    ] {
        let path = dir.join(name);
        write_audio_with_options(&audio, &path, options).unwrap();
        let decoded = AudioBuffer::read(&path).unwrap();
        assert!(
            decoded.samples[0] > 0.999 && decoded.samples[0] <= 1.0,
            "{name}"
        );
        assert!((decoded.samples[1] + 1.0).abs() < 1.0e-6, "{name}");
        assert!((decoded.samples[2] - 0.5).abs() < 1.0e-3, "{name}");
    }
    let raw = dir.join("s16.raw");
    write_audio_with_options(
        &audio,
        &raw,
        EncodeOptions {
            raw_encoding: Some(RawEncoding::PcmS16le),
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let bytes = std::fs::read(&raw).unwrap();
    assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), i16::MAX);
    assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), i16::MIN);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lossy_encoders_accept_over_full_scale_input() {
    let dir = temp_dir("lossy-clamp");
    // Stereo, because the Vorbis encoder does not support mono streams.
    let stereo = sine(44_100, 440.0, 1.8, 0.5)
        .into_iter()
        .flat_map(|sample| [sample, sample])
        .collect();
    let audio = buffer(stereo, 44_100, 2);
    for name in ["hot.mp3", "hot.ogg", "hot.aac", "hot.gsm", "hot.amr"] {
        let path = dir.join(name);
        write_audio_with_options(&audio, &path, EncodeOptions::default()).unwrap();
        let decoded = AudioBuffer::read(&path).unwrap();
        assert!(!decoded.samples.is_empty(), "{name}");
        assert!(decoded.samples.iter().all(|s| s.is_finite()), "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_writer_rejects_non_finite_samples() {
    let dir = temp_dir("non-finite-write");
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let audio = buffer(vec![0.1, bad, 0.2, 0.3], 44_100, 2);
        // The default WAV path used to skip validation and write silence.
        let path = dir.join("default.wav");
        assert!(audio.write_wav(&path).is_err(), "write_wav accepted {bad}");
        for (bits, float) in [(16, false), (24, false), (32, true)] {
            assert!(
                audio
                    .write_wav_with_format(&dir.join("format.wav"), bits, float)
                    .is_err(),
                "{bits}-bit float={float} accepted {bad}"
            );
        }
        for name in [
            "a.wav", "a.flac", "a.mp3", "a.ogg", "a.aac", "a.aiff", "a.au", "a.raw", "a.gsm",
            "a.wv", "a.amr",
        ] {
            assert!(
                write_audio_with_options(&audio, &dir.join(name), EncodeOptions::default())
                    .is_err(),
                "{name} accepted {bad}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn float_readers_reject_nan_and_infinity() {
    let dir = temp_dir("non-finite-read");

    // 32-bit float WAV (used to let NaN through).
    let wav = dir.join("nan.wav");
    write_float_wav(&wav, &[0.0, f32::NAN, 0.5], 44_100, 1);
    let error = AudioBuffer::read(&wav).unwrap_err();
    assert!(format!("{error:#}").contains("NaN"), "{error:#}");
    let wav = dir.join("inf.wav");
    write_float_wav(&wav, &[0.0, f32::INFINITY], 44_100, 1);
    assert!(AudioBuffer::read(&wav).is_err());

    // AU float32 / float64 (float64 used to let NaN through).
    for (code, bytes) in [
        (6_u32, f32::NAN.to_be_bytes().to_vec()),
        (6, f32::NEG_INFINITY.to_be_bytes().to_vec()),
        (7, f64::NAN.to_be_bytes().to_vec()),
        (7, f64::INFINITY.to_be_bytes().to_vec()),
        (7, 1.0e300_f64.to_be_bytes().to_vec()),
    ] {
        let au = dir.join("bad.au");
        let mut file = Vec::new();
        file.extend_from_slice(b".snd");
        file.extend_from_slice(&24_u32.to_be_bytes());
        file.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        file.extend_from_slice(&code.to_be_bytes());
        file.extend_from_slice(&8_000_u32.to_be_bytes());
        file.extend_from_slice(&1_u32.to_be_bytes());
        file.extend_from_slice(&bytes);
        std::fs::write(&au, file).unwrap();
        assert!(
            AudioBuffer::read(&au).is_err(),
            "AU encoding {code} accepted {bytes:?}"
        );
    }

    // RAW float.
    let raw = dir.join("nan.raw");
    std::fs::write(&raw, f32::NAN.to_le_bytes()).unwrap();
    assert!(
        AudioBuffer::read_with_raw_options(
            &raw,
            Some(8_000),
            Some(1),
            Some(RawEncoding::Float32le)
        )
        .is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn float_au_and_float64_wav_inputs_are_not_clamped() {
    let dir = temp_dir("float-inputs");
    let au = dir.join("hot.au");
    let mut file = Vec::new();
    file.extend_from_slice(b".snd");
    file.extend_from_slice(&24_u32.to_be_bytes());
    file.extend_from_slice(&8_u32.to_be_bytes());
    file.extend_from_slice(&6_u32.to_be_bytes());
    file.extend_from_slice(&8_000_u32.to_be_bytes());
    file.extend_from_slice(&1_u32.to_be_bytes());
    file.extend_from_slice(&3.5_f32.to_be_bytes());
    file.extend_from_slice(&(-1.5_f32).to_be_bytes());
    std::fs::write(&au, file).unwrap();
    assert_eq!(AudioBuffer::read(&au).unwrap().samples, vec![3.5, -1.5]);

    let wav = dir.join("hot64.wav");
    let audio = buffer(vec![4.0, -2.25], 8_000, 1);
    write_audio_with_options(
        &audio,
        &wav,
        EncodeOptions {
            wav_bits: Some(64),
            wav_float: true,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    assert_eq!(AudioBuffer::read(&wav).unwrap().samples, vec![4.0, -2.25]);
    let _ = std::fs::remove_dir_all(dir);
}

fn soundx(args: &[&str], input: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_soundx"))
        .arg("convert")
        .arg(input)
        .arg(output)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_rejects_nan_gain_instead_of_writing_silence() {
    let dir = temp_dir("cli-nan-gain");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    write_float_wav(&input, &sine(8_000, 440.0, 0.5, 0.1), 8_000, 1);
    let result = soundx(&["--gain-db", "nan"], &input, &output);
    assert!(
        !result.status.success(),
        "--gain-db nan succeeded: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_float_convert_preserves_headroom() {
    let dir = temp_dir("cli-float");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    write_float_wav(&input, &[1.5, -2.0, 0.25], 8_000, 1);
    let result = soundx(&["--bits", "32", "--float"], &input, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        AudioBuffer::read(&output).unwrap().samples,
        vec![1.5, -2.0, 0.25]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_trim_with_huge_duration_matches_open_ended_trim() {
    let dir = temp_dir("cli-trim");
    let input = dir.join("in.wav");
    write_float_wav(&input, &sine(8_000, 440.0, 0.5, 1.0), 8_000, 1);
    let huge = dir.join("huge.wav");
    let open = dir.join("open.wav");
    let result = soundx(&["--trim", "0.2", "1e30"], &input, &huge);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = soundx(&["--trim", "0.2"], &input, &open);
    assert!(result.status.success());
    let huge = AudioBuffer::read(&huge).unwrap();
    let open = AudioBuffer::read(&open).unwrap();
    assert_eq!(huge.frames(), 6_400);
    assert_eq!(huge.samples, open.samples);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_pad_with_huge_duration_fails_with_a_message() {
    let dir = temp_dir("cli-pad");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    write_float_wav(&input, &sine(8_000, 440.0, 0.5, 0.1), 8_000, 1);
    let result = soundx(&["--pad", "1e30"], &input, &output);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("too large") || stderr.contains("not enough memory"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
