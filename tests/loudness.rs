//! Loudness (ITU-R BS.1770-4 / EBU R128) measurement, true-peak limiting and
//! the `loudness`, `convert --loudness-target` and `stream` commands.

use hound::{SampleFormat, WavSpec, WavWriter};
use serde_json::Value;
use soundx::limiter::TruePeakLimiter;
use soundx::loudness::{LoudnessMeter, LoudnessReport, measure};
use std::path::{Path, PathBuf};
use std::process::Command;

fn db(value: f64) -> f32 {
    10.0_f64.powf(value / 20.0) as f32
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soundx-loudness-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Interleaved sine with the same signal on every channel.
fn sine(rate: u32, channels: usize, freq: f64, dbfs: f64, seconds: f64) -> Vec<f32> {
    let frames = (f64::from(rate) * seconds) as usize;
    let amplitude = f64::from(db(dbfs));
    let mut out = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        let value =
            (amplitude * (std::f64::consts::TAU * freq * n as f64 / f64::from(rate)).sin()) as f32;
        out.extend(std::iter::repeat_n(value, channels));
    }
    out
}

/// Deterministic pink-ish noise (Paul Kellet filter), roughly +-1 peak.
fn pink_noise(frames: usize, channels: usize, seed: u64) -> Vec<f32> {
    let mut state = seed;
    let mut white = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0
    };
    let mut out = Vec::with_capacity(frames * channels);
    let mut filters = vec![[0.0_f64; 7]; channels];
    for _ in 0..frames {
        for b in filters.iter_mut() {
            let w = white();
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.1538520;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
            b[6] = w * 0.115926;
            out.push((pink * 0.11) as f32);
        }
    }
    out
}

fn write_wav(path: &Path, samples: &[f32], rate: u32, channels: u16) {
    let mut writer = WavWriter::create(
        path,
        WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
}

fn read_wav(path: &Path) -> (Vec<f32>, u32, u16) {
    let mut reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
    let samples = match spec.sample_format {
        SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 * scale)
            .collect(),
    };
    (samples, spec.sample_rate, spec.channels)
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_soundx"))
        .args(args)
        .output()
        .unwrap()
}

