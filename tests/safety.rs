//! Regression tests for CLI and file-safety defects.

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soundx-safety-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn soundx<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_soundx"))
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
}

fn assert_fails(output: &Output) -> String {
    assert!(
        !output.status.success(),
        "expected failure, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    stderr(output)
}

/// Write an integer WAV; `amplitude` is relative to full scale.
fn write_int_wav(path: &Path, bits: u16, channels: u16, frames: usize, amplitude: f64) {
    let mut writer = WavWriter::create(
        path,
        WavSpec {
            channels,
            sample_rate: 44_100,
            bits_per_sample: bits,
            sample_format: SampleFormat::Int,
        },
    )
    .unwrap();
    let full = (1_i64 << (bits - 1)) as f64;
    for index in 0..frames {
        let phase = index as f64 / 44_100.0 * 440.0 * std::f64::consts::TAU;
        let value = ((phase.sin() * amplitude * full).round() as i64)
            .clamp(-(full as i64), full as i64 - 1);
        for _ in 0..channels {
            writer.write_sample(value as i32).unwrap();
        }
    }
    writer.finalize().unwrap();
}

fn write_float_wav(path: &Path, samples: &[f32]) {
    let mut writer = WavWriter::create(
        path,
        WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        },
    )
    .unwrap();
    for &sample in samples {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

fn read_ints(path: &Path) -> (WavSpec, Vec<i32>) {
    let mut reader = WavReader::open(path).unwrap();
    let spec = reader.spec();
    let samples = reader.samples::<i32>().map(|s| s.unwrap()).collect();
    (spec, samples)
}

fn read_floats(path: &Path) -> Vec<f32> {
    WavReader::open(path)
        .unwrap()
        .samples::<f32>()
        .map(|s| s.unwrap())
        .collect()
}

fn leftover_temp_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().contains("soundx-tmp"))
        .collect()
}

// 1. In-place processing and failure safety -------------------------------

#[test]
fn stream_gain_in_place_matches_separate_output() {
    let dir = temp_dir("inplace-gain");
    let source = dir.join("source.wav");
    let in_place = dir.join("x.wav");
    let separate = dir.join("separate.wav");
    write_int_wav(&source, 16, 2, 44_100, 0.5);
    std::fs::copy(&source, &in_place).unwrap();
    let original_len = std::fs::metadata(&in_place).unwrap().len();

    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&source),
        OsArg::p(&separate),
        OsArg::s("--gain-db=-3"),
    ]));
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&in_place),
        OsArg::p(&in_place),
        OsArg::s("--gain-db=-3"),
    ]));

    assert_eq!(std::fs::metadata(&in_place).unwrap().len(), original_len);
    assert_eq!(read_ints(&in_place).1, read_ints(&separate).1);
    assert!(leftover_temp_files(&dir).is_empty());
}

#[test]
fn stream_loudness_in_place_matches_separate_output() {
    let dir = temp_dir("inplace-loudness");
    let source = dir.join("source.wav");
    let in_place = dir.join("x.wav");
    let separate = dir.join("separate.wav");
    write_int_wav(&source, 16, 2, 4 * 44_100, 0.1);
    std::fs::copy(&source, &in_place).unwrap();
    let original_len = std::fs::metadata(&in_place).unwrap().len();

    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&source),
        OsArg::p(&separate),
        OsArg::s("--loudness-target=-18"),
    ]));
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&in_place),
        OsArg::p(&in_place),
        OsArg::s("--loudness-target=-18"),
    ]));

    assert_eq!(std::fs::metadata(&in_place).unwrap().len(), original_len);
    let (_, in_place_samples) = read_ints(&in_place);
    assert_eq!(in_place_samples, read_ints(&separate).1);
    assert_ne!(in_place_samples, read_ints(&source).1);
    assert!(leftover_temp_files(&dir).is_empty());
}

