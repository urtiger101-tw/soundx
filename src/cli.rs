use crate::effects::EffectChain;
use crate::parse::parse_effects;
use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print WAV metadata and level statistics.
    Info(InfoArgs),
    /// Measure ITU-R BS.1770-4 / EBU R128 loudness, loudness range and true peak.
    Loudness(LoudnessArgs),
    /// List built-in format support.
    Formats,
    /// List available system audio input and output devices.
    Devices,
    /// Preload one or more audio files into memory, then play them in sequence.
    Play(PlayArgs),
    /// Record audio for a fixed duration or stream continuous 16-bit WAV audio.
    Record(RecordArgs),
    /// Convert one input file into one output file.
    Convert(ConvertArgs),
    /// Concatenate WAV files in order.
    Concat(ConcatArgs),
    /// Mix WAV files into one output.
    Mix(MixArgs),
    /// Generate audio from built-in waveforms.
    Synth(SynthArgs),
    /// Process WAV in a streaming path for very large files.
    Stream(StreamArgs),
    /// Convert many files in parallel.
    Batch(BatchArgs),
    /// Run a repeatable JSON processing plan.
    RunPlan(RunPlanArgs),
    /// Serve the local Model Context Protocol over stdin/stdout (no network listener).
    Mcp,
    /// Install or remove soundx Skill and MCP configuration for an agent.
    Integrate(IntegrationArgs),
    /// Windows Core Audio endpoint and per-application volume control (JSON output).
    Windows(WindowsArgs),
}

#[derive(Debug, Args)]
pub struct IntegrationArgs {
    #[command(subcommand)]
    pub command: IntegrationCommand,
}