fn run_ok(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn lufs(report: &LoudnessReport) -> f64 {
    report.integrated_lufs.expect("loudness above the gate")
}

fn measure_file(path: &Path) -> LoudnessReport {
    let (samples, rate, channels) = read_wav(path);
    measure(&samples, rate, channels).unwrap()
}

#[test]
fn ebu_3341_sine_levels_are_within_a_tenth_of_a_lu() {
    for rate in [48_000, 44_100] {
        for level in [-23.0, -20.0] {
            let samples = sine(rate, 2, 1000.0, level, 20.0);
            let report = measure(&samples, rate, 2).unwrap();
            assert!(
                (lufs(&report) - level).abs() <= 0.1,
                "{rate} Hz {level} dBFS measured {}",
                lufs(&report)
            );
            assert!(report.loudness_range_lu < 0.1);
            assert!((report.sample_peak_dbfs.unwrap() - level).abs() < 0.01);
        }
    }
}

#[test]
fn k_weighting_is_derived_for_any_sample_rate() {
    for rate in [8_000, 22_050, 32_000, 88_200, 96_000, 192_000] {
        let samples = sine(rate, 2, 1000.0, -23.0, 12.0);
        let report = measure(&samples, rate, 2).unwrap();
        assert!(
            (lufs(&report) + 23.0).abs() <= 0.1,
            "{rate} Hz measured {}",
            lufs(&report)
        );
    }
}

#[test]
fn mono_counts_as_a_single_full_weight_channel() {
    let mono = measure(&sine(48_000, 1, 1000.0, -20.0, 10.0), 48_000, 1).unwrap();
    let stereo = measure(&sine(48_000, 2, 1000.0, -20.0, 10.0), 48_000, 2).unwrap();
    assert!((lufs(&stereo) - lufs(&mono) - 3.0103).abs() < 0.02);
}

#[test]
fn surround_channels_are_weighted_and_lfe_is_excluded() {
    let frames = 48_000 * 10;
    let tone = sine(48_000, 1, 1000.0, -20.0, 10.0);
    let in_channel = |index: usize| {
        let mut samples = vec![0.0; frames * 6];
        for (frame, value) in tone.iter().enumerate() {
            samples[frame * 6 + index] = *value;
        }
        measure(&samples, 48_000, 6).unwrap()
    };
    let front = lufs(&in_channel(0));
    let surround = lufs(&in_channel(4));
    assert!(
        (surround - front - 1.4933).abs() < 0.02,
        "{surround} {front}"
    );
    assert!(
        in_channel(3).integrated_lufs.is_none(),
        "LFE must be excluded"
    );
}

#[test]
fn silence_and_quiet_periods_are_gated_out() {
    let rate = 48_000;
    let tone = sine(rate, 2, 440.0, -20.0, 5.0);
    let reference = lufs(&measure(&tone, rate, 2).unwrap());

    // 5 s tone, 10 s digital silence, 5 s tone: the silence does not count.
    let mut gapped = tone.clone();
    gapped.extend(std::iter::repeat_n(0.0, rate as usize * 2 * 10));
    gapped.extend_from_slice(&tone);
    let report = measure(&gapped, rate, 2).unwrap();
    // Blocks straddling the silence boundary carry part of the tone and pass
    // the relative gate, which costs ~0.13 LU here; the 10 s hole itself does not.
    assert!(
        (lufs(&report) - reference).abs() <= 0.2,
        "{}",
        lufs(&report)
    );
    assert!(lufs(&report) < reference + 0.01);
    assert!(report.momentary_max_lufs.unwrap() > reference - 0.5);

    // Material 20 LU below the loud part is removed by the relative gate.
    let mut mixed = sine(rate, 2, 440.0, -20.0, 10.0);
    mixed.extend(sine(rate, 2, 440.0, -40.0, 10.0));
    let report = measure(&mixed, rate, 2).unwrap();
    assert!(
        (lufs(&report) - reference).abs() <= 0.1,
        "{}",
        lufs(&report)
    );

    // Everything under -70 LUFS: no integrated value at all.
    let quiet = sine(rate, 2, 1000.0, -85.0, 10.0);
    let report = measure(&quiet, rate, 2).unwrap();
    assert!(report.integrated_lufs.is_none());
    assert_eq!(report.loudness_range_lu, 0.0);
}

#[test]
fn loudness_range_of_stepped_levels_follows_tech_3342() {
    let rate = 48_000;
    // Two plateaus 10 LU apart.
    let mut two = sine(rate, 2, 1000.0, -20.0, 20.0);
    two.extend(sine(rate, 2, 1000.0, -30.0, 20.0));
    let report = measure(&two, rate, 2).unwrap();
    assert!(
        (report.loudness_range_lu - 10.0).abs() <= 0.5,
        "LRA {}",
        report.loudness_range_lu
    );
    // Three plateaus 20 LU apart in total.
    let mut three = sine(rate, 2, 1000.0, -40.0, 20.0);
    three.extend(sine(rate, 2, 1000.0, -30.0, 20.0));
    three.extend(sine(rate, 2, 1000.0, -20.0, 20.0));
    let report = measure(&three, rate, 2).unwrap();
    assert!(
        (report.loudness_range_lu - 20.0).abs() <= 0.5,
        "LRA {}",
        report.loudness_range_lu
    );
    // Material beyond the -20 LU relative gate is ignored.
    let mut gated = sine(rate, 2, 1000.0, -20.0, 30.0);
    gated.extend(sine(rate, 2, 1000.0, -60.0, 10.0));
    let report = measure(&gated, rate, 2).unwrap();
    assert!(
        report.loudness_range_lu < 1.0,
        "{}",
        report.loudness_range_lu
    );
}

#[test]
fn true_peak_sees_inter_sample_peaks() {
    for rate in [48_000, 44_100] {
        // fs/4 sine at 45 degrees: every sample is +-0.7071 but the waveform
        // between the samples reaches 1.0.
        let frames = rate as usize;
        let samples: Vec<f32> = (0..frames)
            .map(|n| {
                (std::f64::consts::FRAC_PI_2 * n as f64 + std::f64::consts::FRAC_PI_4).sin() as f32
            })
            .collect();
        let report = measure(&samples, rate, 1).unwrap();
        let sample_peak = report.sample_peak_dbfs.unwrap();
        let true_peak = report.true_peak_dbtp.unwrap();
        assert!((sample_peak + 3.0103).abs() < 0.01, "{sample_peak}");
        assert!(
            true_peak > sample_peak + 2.9,
            "{true_peak} vs {sample_peak}"
        );
        assert!(true_peak.abs() < 0.1, "{true_peak}");
    }
}

#[test]
fn chunked_streaming_equals_one_shot_measurement() {
    let samples = pink_noise(48_000 * 12, 2, 7);
    let whole = measure(&samples, 48_000, 2).unwrap();
    let mut meter = LoudnessMeter::new(48_000, 2).unwrap();
    for chunk in samples.chunks(777) {
        meter.push(chunk);
    }
    let chunked = meter.finish();
    assert_eq!(whole.integrated_lufs, chunked.integrated_lufs);
    assert_eq!(whole.loudness_range_lu, chunked.loudness_range_lu);
    assert_eq!(whole.true_peak_dbtp, chunked.true_peak_dbtp);
}

fn limit(samples: &[f32], rate: u32, channels: u16, ceiling: f64) -> (Vec<f32>, TruePeakLimiter) {
    let mut limiter = TruePeakLimiter::new(rate, channels, ceiling).unwrap();
    let mut out = Vec::new();
    for chunk in samples.chunks(1000 * usize::from(channels)) {
        limiter.process(chunk, &mut out);
    }
    limiter.finish(&mut out);
    (out, limiter)
}

#[test]
fn limiter_keeps_true_peak_under_the_ceiling_and_preserves_length() {
    let rate = 48_000;
    let mut samples = pink_noise(rate as usize * 6, 2, 3);
    for sample in &mut samples {
        *sample *= 2.2; // well above full scale
    }
    // Inter-sample peaks: fs/4 bursts at 45 degrees.
    for burst in [20_000usize, 90_000, 200_001] {
        for n in 0..64 {
            let v = 1.3
                * (std::f64::consts::FRAC_PI_2 * n as f64 + std::f64::consts::FRAC_PI_4).sin()
                    as f32;
            samples[(burst + n) * 2] = v;
            samples[(burst + n) * 2 + 1] = -v;
        }
    }
    let input = measure(&samples, rate, 2).unwrap();
    assert!(input.true_peak_dbtp.unwrap() > 3.0);
    let ceiling = -1.0;
    let (out, limiter) = limit(&samples, rate, 2, ceiling);
    assert_eq!(out.len(), samples.len());
    let output = measure(&out, rate, 2).unwrap();
    assert!(
        output.true_peak_dbtp.unwrap() <= ceiling + 0.1,
        "true peak {}",
        output.true_peak_dbtp.unwrap()
    );
    assert!(limiter.max_gain_reduction_db() > 3.0);
}

#[test]
fn limiter_is_transparent_when_nothing_exceeds_the_ceiling() {
    let samples = pink_noise(48_000 * 3, 2, 11);
    let (out, limiter) = limit(&samples, 48_000, 2, -0.1);
    assert_eq!(out.len(), samples.len());
    let worst = samples
        .iter()
        .zip(&out)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(worst < 1e-6, "{worst}");
    assert_eq!(limiter.max_gain_reduction_db(), 0.0);
}

#[test]
fn limiter_output_is_sample_aligned_and_smooth() {
    let rate = 48_000;
    let frames = rate as usize;
    // A single loud click in silence keeps its position and is brought to the
    // ceiling exactly at the click, not before or after.
    let mut click = vec![0.0_f32; frames];
    click[10_000] = 2.0;
    let (out, _) = limit(&click, rate, 1, -6.0);
    assert_eq!(out.len(), frames);
    let (index, peak) = out
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    assert_eq!(index, 10_000);
    assert!((peak - db(-6.0)).abs() < 1e-4, "{peak}");

    // A steady over-range sine: the gain glides down, never steps.
    let tone: Vec<f32> = sine(rate, 1, 997.0, 3.0, 1.0); // +3 dBFS
    let (out, _) = limit(&tone, rate, 1, -1.0);
    assert_eq!(out.len(), tone.len());
    let gains: Vec<f64> = tone
        .iter()
        .zip(&out)
        .filter(|(i, _)| i.abs() > 0.7)
        .map(|(i, o)| f64::from(*o) / f64::from(*i))
        .collect();
    let steepest = gains
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f64::max);
    assert!(steepest < 0.02, "gain step {steepest}");
    let settled = measure(&out[20_000..], rate, 1).unwrap();
    assert!(settled.true_peak_dbtp.unwrap() <= -0.9);
    assert!(
        settled.sample_peak_dbfs.unwrap() > -2.0,
        "no needless attenuation"
    );
}