#[test]
fn convert_in_place_replaces_the_file() {
    let dir = temp_dir("inplace-convert");
    let file = dir.join("x.wav");
    write_int_wav(&file, 16, 1, 4_410, 0.5);
    let before = read_ints(&file).1;
    assert_ok(&soundx([
        OsArg::s("convert"),
        OsArg::p(&file),
        OsArg::p(&file),
        OsArg::s("--gain-db=-6"),
    ]));
    let after = read_ints(&file).1;
    assert_eq!(after.len(), before.len());
    assert!(after[1000].abs() < before[1000].abs());
    assert!(leftover_temp_files(&dir).is_empty());
}

#[test]
fn failed_conversion_keeps_existing_output_and_leaves_no_temp_files() {
    let dir = temp_dir("failed-convert");
    let input = dir.join("three.wav");
    let output = dir.join("out.mp3");
    write_int_wav(&input, 16, 3, 4_410, 0.5);
    std::fs::write(&output, b"precious existing content").unwrap();

    let result = soundx([OsArg::s("convert"), OsArg::p(&input), OsArg::p(&output)]);
    assert_fails(&result);
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"precious existing content"
    );
    assert!(leftover_temp_files(&dir).is_empty());

    // The SoX-style path shares the same guarantee.
    let result = soundx([
        OsArg::p(&input),
        OsArg::p(&output),
        OsArg::s("gain"),
        OsArg::s("-1"),
    ]);
    assert_fails(&result);
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"precious existing content"
    );
    assert!(leftover_temp_files(&dir).is_empty());
}

#[test]
fn failed_stream_keeps_existing_output_and_leaves_no_temp_files() {
    let dir = temp_dir("failed-stream");
    let input = dir.join("nan.wav");
    let output = dir.join("out.wav");
    write_float_wav(&input, &[0.1, f32::NAN, 0.2]);
    std::fs::write(&output, b"precious").unwrap();
    let result = soundx([OsArg::s("stream"), OsArg::p(&input), OsArg::p(&output)]);
    let message = assert_fails(&result);
    assert!(message.contains("NaN"), "{message}");
    assert_eq!(std::fs::read(&output).unwrap(), b"precious");
    assert!(leftover_temp_files(&dir).is_empty());
}

// 2. Negative numbers and non-finite values --------------------------------

#[test]
fn negative_numbers_are_accepted_as_option_values() {
    let dir = temp_dir("negative");
    let input = dir.join("in.wav");
    write_int_wav(&input, 16, 1, 22_050, 0.5);
    for (name, extra) in [
        ("gain", vec!["--gain-db", "-6"]),
        ("norm", vec!["--normalize", "--normalize-db", "-3"]),
        ("silence", vec!["--silence", "-40", "0.1"]),
    ] {
        let output = dir.join(format!("{name}.wav"));
        let result = soundx(
            [OsArg::s("convert"), OsArg::p(&input), OsArg::p(&output)]
                .into_iter()
                .chain(extra.into_iter().map(OsArg::s)),
        );
        assert_ok(&result);
        assert!(output.exists(), "{name}");
    }
    let output = dir.join("batch");
    assert_ok(&soundx([
        OsArg::s("batch"),
        OsArg::p(&input),
        OsArg::s("--out-dir"),
        OsArg::p(&output),
        OsArg::s("--gain-db"),
        OsArg::s("-6"),
    ]));
    assert_ok(&soundx([
        OsArg::s("synth"),
        OsArg::p(&dir.join("synth.wav")),
        OsArg::s("--gain-db"),
        OsArg::s("-6"),
    ]));
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&dir.join("stream.wav")),
        OsArg::s("--gain-db"),
        OsArg::s("-6"),
    ]));
}