#[derive(Debug, Subcommand)]
pub enum IntegrationCommand {
    /// Show supported agent targets and their current integration state.
    List {
        /// Override user home, for a portable profile or isolated installation.
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install both the Skill and stdio MCP entry. Existing unrelated settings are preserved.
    Install {
        #[arg(long, value_enum)]
        agent: AgentTarget,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Remove only unchanged soundx-managed files and MCP entries.
    Remove {
        #[arg(
            long,
            value_enum,
            required_unless_present = "all",
            conflicts_with = "all"
        )]
        agent: Option<AgentTarget>,
        #[arg(long)]
        all: bool,
        /// Only remove registrations referring to this executable (used by the uninstaller).
        #[arg(long)]
        only_executable: Option<PathBuf>,
        #[arg(long)]
        home: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AgentTarget {
    Codex,
    Claude,
    Opencode,
    Agy,
}

#[derive(Debug, Args)]
pub struct WindowsArgs {
    #[command(subcommand)]
    pub command: WindowsCommand,
}

#[derive(Debug, Subcommand)]
pub enum WindowsCommand {
    /// List active Core Audio endpoints, defaults, volume and mute state.
    Devices,
    /// Read volume/mute, or supply --volume/--mute to set and read back.
    Endpoint {
        #[arg(long)]
        device: Option<String>,
        #[arg(long, default_value = "output", value_parser = ["input", "output"])]
        flow: String,
        /// Volume percentage, 0 through 100.
        #[arg(long)]
        volume: Option<f32>,
        #[arg(long, action = clap::ArgAction::Set)]
        mute: Option<bool>,
    },
    /// List audio sessions on the selected output endpoint.
    Sessions {
        #[arg(long)]
        device: Option<String>,
    },
    /// Read or adjust a session selected by its exact session instance id.
    Session {
        session_id: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        volume: Option<f32>,
        #[arg(long, action = clap::ArgAction::Set)]
        mute: Option<bool>,
    },
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LoudnessArgs {
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,
    /// Print one JSON array with an object per input.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlayArgs {
    #[arg(required = true, num_args = 1..)]
    pub inputs: Vec<PathBuf>,
    /// Repeat the concatenated input sequence until Ctrl+C.
    #[arg(long = "loop", alias = "repeat")]
    pub loop_play: bool,
    /// Select a device by the name printed by `soundx devices`.
    #[arg(long)]
    pub device: Option<String>,
    /// Request an output sample rate supported by the selected device.
    #[arg(long, alias = "sample-rate")]
    pub rate: Option<u32>,
    /// Request an output channel count supported by the selected device.
    #[arg(long)]
    pub channels: Option<u16>,
}

#[derive(Debug, Args)]
pub struct RecordArgs {
    pub output: PathBuf,
    /// Recording duration in seconds (ignored with --continuous).
    #[arg(long, default_value_t = 5.0)]
    pub duration: f64,
    /// Record until Ctrl+C and stream a 16-bit PCM WAV to disk.
    #[arg(long)]
    pub continuous: bool,
    /// Select a device by the name printed by `soundx devices`.
    #[arg(long)]
    pub device: Option<String>,
    /// Request an input sample rate supported by the selected device.
    #[arg(long, alias = "sample-rate")]
    pub rate: Option<u32>,
    /// Request an input channel count supported by the selected device.
    #[arg(long)]
    pub channels: Option<u16>,
}

#[derive(Debug, Args)]
pub struct ConvertArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// RAW input sample rate; required when the input extension is .raw.
    #[arg(long)]
    pub input_raw_rate: Option<u32>,
    /// RAW input channel count; required when the input extension is .raw.
    #[arg(long)]
    pub input_raw_channels: Option<u16>,
    /// RAW input sample encoding (default: pcm-s16le).
    #[arg(long, value_enum)]
    pub input_raw_encoding: Option<crate::audio::RawEncoding>,
    /// RAW output sample encoding (default: pcm-s16le).
    #[arg(long, value_enum)]
    pub output_raw_encoding: Option<crate::audio::RawEncoding>,
    /// WAV output depth: 8/16/24/32-bit PCM or 32/64-bit floating point.
    #[arg(long)]
    pub bits: Option<u16>,
    /// Write floating-point WAV (requires --bits 32 or --bits 64).
    #[arg(long)]
    pub float: bool,
    /// Write WAV IMA or Microsoft ADPCM instead of PCM.
    #[arg(long, value_enum)]
    pub wav_adpcm: Option<crate::codecs::WavAdpcmEncoding>,
    /// AIFF output depth: 8, 16, 24, or 32 bits (default: 16).
    #[arg(long)]
    pub aiff_bits: Option<u16>,
    /// Target bitrate for MP3/AAC output, in kilobits per second.
    #[arg(long, default_value_t = 192)]
    pub bitrate_kbps: u32,
    /// AU/SND encoding. Defaults to 16-bit PCM.
    #[arg(long, value_enum)]
    pub au_encoding: Option<crate::encode::AuEncoding>,
    #[arg(long)]
    pub gain_db: Option<f32>,
    #[arg(long)]
    pub normalize: bool,
    #[arg(long, default_value_t = -1.0)]
    pub normalize_db: f32,
    /// Target integrated loudness in LUFS (ITU-R BS.1770-4), e.g. --loudness-target=-18.
    /// Applied last: linear gain, then a true-peak limiter when the ceiling would be exceeded.
    #[arg(long, allow_negative_numbers = true, conflicts_with = "normalize")]
    pub loudness_target: Option<f64>,
    /// True-peak ceiling in dBTP for --loudness-target (default -1.0); alone it only limits peaks.
    #[arg(long, allow_negative_numbers = true, conflicts_with = "normalize")]
    pub true_peak: Option<f64>,
    #[arg(long, num_args = 1..=2, value_names = ["START", "DURATION"])]
    pub trim: Vec<f32>,
    #[arg(long, num_args = 1..=2, value_names = ["IN", "OUT"])]
    pub fade: Vec<f32>,
    #[arg(long)]
    pub reverse: bool,
    #[arg(long)]
    pub speed: Option<f32>,
    #[arg(long)]
    pub lowpass: Option<f32>,
    #[arg(long)]
    pub highpass: Option<f32>,
    #[arg(long)]
    pub limiter: Option<f32>,
    #[arg(long, num_args = 1..=2, value_names = ["START", "END"])]
    pub pad: Vec<f32>,
    #[arg(long, num_args = 1..=2, value_names = ["THRESHOLD_DB", "MIN_SECONDS"])]
    pub silence: Vec<f32>,
    #[arg(long)]
    pub rate: Option<u32>,
    #[arg(long)]
    pub channels: Option<u16>,
    #[arg(long)]
    pub stat_json: bool,
}

#[derive(Debug, Args)]
pub struct BatchArgs {
    #[arg(required = true)]
    pub inputs: Vec<PathBuf>,
    #[arg(long)]
    pub out_dir: PathBuf,
    #[arg(long, default_value = "wav")]
    pub ext: String,
    #[arg(long)]
    pub gain_db: Option<f32>,
    #[arg(long)]
    pub normalize: bool,
    #[arg(long, default_value_t = -1.0)]
    pub normalize_db: f32,
    #[arg(long)]
    pub rate: Option<u32>,
    #[arg(long)]
    pub channels: Option<u16>,
    #[arg(long)]
    pub speed: Option<f32>,
    #[arg(long)]
    pub lowpass: Option<f32>,
    #[arg(long)]
    pub highpass: Option<f32>,
    #[arg(long)]
    pub limiter: Option<f32>,
    #[arg(long)]
    pub stat_json: bool,
}

#[derive(Debug)]
pub struct LegacyCommand {
    pub input: PathBuf,
    pub output: PathBuf,
    pub effects: Vec<crate::effects::Effect>,
    pub stat_json: bool,
}

#[derive(Debug, Args)]
pub struct ConcatArgs {
    #[arg(required = true, num_args = 2..)]
    pub inputs: Vec<PathBuf>,
    #[arg(short, long)]
    pub output: PathBuf,
    #[arg(long)]
    pub normalize: bool,
    #[arg(long)]
    pub stat_json: bool,
}

#[derive(Debug, Args)]
pub struct MixArgs {
    #[arg(required = true, num_args = 2..)]
    pub inputs: Vec<PathBuf>,
    #[arg(short, long)]
    pub output: PathBuf,
    #[arg(long)]
    pub normalize: bool,
    #[arg(long)]
    pub stat_json: bool,
}

#[derive(Debug, Args)]
pub struct RunPlanArgs {
    pub plan: PathBuf,
}

#[derive(Debug, Args)]
pub struct SynthArgs {
    pub output: PathBuf,
    #[arg(long, default_value_t = 1.0)]
    pub duration: f32,
    #[arg(long, default_value_t = 440.0)]
    pub freq: f32,
    #[arg(long, default_value_t = 44_100)]
    pub rate: u32,
    #[arg(long, default_value_t = 1)]
    pub channels: u16,
    #[arg(long, default_value_t = 0.5)]
    pub amplitude: f32,
    #[arg(long, value_enum, default_value_t = Waveform::Sine)]
    pub waveform: Waveform,
    #[arg(long)]
    pub gain_db: Option<f32>,
    #[arg(long)]
    pub normalize: bool,
    #[arg(long)]
    pub fade: Option<f32>,
    #[arg(long)]
    pub stat_json: bool,
}

#[derive(Debug, Args)]
pub struct StreamArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    #[arg(long)]
    pub gain_db: Option<f32>,
    #[arg(long)]
    pub fade_in: Option<f32>,
    #[arg(long)]
    pub fade_out: Option<f32>,
    #[arg(long, default_value_t = 1.0)]
    pub limiter: f32,
    /// Target integrated loudness in LUFS, measured and applied in streaming passes.
    #[arg(
        long,
        allow_negative_numbers = true,
        conflicts_with_all = ["gain_db", "fade_in", "fade_out", "limiter"]
    )]
    pub loudness_target: Option<f64>,
    /// True-peak ceiling in dBTP (default -1.0 with --loudness-target); alone it only limits peaks.
    #[arg(
        long,
        allow_negative_numbers = true,
        conflicts_with_all = ["gain_db", "fade_in", "fade_out", "limiter"]
    )]
    pub true_peak: Option<f64>,
    /// Output depth: 8, 16, 24 or 32 bits (default: 16, or the input depth with loudness options).
    #[arg(long)]
    pub bits: Option<u16>,
    /// Write 32-bit floating-point samples.
    #[arg(long)]
    pub float: bool,
    /// Print the loudness measurements as JSON (loudness options only).
    #[arg(long)]
    pub stat_json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Waveform {
    Sine,
    Square,
    Triangle,
    Saw,
    Noise,
    Silence,
}