#[test]
fn loudness_command_reports_json_and_null_for_silence() {
    let dir = temp_dir("cli");
    let tone = dir.join("tone.wav");
    let silent = dir.join("silent.wav");
    write_wav(&tone, &sine(48_000, 2, 1000.0, -23.0, 5.0), 48_000, 2);
    write_wav(&silent, &vec![0.0; 48_000 * 2 * 2], 48_000, 2);
    let tone_arg = tone.to_str().unwrap();
    let silent_arg = silent.to_str().unwrap();

    let json: Value =
        serde_json::from_str(&run_ok(&["loudness", tone_arg, silent_arg, "--json"])).unwrap();
    let reports = json.as_array().unwrap();
    assert_eq!(reports.len(), 2);
    let first = &reports[0];
    assert!((first["integrated_lufs"].as_f64().unwrap() + 23.0).abs() <= 0.1);
    assert_eq!(first["sample_rate"], 48_000);
    assert_eq!(first["channels"], 2);
    assert_eq!(first["frames"], 240_000);
    assert_eq!(first["standard"], "ITU-R BS.1770-4 / EBU R128");
    assert_eq!(first["gating"]["absolute_lufs"], -70.0);
    assert_eq!(first["gating"]["relative_lu"], -10.0);
    assert_eq!(first["gating"]["block_ms"], 400);
    assert_eq!(first["gating"]["overlap"], 0.75);
    for key in [
        "loudness_range_lu",
        "true_peak_dbtp",
        "sample_peak_dbfs",
        "momentary_max_lufs",
        "short_term_max_lufs",
        "duration_seconds",
        "path",
    ] {
        assert!(first.get(key).is_some(), "missing {key}");
    }
    let second = &reports[1];
    assert!(second["integrated_lufs"].is_null());
    assert_eq!(second["loudness_range_lu"], 0.0);

    let text = run_ok(&["loudness", tone_arg]);
    assert!(text.contains("Integrated Loudness: -23.0 LUFS"), "{text}");
    assert!(run_ok(&["loudness", "--help"]).contains("loudness"));
    assert!(
        !run(&["loudness", dir.join("missing.wav").to_str().unwrap()])
            .status
            .success()
    );
}