#[test]
fn non_finite_numbers_are_rejected() {
    let dir = temp_dir("nonfinite");
    let input = dir.join("in.wav");
    let output = dir.join("out.wav");
    write_int_wav(&input, 16, 1, 4_410, 0.5);
    for args in [
        vec!["convert", "--gain-db", "nan"],
        vec!["convert", "--gain-db", "inf"],
        vec!["convert", "--speed", "nan"],
        vec!["convert", "--silence", "-40", "inf"],
        vec!["convert", "--loudness-target", "nan"],
    ] {
        let mut command = vec![OsArg::s(args[0]), OsArg::p(&input), OsArg::p(&output)];
        command.extend(args[1..].iter().map(|a| OsArg::s(a)));
        let message = assert_fails(&soundx(command));
        assert!(
            message.contains("finite") || message.contains("invalid"),
            "{args:?}: {message}"
        );
    }
    for effects in [
        vec!["gain", "nan"],
        vec!["trim", "0", "inf"],
        vec!["speed", "NaN"],
    ] {
        let mut command = vec![OsArg::p(&input), OsArg::p(&output)];
        command.extend(effects.iter().map(|a| OsArg::s(a)));
        assert_fails(&soundx(command));
    }
    assert!(!output.exists());
}

#[test]
fn normalize_db_requires_normalize() {
    let dir = temp_dir("normdb");
    let input = dir.join("in.wav");
    write_int_wav(&input, 16, 1, 4_410, 0.5);
    let result = soundx([
        OsArg::s("convert"),
        OsArg::p(&input),
        OsArg::p(&dir.join("out.wav")),
        OsArg::s("--normalize-db"),
        OsArg::s("-3"),
    ]);
    let message = assert_fails(&result);
    assert!(message.contains("--normalize"), "{message}");
}

// 3. Batch collisions -------------------------------------------------------

#[test]
fn batch_rejects_two_inputs_with_the_same_output() {
    let dir = temp_dir("batch-collide");
    std::fs::create_dir_all(dir.join("d1")).unwrap();
    std::fs::create_dir_all(dir.join("d2")).unwrap();
    write_int_wav(&dir.join("d1/x.wav"), 16, 1, 1_000, 0.5);
    write_int_wav(&dir.join("d2/x.wav"), 16, 1, 1_000, 0.2);
    let out = dir.join("out");
    let result = soundx([
        OsArg::s("batch"),
        OsArg::p(&dir.join("d1/x.wav")),
        OsArg::p(&dir.join("d2/x.wav")),
        OsArg::s("--out-dir"),
        OsArg::p(&out),
    ]);
    let message = assert_fails(&result);
    assert!(
        message.contains("d1") && message.contains("d2"),
        "{message}"
    );
    assert!(!out.join("x.wav").exists());
}

#[test]
fn batch_rejects_different_extensions_with_the_same_stem() {
    let dir = temp_dir("batch-stem");
    write_int_wav(&dir.join("x.wav"), 16, 1, 1_000, 0.5);
    std::fs::copy(dir.join("x.wav"), dir.join("x.flac")).unwrap();
    let out = dir.join("out");
    let result = soundx([
        OsArg::s("batch"),
        OsArg::p(&dir.join("x.wav")),
        OsArg::p(&dir.join("x.flac")),
        OsArg::s("--out-dir"),
        OsArg::p(&out),
    ]);
    assert_fails(&result);
    assert!(!out.exists() || !out.join("x.wav").exists());
}

#[test]
fn batch_rejects_output_equal_to_an_input() {
    let dir = temp_dir("batch-self");
    let input = dir.join("x.wav");
    write_int_wav(&input, 16, 1, 1_000, 0.5);
    let before = std::fs::read(&input).unwrap();
    let result = soundx([
        OsArg::s("batch"),
        OsArg::p(&input),
        OsArg::s("--out-dir"),
        OsArg::p(&dir),
        OsArg::s("--gain-db"),
        OsArg::s("-6"),
    ]);
    let message = assert_fails(&result);
    assert!(message.contains("overwrite"), "{message}");
    assert_eq!(std::fs::read(&input).unwrap(), before);
}