#[derive(Debug, Deserialize)]
pub struct ProcessingPlan {
    #[serde(default)]
    pub mode: PlanMode,
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    #[serde(default)]
    pub effects: Vec<String>,
    #[serde(default)]
    pub normalize: bool,
    #[serde(default)]
    pub stat_json: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanMode {
    #[default]
    Convert,
    Concat,
    Mix,
}

impl LegacyCommand {
    pub fn from_env() -> Result<Option<Self>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() || is_subcommand(&args[0]) || args[0].starts_with('-') {
            return Ok(None);
        }

        if args.len() < 2 {
            bail!("SoX-style usage requires <input> <output> [effect ...]");
        }

        let input = PathBuf::from(&args[0]);
        let output = PathBuf::from(&args[1]);
        let (effects, stat_json) = parse_effects(&args[2..])?;
        Ok(Some(Self {
            input,
            output,
            effects,
            stat_json,
        }))
    }
}

impl ConvertArgs {
    /// Loudness goal requested with `--loudness-target` / `--true-peak`.
    pub fn loudness_goal(&self) -> Result<Option<crate::limiter::LoudnessGoal>> {
        if self.loudness_target.is_none() && self.true_peak.is_none() {
            return Ok(None);
        }
        let goal = crate::limiter::LoudnessGoal {
            target_lufs: self.loudness_target,
            ceiling_dbtp: self
                .true_peak
                .unwrap_or(crate::limiter::DEFAULT_CEILING_DBTP),
        };
        goal.validate()?;
        Ok(Some(goal))
    }