#[test]
fn convert_hits_the_loudness_target_with_the_true_peak_ceiling() {
    let dir = temp_dir("convert");
    let input = dir.join("loud.wav");
    let output = dir.join("out.wav");
    // Peaky material: quiet noise with strong fs/4 bursts every half second,
    // so reaching -18 LUFS pushes the bursts far past the ceiling.
    let mut samples: Vec<f32> = pink_noise(48_000 * 20, 2, 5)
        .iter()
        .map(|s| s * 0.04)
        .collect();
    for burst in (0..40).map(|k| 12_000 + k * 24_000) {
        for n in 0..32 {
            let v = 1.0
                * (std::f64::consts::FRAC_PI_2 * n as f64 + std::f64::consts::FRAC_PI_4).sin()
                    as f32;
            samples[(burst + n) * 2] = v;
            samples[(burst + n) * 2 + 1] = v;
        }
    }
    write_wav(&input, &samples, 48_000, 2);
    let before = measure_file(&input);

    let stdout = run_ok(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--bits",
        "24",
        "--loudness-target=-18",
        "--true-peak=-1.8",
        "--stat-json",
    ]);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    let loudness = &json["loudness"];
    assert_eq!(loudness["limited"], true);
    assert!(loudness["limiter_gain_reduction_max_db"].as_f64().unwrap() > 0.0);
    assert!(loudness["gain_db"].is_number());
    assert!((loudness["input"]["integrated_lufs"].as_f64().unwrap() - lufs(&before)).abs() < 1e-6);
    assert!((loudness["output"]["integrated_lufs"].as_f64().unwrap() + 18.0).abs() <= 0.1);

    let after = measure_file(&output);
    let (written, _, _) = read_wav(&output);
    assert_eq!(
        written.len(),
        samples.len(),
        "frame count must be unchanged"
    );
    assert!((lufs(&after) + 18.0).abs() <= 0.1, "{}", lufs(&after));
    assert!(
        after.true_peak_dbtp.unwrap() <= -1.8 + 0.1,
        "{:?}",
        after.true_peak_dbtp
    );
}