// 4. Glob handling ----------------------------------------------------------

#[test]
fn batch_accepts_literal_names_containing_glob_characters() {
    let dir = temp_dir("glob-literal");
    let input = dir.join("song [live].wav");
    write_int_wav(&input, 16, 1, 1_000, 0.5);
    let out = dir.join("out");
    assert_ok(&soundx([
        OsArg::s("batch"),
        OsArg::p(&input),
        OsArg::s("--out-dir"),
        OsArg::p(&out),
    ]));
    assert!(out.join("song [live].wav").exists());
}

#[test]
fn batch_errors_on_a_pattern_that_matches_nothing() {
    let dir = temp_dir("glob-empty");
    write_int_wav(&dir.join("a.wav"), 16, 1, 1_000, 0.5);
    let out = dir.join("out");
    let missing = dir.join("nothing-*.flac");
    let result = soundx([
        OsArg::s("batch"),
        OsArg::p(&dir.join("*.wav")),
        OsArg::p(&missing),
        OsArg::s("--out-dir"),
        OsArg::p(&out),
    ]);
    let message = assert_fails(&result);
    assert!(message.contains("nothing-*.flac"), "{message}");
    assert!(!out.join("a.wav").exists());
}

// 5. Shared effect order ----------------------------------------------------

#[test]
fn batch_and_convert_build_the_same_effect_chain() {
    let dir = temp_dir("order");
    let input = dir.join("in.wav");
    write_int_wav(&input, 16, 2, 22_050, 0.5);
    let converted = dir.join("converted.wav");
    assert_ok(&soundx([
        OsArg::s("convert"),
        OsArg::p(&input),
        OsArg::p(&converted),
        OsArg::s("--speed"),
        OsArg::s("1.5"),
        OsArg::s("--rate"),
        OsArg::s("22050"),
        OsArg::s("--channels"),
        OsArg::s("1"),
        OsArg::s("--lowpass"),
        OsArg::s("4000"),
    ]));
    let out = dir.join("out");
    assert_ok(&soundx([
        OsArg::s("batch"),
        OsArg::p(&input),
        OsArg::s("--out-dir"),
        OsArg::p(&out),
        OsArg::s("--speed"),
        OsArg::s("1.5"),
        OsArg::s("--rate"),
        OsArg::s("22050"),
        OsArg::s("--channels"),
        OsArg::s("1"),
        OsArg::s("--lowpass"),
        OsArg::s("4000"),
    ]));
    assert_eq!(read_ints(&out.join("in.wav")), read_ints(&converted));
}

// 6. run-plan validation ----------------------------------------------------

fn run_plan_text(dir: &Path, text: &str) -> Output {
    let plan = dir.join("plan.json");
    std::fs::write(&plan, text).unwrap();
    soundx([OsArg::s("run-plan"), OsArg::p(&plan)])
}