    pub fn effect_chain(&self) -> Result<EffectChain> {
        let mut effects = Vec::new();
        if let Some(db) = self.gain_db {
            effects.push(crate::effects::Effect::GainDb(db));
        }
        if self.normalize {
            effects.push(crate::effects::Effect::Normalize {
                target_db: self.normalize_db,
            });
        }
        if !self.trim.is_empty() {
            effects.push(crate::effects::Effect::Trim {
                start_sec: self.trim[0],
                duration_sec: self.trim.get(1).copied(),
            });
        }
        if !self.fade.is_empty() {
            effects.push(crate::effects::Effect::Fade {
                in_sec: self.fade[0],
                out_sec: self.fade.get(1).copied().unwrap_or(0.0),
            });
        }
        if self.reverse {
            effects.push(crate::effects::Effect::Reverse);
        }
        if let Some(factor) = self.speed {
            effects.push(crate::effects::Effect::Speed { factor });
        }
        if let Some(hz) = self.lowpass {
            effects.push(crate::effects::Effect::LowPass { hz });
        }
        if let Some(hz) = self.highpass {
            effects.push(crate::effects::Effect::HighPass { hz });
        }
        if let Some(threshold) = self.limiter {
            effects.push(crate::effects::Effect::Limiter { threshold });
        }
        if !self.pad.is_empty() {
            effects.push(crate::effects::Effect::Pad {
                start_sec: self.pad[0],
                end_sec: self.pad.get(1).copied().unwrap_or(0.0),
            });
        }
        if !self.silence.is_empty() {
            effects.push(crate::effects::Effect::Silence {
                threshold_db: self.silence[0],
                min_duration_sec: self.silence.get(1).copied().unwrap_or(0.1),
            });
        }
        if let Some(sample_rate) = self.rate {
            effects.push(crate::effects::Effect::Rate { sample_rate });
        }
        if let Some(channels) = self.channels {
            effects.push(crate::effects::Effect::Channels { channels });
        }
        Ok(EffectChain::new(effects))
    }
}

impl BatchArgs {
    pub fn effect_chain(&self) -> Result<EffectChain> {
        let mut effects = Vec::new();
        if let Some(db) = self.gain_db {
            effects.push(crate::effects::Effect::GainDb(db));
        }
        if self.normalize {
            effects.push(crate::effects::Effect::Normalize {
                target_db: self.normalize_db,
            });
        }
        if let Some(sample_rate) = self.rate {
            effects.push(crate::effects::Effect::Rate { sample_rate });
        }
        if let Some(channels) = self.channels {
            effects.push(crate::effects::Effect::Channels { channels });
        }
        if let Some(factor) = self.speed {
            effects.push(crate::effects::Effect::Speed { factor });
        }
        if let Some(hz) = self.lowpass {
            effects.push(crate::effects::Effect::LowPass { hz });
        }
        if let Some(hz) = self.highpass {
            effects.push(crate::effects::Effect::HighPass { hz });
        }
        if let Some(threshold) = self.limiter {
            effects.push(crate::effects::Effect::Limiter { threshold });
        }
        Ok(EffectChain::new(effects))
    }
}

impl SynthArgs {
    pub fn effect_chain(&self) -> Result<EffectChain> {
        let mut effects = Vec::new();
        if let Some(db) = self.gain_db {
            effects.push(crate::effects::Effect::GainDb(db));
        }
        if self.normalize {
            effects.push(crate::effects::Effect::Normalize { target_db: -1.0 });
        }
        if let Some(duration) = self.fade {
            effects.push(crate::effects::Effect::Fade {
                in_sec: duration,
                out_sec: duration,
            });
        }
        Ok(EffectChain::new(effects))
    }
}

fn is_subcommand(value: &str) -> bool {
    matches!(
        value,
        "info"
            | "loudness"
            | "formats"
            | "devices"
            | "play"
            | "record"
            | "convert"
            | "concat"
            | "mix"
            | "synth"
            | "stream"
            | "batch"
            | "run-plan"
            | "mcp"
            | "integrate"
            | "windows"
            | "help"
    )
}
