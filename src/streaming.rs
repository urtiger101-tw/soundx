use crate::audio::quantize_pcm;
use crate::cli::StreamArgs;
use crate::io::TempOutput;
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
    if args.stat_json {
        bail!("--stat-json requires --loudness-target or --true-peak");
    }
    if !(0.0..=1.0).contains(&args.limiter) {
        bail!("stream limiter must be between 0 and 1");
    }
    if args.fade_in.is_some_and(|value| value < 0.0)
        || args.fade_out.is_some_and(|value| value < 0.0)
    {
        bail!("stream fade durations must be >= 0");
    }

    let (input_spec, duration) = {
        let reader = WavReader::open(&args.input)
            .with_context(|| format!("failed to open {}", args.input.display()))?;
        (reader.spec(), reader.duration())
    };
    if input_spec.channels == 0 {
        bail!("WAV file has zero channels");
    }
    let (bits, float) = resolve_output_format(&input_spec, args)?;

    // Declared before the sink so the writer is closed before a failed temp
    // file is removed.
    let temp = TempOutput::new(&args.output)?;
    let mut sink = WavSink::create(temp.path(), &input_spec, bits, float)?;

    let channels = u64::from(input_spec.channels);
    let total_frames = duration;
    let gain = args
        .gain_db
        .map(|db| 10.0_f32.powf(db / 20.0))
        .unwrap_or(1.0);
    let fade_in_frames =
        seconds_to_frames(args.fade_in.unwrap_or(0.0), input_spec.sample_rate) as u32;
    let fade_out_frames =
        seconds_to_frames(args.fade_out.unwrap_or(0.0), input_spec.sample_rate) as u32;
    // Float output is never clamped; integer output clamps while quantizing.
    let limiter = (args.limiter < 1.0).then_some(args.limiter);

    let mut sample_index: u64 = 0;
    replay_wav(&args.input, &mut |chunk| {
        for &sample in chunk {
            let frame = (sample_index / channels) as u32;
            sample_index += 1;
            let mut value = sample * gain;
            value *= fade_factor(frame, total_frames, fade_in_frames, fade_out_frames);
            if let Some(limit) = limiter {
                value = value.clamp(-limit, limit);
            }
            sink.write_sample(value)?;
        }
        Ok(())
    })?;

    sink.finish()?;
    temp.commit()
}

/// Output depth and sample format: explicit `--bits` / `--float`, otherwise
/// the input's own depth (rounded up to a supported size) and sample format.
fn resolve_output_format(input: &WavSpec, args: &StreamArgs) -> Result<(u16, bool)> {
    let (bits, float) = match (args.bits, args.float) {
        (None, false) => {
            if input.sample_format == SampleFormat::Float {
                (32, true)
            } else {
                let bits = match input.bits_per_sample {
                    0 => bail!("WAV file has zero bits per sample"),
                    1..=8 => 8,
                    9..=16 => 16,
                    17..=24 => 24,
                    _ => 32,
                };
                (bits, false)
            }
        }
        (None, true) => (32, true),
        (Some(bits), float) => (bits, float),
    };
    if !matches!(bits, 8 | 16 | 24 | 32) || (float && bits != 32) {
        bail!("WAV output requires 8, 16, 24, or 32 bits; float requires 32 bits");
    }
    Ok((bits, float))
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
/// samples are scaled to [-1, 1) like the in-memory reader. Float samples are
/// passed through unclamped (clamping happens only at integer quantization);
/// NaN and infinite samples are an error.
pub fn replay_wav(path: &Path, emit: &mut dyn FnMut(&[f32]) -> Result<()>) -> Result<()> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels == 0 {
        bail!("WAV file has zero channels");
    }
    let chunk = CHUNK_FRAMES * usize::from(spec.channels);
    match spec.sample_format {
        SampleFormat::Float => replay_samples(reader.samples::<f32>(), chunk, |v| v, emit),
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
        let value = convert(sample?);
        if !value.is_finite() {
            bail!("WAV file contains a NaN or infinite sample");
        }
        buffer.push(value);
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
    fn create(path: &Path, input: &WavSpec, bits: u16, float: bool) -> Result<Self> {
        let writer = WavWriter::create(
            path,
            WavSpec {
                channels: input.channels,
                sample_rate: input.sample_rate,
                bits_per_sample: bits,
                sample_format: if float {
                    SampleFormat::Float
                } else {
                    SampleFormat::Int
                },
            },
        )
        .with_context(|| format!("failed to create {}", path.display()))?;
        Ok(Self {
            writer,
            bits,
            float,
        })
    }

    fn write_sample(&mut self, sample: f32) -> Result<()> {
        if self.float {
            self.writer.write_sample(sample)?;
        } else {
            let value = quantize_pcm(sample, u32::from(self.bits)) as i32;
            match self.bits {
                8 => self.writer.write_sample(value as i8)?,
                16 => self.writer.write_sample(value as i16)?,
                _ => self.writer.write_sample(value)?,
            }
        }
        Ok(())
    }

    fn write(&mut self, samples: &[f32]) -> Result<()> {
        for &sample in samples {
            self.write_sample(sample)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        self.writer.finalize()?;
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
    let (bits, float) = resolve_output_format(&input_spec, args)?;
    // Declared before the sink so the writer is closed before a failed temp
    // file is removed.
    let temp = TempOutput::new(&args.output)?;
    let mut sink = WavSink::create(temp.path(), &input_spec, bits, float)?;

    let input = args.input.clone();
    let outcome = limiter::normalize(
        &mut |emit: &mut dyn FnMut(&[f32]) -> Result<()>| replay_wav(&input, emit),
        input_spec.sample_rate,
        input_spec.channels,
        goal,
        args.stat_json,
        &mut |chunk: &[f32]| sink.write(chunk),
    )?;
    sink.finish()?;
    temp.commit()?;

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