#[test]
fn run_plan_rejects_unknown_fields_and_extra_convert_inputs() {
    let dir = temp_dir("plan");
    let a = dir.join("a.wav").display().to_string().replace('\\', "/");
    let b = dir.join("b.wav").display().to_string().replace('\\', "/");
    let out = dir.join("out.wav").display().to_string().replace('\\', "/");
    write_int_wav(&dir.join("a.wav"), 16, 1, 1_000, 0.5);
    write_int_wav(&dir.join("b.wav"), 16, 1, 1_000, 0.5);

    let typo = format!(r#"{{"inputs":["{a}"],"output":"{out}","effect":["gain","-3"]}}"#);
    let message = assert_fails(&run_plan_text(&dir, &typo));
    assert!(message.contains("effect"), "{message}");

    let two = format!(r#"{{"inputs":["{a}","{b}"],"output":"{out}"}}"#);
    let message = assert_fails(&run_plan_text(&dir, &two));
    assert!(message.contains("exactly one"), "{message}");
    assert!(!dir.join("out.wav").exists());

    let good = format!(r#"{{"inputs":["{a}"],"output":"{out}","effects":["gain","-3"]}}"#);
    assert_ok(&run_plan_text(&dir, &good));
}

// 7. Legacy-mode detection --------------------------------------------------

#[test]
fn typoed_subcommands_are_reported_as_subcommands() {
    let dir = temp_dir("typo");
    let input = dir.join("a.wav");
    write_int_wav(&input, 16, 1, 1_000, 0.5);
    let message = assert_fails(&soundx([
        OsArg::s("convrt"),
        OsArg::p(&input),
        OsArg::p(&dir.join("b.wav")),
    ]));
    assert!(message.contains("unrecognized subcommand"), "{message}");
    let message = assert_fails(&soundx([OsArg::s("inof"), OsArg::p(&input)]));
    assert!(message.contains("unrecognized subcommand"), "{message}");
}

#[test]
fn sox_style_still_works_for_extensionless_existing_input() {
    let dir = temp_dir("legacy-bare");
    let input = dir.join("clip");
    write_int_wav(&input, 16, 1, 1_000, 0.5);
    let output = dir.join("out.wav");
    assert_ok(&soundx([
        OsArg::p(&input),
        OsArg::p(&output),
        OsArg::s("gain"),
        OsArg::s("-3"),
    ]));
    assert!(output.exists());
}

#[cfg(windows)]
#[test]
fn non_utf8_arguments_do_not_panic() {
    use std::os::windows::ffi::OsStringExt;
    let bad = std::ffi::OsString::from_wide(&[0x61, 0xD800, 0x2E, 0x77, 0x61, 0x76]);
    let result = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .arg(&bad)
        .arg("out.wav")
        .output()
        .unwrap();
    assert!(!stderr(&result).contains("panicked"), "{}", stderr(&result));
    let result = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!stderr(&result).contains("panicked"), "{}", stderr(&result));
}

#[cfg(unix)]
#[test]
fn non_utf8_arguments_do_not_panic() {
    use std::os::unix::ffi::OsStringExt;
    let bad = std::ffi::OsString::from_vec(vec![b'a', 0xFF, b'.', b'w', b'a', b'v']);
    let result = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .arg(&bad)
        .arg("out.wav")
        .output()
        .unwrap();
    assert!(!stderr(&result).contains("panicked"), "{}", stderr(&result));
}

// 8. stream bit depth and quantization --------------------------------------

#[test]
fn stream_defaults_to_input_depth_and_honours_bits_and_float() {
    let dir = temp_dir("depth");
    let input = dir.join("in24.wav");
    write_int_wav(&input, 24, 1, 4_410, 0.5);

    let default = dir.join("default.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&default),
    ]));
    let spec = WavReader::open(&default).unwrap().spec();
    assert_eq!(
        (spec.bits_per_sample, spec.sample_format),
        (24, SampleFormat::Int)
    );
    assert_eq!(read_ints(&default).1, read_ints(&input).1);

    let sixteen = dir.join("sixteen.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&sixteen),
        OsArg::s("--bits"),
        OsArg::s("16"),
    ]));
    assert_eq!(
        WavReader::open(&sixteen).unwrap().spec().bits_per_sample,
        16
    );

    let float = dir.join("float.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&float),
        OsArg::s("--float"),
    ]));
    let spec = WavReader::open(&float).unwrap().spec();
    assert_eq!(
        (spec.bits_per_sample, spec.sample_format),
        (32, SampleFormat::Float)
    );

    let eight = dir.join("eight.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&eight),
        OsArg::s("--bits"),
        OsArg::s("8"),
    ]));
    assert_eq!(WavReader::open(&eight).unwrap().spec().bits_per_sample, 8);
}