#[test]
fn convert_without_limiting_is_a_pure_gain() {
    let dir = temp_dir("gain");
    let input = dir.join("quiet.wav");
    let output = dir.join("out.wav");
    write_wav(&input, &sine(44_100, 2, 1000.0, -30.0, 10.0), 44_100, 2);
    let stdout = run_ok(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--loudness-target",
        "-23",
        "--stat-json",
    ]);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["loudness"]["limited"], false);
    assert_eq!(json["loudness"]["limiter_gain_reduction_max_db"], 0.0);
    assert!((json["loudness"]["gain_db"].as_f64().unwrap() - 7.0).abs() < 0.01);
    let after = measure_file(&output);
    assert!((lufs(&after) + 23.0).abs() <= 0.1);
}

#[test]
fn true_peak_alone_only_limits() {
    let dir = temp_dir("peak-only");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    let samples = sine(48_000, 2, 1000.0, -0.5, 3.0);
    write_wav(&input, &samples, 48_000, 2);
    run_ok(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--true-peak=-6",
    ]);
    let after = measure_file(&output);
    assert!(after.true_peak_dbtp.unwrap() <= -5.9);
    assert_eq!(read_wav(&output).0.len(), samples.len());
}

#[test]
fn convert_of_silence_leaves_it_untouched() {
    let dir = temp_dir("silence");
    let input = dir.join("silent.wav");
    let output = dir.join("out.wav");
    write_wav(&input, &vec![0.0; 48_000 * 2], 48_000, 2);
    let stdout = run_ok(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--loudness-target=-18",
        "--stat-json",
    ]);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["loudness"]["input"]["integrated_lufs"].is_null());
    assert_eq!(json["loudness"]["gain_db"], 0.0);
    assert!(read_wav(&output).0.iter().all(|s| *s == 0.0));
}

#[test]
fn loudness_options_are_validated() {
    let dir = temp_dir("validate");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    write_wav(&input, &sine(48_000, 1, 440.0, -20.0, 1.0), 48_000, 1);
    let i = input.to_str().unwrap();
    let o = output.to_str().unwrap();
    let must_fail = |args: &[&str]| {
        let result = run(args);
        assert!(!result.status.success(), "{args:?} should fail");
        assert!(!output.exists(), "{args:?} must not create output");
    };
    must_fail(&["convert", i, o, "--normalize", "--loudness-target=-18"]);
    must_fail(&["convert", i, o, "--normalize", "--true-peak=-1"]);
    must_fail(&["convert", i, o, "--loudness-target=5"]);
    must_fail(&["convert", i, o, "--loudness-target=-90"]);
    must_fail(&["convert", i, o, "--loudness-target=-18", "--true-peak=2"]);
    must_fail(&["stream", i, o, "--loudness-target=-18", "--gain-db=3"]);
    must_fail(&["stream", i, o, "--bits", "24"]);
}

#[test]
fn stream_matches_convert() {
    let dir = temp_dir("stream");
    let input = dir.join("in.wav");
    let converted = dir.join("convert.wav");
    let streamed = dir.join("stream.wav");
    let mut samples = pink_noise(44_100 * 15, 2, 9);
    for sample in &mut samples {
        *sample *= 1.6;
    }
    write_wav(&input, &samples, 44_100, 2);
    let (i, c, s) = (
        input.to_str().unwrap(),
        converted.to_str().unwrap(),
        streamed.to_str().unwrap(),
    );
    run_ok(&[
        "convert",
        i,
        c,
        "--bits",
        "16",
        "--loudness-target=-20",
        "--true-peak=-2",
    ]);
    let stdout = run_ok(&[
        "stream",
        i,
        s,
        "--loudness-target=-20",
        "--true-peak=-2",
        "--bits",
        "16",
        "--stat-json",
    ]);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["loudness"]["gain_db"].is_number());

    let (a, rate_a, ch_a) = read_wav(&converted);
    let (b, rate_b, ch_b) = read_wav(&streamed);
    assert_eq!((rate_a, ch_a), (rate_b, ch_b));
    assert_eq!(a.len(), samples.len());
    assert_eq!(b.len(), samples.len());
    let worst = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max);
    assert!(worst < 4.0 / 32768.0, "worst sample difference {worst}");
    let (la, lb) = (measure_file(&converted), measure_file(&streamed));
    assert!((lufs(&la) - lufs(&lb)).abs() < 0.02);
    assert!((lufs(&lb) + 20.0).abs() <= 0.1);
    assert!(lb.true_peak_dbtp.unwrap() <= -2.0 + 0.1);
}

