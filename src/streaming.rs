use crate::cli::StreamArgs;
use crate::limiter::{self, LoudnessGoal};
use crate::loudness::{LoudnessMeter, LoudnessReport};
use crate::util::seconds_to_frames;
use anyhow::{Context, Result, bail};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::path::Path;

/// Frames per chunk when streaming WAV data for loudness work.
const CHUNK_FRAMES: usize = 16_384;

pub fn stream_wav(args: &StreamArgs) -> Result<()> {
    if args.loudness_target.is_some() || args.true_peak.is_some() {
        return stream_loudness(args);
    }
    if args.bits.is_some() || args.float || args.stat_json {
        bail!("--bits, --float and --stat-json require --loudness-target or --true-peak");
    }
    if !(0.0..=1.0).contains(&args.limiter) {
        bail!("stream limiter must be between 0 and 1");
    }
    if args.fade_in.is_some_and(|value| value < 0.0)
        || args.fade_out.is_some_and(|value| value < 0.0)
    {
        bail!("stream fade durations must be >= 0");
    }

    let mut reader = WavReader::open(&args.input)
        .with_context(|| format!("failed to open {}", args.input.display()))?;
    let input_spec = reader.spec();
    if input_spec.channels == 0 {
        bail!("WAV file has zero channels");
    }

    let output_spec = WavSpec {
        channels: input_spec.channels,
        sample_rate: input_spec.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    if let Some(parent) = args
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut writer = WavWriter::create(&args.output, output_spec)
        .with_context(|| format!("failed to create {}", args.output.display()))?;

    let channels = u32::from(input_spec.channels);
    let total_frames = reader.duration() / channels;
    let gain = args
        .gain_db
        .map(|db| 10.0_f32.powf(db / 20.0))
        .unwrap_or(1.0);
    let fade_in_frames =
        seconds_to_frames(args.fade_in.unwrap_or(0.0), input_spec.sample_rate) as u32;
    let fade_out_frames =
        seconds_to_frames(args.fade_out.unwrap_or(0.0), input_spec.sample_rate) as u32;

    match input_spec.sample_format {
        SampleFormat::Float => stream_samples::<f32, _>(
            reader.samples::<f32>(),
            &mut writer,
            StreamSettings {
                channels,
                total_frames,
                gain,
                fade_in_frames,
                fade_out_frames,
                limiter: args.limiter,
            },
            |value| value,
        )?,
        SampleFormat::Int => match input_spec.bits_per_sample {
            0 => bail!("WAV file has zero bits per sample"),
            1..=8 => {
                let max = ((1_i32 << (input_spec.bits_per_sample - 1)) - 1) as f32;
                stream_samples::<i8, _>(
                    reader.samples::<i8>(),
                    &mut writer,
                    StreamSettings {
                        channels,
                        total_frames,
                        gain,
                        fade_in_frames,
                        fade_out_frames,
                        limiter: args.limiter,
                    },
                    |value| value as f32 / max,
                )?
            }
            9..=16 => {
                let max = ((1_i32 << (input_spec.bits_per_sample - 1)) - 1) as f32;
                stream_samples::<i16, _>(
                    reader.samples::<i16>(),
                    &mut writer,
                    StreamSettings {
                        channels,
                        total_frames,
                        gain,
                        fade_in_frames,
                        fade_out_frames,
                        limiter: args.limiter,
                    },
                    |value| value as f32 / max,
                )?
            }
            17..=32 => {
                let max = ((1_i64 << (input_spec.bits_per_sample - 1)) - 1) as f32;
                stream_samples::<i32, _>(
                    reader.samples::<i32>(),
                    &mut writer,
                    StreamSettings {
                        channels,
                        total_frames,
                        gain,
                        fade_in_frames,
                        fade_out_frames,
                        limiter: args.limiter,
                    },
                    |value| value as f32 / max,
                )?
            }
            bits => bail!("unsupported integer WAV depth: {bits}"),
        },
    }

    writer.finalize()?;
    Ok(())
}

#[derive(Clone, Copy)]
struct StreamSettings {
    channels: u32,
    total_frames: u32,
    gain: f32,
    fade_in_frames: u32,
    fade_out_frames: u32,
    limiter: f32,
}

fn stream_samples<T, F>(
    samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, T>,
    writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>,
    settings: StreamSettings,
    to_f32: F,
) -> Result<()>
where
    T: hound::Sample,
    F: Fn(T) -> f32,
{
    for (sample_index, sample) in samples.enumerate() {
        let frame = sample_index as u32 / settings.channels;
        let mut value = to_f32(sample?) * settings.gain;
        value *= fade_factor(
            frame,
            settings.total_frames,
            settings.fade_in_frames,
            settings.fade_out_frames,
        );
        value = value.clamp(-settings.limiter, settings.limiter);
        writer.write_sample((value.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)?;
    }

    Ok(())
}

fn fade_factor(frame: u32, total_frames: u32, fade_in_frames: u32, fade_out_frames: u32) -> f32 {
    let fade_in = if fade_in_frames > 0 {
        (frame as f32 / fade_in_frames as f32).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let fade_out = if fade_out_frames > 0 && total_frames > 0 {
        let frames_from_end = total_frames.saturating_sub(frame + 1);
        (frames_from_end as f32 / fade_out_frames as f32).clamp(0.0, 1.0)
    } else {
        1.0
    };
    fade_in.min(fade_out)
}

/// Stream a WAV file as chunks of whole interleaved `f32` frames. Integer
/// samples are scaled to [-1, 1) and float samples are clamped, exactly like
/// the in-memory reader, so streaming and in-memory results agree.
pub fn replay_wav(path: &Path, emit: &mut dyn FnMut(&[f32]) -> Result<()>) -> Result<()> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels == 0 {
        bail!("WAV file has zero channels");
    }
    let chunk = CHUNK_FRAMES * usize::from(spec.channels);
    match spec.sample_format {
        SampleFormat::Float => {
            replay_samples(reader.samples::<f32>(), chunk, |v| v.clamp(-1.0, 1.0), emit)
        }
        SampleFormat::Int => {
            let scale = match spec.bits_per_sample {
                0 => bail!("WAV file has zero bits per sample"),
                bits @ 1..=32 => 1.0 / (1_u64 << (bits - 1)) as f32,
                bits => bail!("unsupported integer WAV depth: {bits}"),
            };
            match spec.bits_per_sample {
                1..=8 => replay_samples(
                    reader.samples::<i8>(),
                    chunk,
                    |v| i32::from(v) as f32 * scale,
                    emit,
                ),
                9..=16 => replay_samples(
                    reader.samples::<i16>(),
                    chunk,
                    |v| i32::from(v) as f32 * scale,
                    emit,
                ),
                _ => replay_samples(reader.samples::<i32>(), chunk, |v| v as f32 * scale, emit),
            }
        }
    }
}

fn replay_samples<T>(
    samples: impl Iterator<Item = hound::Result<T>>,
    chunk: usize,
    convert: impl Fn(T) -> f32,
    emit: &mut dyn FnMut(&[f32]) -> Result<()>,
) -> Result<()> {
    let mut buffer = Vec::with_capacity(chunk);
    for sample in samples {
        buffer.push(convert(sample?));
        if buffer.len() == chunk {
            emit(&buffer)?;
            buffer.clear();
        }
    }
    emit(&buffer)
}

/// Measure a WAV file with bounded memory.
pub fn measure_wav(path: &Path) -> Result<LoudnessReport> {
    let spec = WavReader::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?
        .spec();
    let mut meter = LoudnessMeter::new(spec.sample_rate, spec.channels)?;
    replay_wav(path, &mut |chunk| {
        meter.push(chunk);
        Ok(())
    })?;
    Ok(meter.finish())
}

struct WavSink {
    writer: WavWriter<std::io::BufWriter<std::fs::File>>,
    bits: u16,
    float: bool,
}

impl WavSink {
    fn write(&mut self, samples: &[f32]) -> Result<()> {
        if self.float {
            for &sample in samples {
                self.writer.write_sample(sample)?;
            }
        } else {
            let scale = ((1_u64 << (self.bits - 1)) - 1) as f32;
            for &sample in samples {
                let value = (sample.clamp(-1.0, 1.0) * scale).round() as i32;
                self.writer.write_sample(value)?;
            }
        }
        Ok(())
    }
}

/// Two-pass (plus refinement passes when the limiter engages) loudness
/// normalisation of a WAV file with bounded memory.
fn stream_loudness(args: &StreamArgs) -> Result<()> {
    let goal = LoudnessGoal {
        target_lufs: args.loudness_target,
        ceiling_dbtp: args.true_peak.unwrap_or(limiter::DEFAULT_CEILING_DBTP),
    };
    goal.validate()?;

    let input_spec = WavReader::open(&args.input)
        .with_context(|| format!("failed to open {}", args.input.display()))?
        .spec();
    if input_spec.channels == 0 {
        bail!("WAV file has zero channels");
    }
    let (bits, float) = match (args.bits, args.float) {
        (None, false) => (
            input_spec.bits_per_sample,
            input_spec.sample_format == SampleFormat::Float,
        ),
        (None, true) => (32, true),
        (Some(bits), float) => (bits, float),
    };
    if !matches!(bits, 8 | 16 | 24 | 32) || (float && bits != 32) {
        bail!("WAV output requires 8, 16, 24, or 32 bits; float requires 32 bits");
    }
    if let Some(parent) = args
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let writer = WavWriter::create(
        &args.output,
        WavSpec {
            channels: input_spec.channels,
            sample_rate: input_spec.sample_rate,
            bits_per_sample: bits,
            sample_format: if float {
                SampleFormat::Float
            } else {
                SampleFormat::Int
            },
        },
    )
    .with_context(|| format!("failed to create {}", args.output.display()))?;
    let mut sink = WavSink {
        writer,
        bits,
        float,
    };

    let input = args.input.clone();
    let outcome = limiter::normalize(
        &mut |emit: &mut dyn FnMut(&[f32]) -> Result<()>| replay_wav(&input, emit),
        input_spec.sample_rate,
        input_spec.channels,
        goal,
        args.stat_json,
        &mut |chunk: &[f32]| sink.write(chunk),
    )?;
    sink.writer.finalize()?;

    if args.loudness_target.is_some() && outcome.input.integrated_lufs.is_none() {
        eprintln!("warning: input is silent (below the -70 LUFS gate); no gain applied");
    }
    if args.stat_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "loudness": outcome }))?
        );
    }
    Ok(())
}