#[test]
fn stream_and_convert_produce_identical_samples() {
    let dir = temp_dir("identical");
    let input = dir.join("in24.wav");
    write_int_wav(&input, 24, 2, 22_050, 0.9);
    for bits in ["24", "16"] {
        let streamed = dir.join(format!("stream{bits}.wav"));
        let converted = dir.join(format!("convert{bits}.wav"));
        let mut stream = vec![
            OsArg::s("stream"),
            OsArg::p(&input),
            OsArg::p(&streamed),
            OsArg::s("--gain-db=-6"),
        ];
        stream.extend([OsArg::s("--bits"), OsArg::s(bits)]);
        assert_ok(&soundx(stream));
        assert_ok(&soundx([
            OsArg::s("convert"),
            OsArg::p(&input),
            OsArg::p(&converted),
            OsArg::s("--gain-db=-6"),
            OsArg::s("--bits"),
            OsArg::s(bits),
        ]));
        assert_eq!(read_ints(&streamed), read_ints(&converted), "{bits}-bit");
    }
    // Plain 24-bit default (no --bits) matches convert --bits 24.
    let streamed = dir.join("stream-default.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&streamed),
        OsArg::s("--gain-db=-6"),
    ]));
    assert_eq!(read_ints(&streamed), read_ints(&dir.join("convert24.wav")));
}

#[test]
fn stream_clamps_integer_output_when_gain_overflows() {
    let dir = temp_dir("clamp-int");
    let input = dir.join("in.wav");
    write_int_wav(&input, 16, 1, 4_410, 0.9);
    let output = dir.join("out.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&output),
        OsArg::s("--gain-db=12"),
    ]));
    let (_, samples) = read_ints(&output);
    assert_eq!(*samples.iter().max().unwrap(), i32::from(i16::MAX));
    assert_eq!(*samples.iter().min().unwrap(), i32::from(i16::MIN));
}

// 9. Float samples above full scale are not clamped on read ------------------

#[test]
fn stream_float_output_keeps_samples_above_full_scale() {
    let dir = temp_dir("float-over");
    let input = dir.join("hot.wav");
    write_float_wav(&input, &[0.5, 2.0, -3.0, 0.25]);
    let output = dir.join("out.wav");
    assert_ok(&soundx([
        OsArg::s("stream"),
        OsArg::p(&input),
        OsArg::p(&output),
    ]));
    assert_eq!(read_floats(&output), vec![0.5, 2.0, -3.0, 0.25]);

    // Loudness path: true peak of an over-full-scale float file is measured
    // above 0 dBTP rather than being clamped to full scale.
    let report = soundx([
        OsArg::s("loudness"),
        OsArg::s("--json"),
        OsArg::p(&hot_tone(&dir)),
    ]);
    assert_ok(&report);
    let json: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    let text = json.to_string();
    assert!(text.contains("true_peak"), "{text}");
    let peak = find_number(&json, "true_peak_dbtp").or_else(|| find_number(&json, "true_peak"));
    assert!(peak.is_some_and(|p| p > 3.0), "{text}");
}

fn hot_tone(dir: &Path) -> PathBuf {
    let path = dir.join("hot-tone.wav");
    let samples: Vec<f32> = (0..44_100)
        .map(|i| (i as f32 / 44_100.0 * 440.0 * std::f32::consts::TAU).sin() * 2.0)
        .collect();
    write_float_wav(&path, &samples);
    path
}

fn find_number(value: &serde_json::Value, key: &str) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => map.iter().find_map(|(k, v)| {
            if k.starts_with(key) {
                v.as_f64()
            } else {
                find_number(v, key)
            }
        }),
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_number(v, key)),
        _ => None,
    }
}

// Argument helper ------------------------------------------------------------

struct OsArg;

impl OsArg {
    fn s(value: &str) -> std::ffi::OsString {
        std::ffi::OsString::from(value)
    }

    fn p(value: &Path) -> std::ffi::OsString {
        value.as_os_str().to_owned()
    }
}