#[test]
fn stream_keeps_the_input_bit_depth_by_default() {
    let dir = temp_dir("stream-depth");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    let mut writer = WavWriter::create(
        &input,
        WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        },
    )
    .unwrap();
    for sample in sine(48_000, 1, 1000.0, -30.0, 5.0) {
        writer.write_sample((sample * 8_388_607.0) as i32).unwrap();
    }
    writer.finalize().unwrap();
    run_ok(&[
        "stream",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--loudness-target=-20",
    ]);
    let spec = hound::WavReader::open(&output).unwrap().spec();
    assert_eq!(spec.bits_per_sample, 24);
}

#[test]
fn mcp_exposes_the_loudness_tool() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    let dir = temp_dir("mcp");
    let tone = dir.join("tone.wav");
    write_wav(&tone, &sine(48_000, 2, 1000.0, -20.0, 3.0), 48_000, 2);
    let mut child = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    writeln!(stdin, "{init}").unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("\"serverInfo\""), "{line}");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    let mut call = |value: Value| -> Value {
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let tools = call(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "soundx_loudness")
    );
    let result = call(
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"soundx_loudness","arguments":{"inputs":[tone]}}}),
    );
    assert_eq!(result["result"]["isError"], false, "{result}");
    let loudness = &result["result"]["structuredContent"]["loudness"][0];
    assert!((loudness["integrated_lufs"].as_f64().unwrap() + 20.0).abs() <= 0.1);
    // Also reachable through the generic runner.
    let result = call(
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
        "name":"soundx_run","arguments":{"arguments":["loudness", tone, "--json"]}}}),
    );
    assert_eq!(result["result"]["isError"], false, "{result}");
    drop(stdin);
    let _ = child.wait();
}

fn ffmpeg() -> Option<String> {
    let program = std::env::var("SOUNDX_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string());
    Command::new(&program)
        .arg("-version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| program)
}

/// Parse `I`, `LRA` and true `Peak` from the summary of `ebur128=peak=true`.
fn ffmpeg_measure(program: &str, path: &Path) -> (f64, f64, f64) {
    let output = Command::new(program)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stderr).into_owned();
    let summary = &text[text.rfind("Summary:").expect("ffmpeg summary")..];
    let value = |label: &str| -> f64 {
        let start = summary
            .find(label)
            .unwrap_or_else(|| panic!("{label} in {summary}"))
            + label.len();
        summary[start..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    (value("I:"), value("LRA:"), value("Peak:"))
}

#[test]
fn matches_ffmpeg_ebur128_when_available() {
    let Some(program) = ffmpeg() else {
        eprintln!("skipping: ffmpeg not available (set SOUNDX_FFMPEG to its path)");
        return;
    };
    let dir = temp_dir("ffmpeg");
    for (name, rate, make) in [
        ("pink48", 48_000u32, 0),
        ("pink44", 44_100, 0),
        ("peaky48", 48_000, 1),
        ("steps48", 48_000, 2),
    ] {
        let frames = rate as usize * 40;
        let mut samples = pink_noise(frames, 2, 21);
        match make {
            0 => samples.iter_mut().for_each(|s| *s *= 1.5),
            1 => samples
                .iter_mut()
                .enumerate()
                .for_each(|(n, s)| *s = (*s * 6.0).tanh() * 0.9 + 0.08 * (n as f32 * 1.9).sin()),
            _ => {
                for (n, s) in samples.iter_mut().enumerate() {
                    let third = frames * 2 / 3;
                    *s *= if n < third {
                        0.15
                    } else if n < third * 2 {
                        0.5
                    } else {
                        1.4
                    };
                }
            }
        }
        let path = dir.join(format!("{name}.wav"));
        write_wav(&path, &samples, rate, 2);
        let ours = measure_file(&path);
        let (integrated, range, peak) = ffmpeg_measure(&program, &path);
        assert!(
            (lufs(&ours) - integrated).abs() <= 0.2,
            "{name} I {} vs {integrated}",
            lufs(&ours)
        );
        assert!(
            (ours.loudness_range_lu - range).abs() <= 0.5,
            "{name} LRA {} vs {range}",
            ours.loudness_range_lu
        );
        assert!(
            (ours.true_peak_dbtp.unwrap() - peak).abs() <= 0.3,
            "{name} TP {:?} vs {peak}",
            ours.true_peak_dbtp
        );
    }
}
