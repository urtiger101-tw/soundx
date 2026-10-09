mod audio;
mod cli;
mod codecs;
mod device;
mod effects;
mod encode;
mod integrations;
mod io;
mod limiter;
mod loudness;
mod mcp;
mod mix;
mod parse;
mod stats;
mod streaming;
mod synth;
mod util;
mod windows_audio;

use anyhow::{Context, Result, anyhow, bail};
use audio::AudioBuffer;
use clap::Parser;
use cli::{
    BatchArgs, Cli, Commands, ConcatArgs, ConvertArgs, InfoArgs, LegacyCommand, LoudnessArgs,
    MixArgs, PlanMode, ProcessingPlan, RunPlanArgs, StreamArgs, SynthArgs,
};
use effects::{Effect, EffectChain};
use parse::parse_effects;
use rayon::prelude::*;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let legacy = LegacyCommand::from_env()?;
    if let Some(command) = legacy {
        return run_legacy(command);
    }

    let cli = Cli::parse();
    match cli.command {
        Commands::Info(args) => run_info(args),
        Commands::Loudness(args) => run_loudness(args),
        Commands::Formats => run_formats(),
        Commands::Devices => device::list_devices(),
        Commands::Play(args) => device::play_file(args),
        Commands::Record(args) => device::record_file(args),
        Commands::Convert(args) => run_convert(args),
        Commands::Concat(args) => run_concat(args),
        Commands::Mix(args) => run_mix(args),
        Commands::Synth(args) => run_synth(args),
        Commands::Stream(args) => run_stream(args),
        Commands::Batch(args) => run_batch(args),
        Commands::RunPlan(args) => run_plan(args),
        Commands::Mcp => mcp::serve(),
        Commands::Integrate(args) => integrations::run(args),
        Commands::Windows(args) => {
            let result = match args.command {
                cli::WindowsCommand::Devices => windows_audio::endpoints()?,
                cli::WindowsCommand::Endpoint {
                    device,
                    flow,
                    volume,
                    mute,
                } => windows_audio::endpoint(device.as_deref(), &flow, volume, mute)?,
                cli::WindowsCommand::Sessions { device } => {
                    windows_audio::sessions(device.as_deref())?
                }
                cli::WindowsCommand::Session {
                    device,
                    session_id,
                    volume,
                    mute,
                } => windows_audio::session(device.as_deref(), &session_id, volume, mute)?,
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
    }
}

fn run_formats() -> Result<()> {
    println!(
        "read: wav (PCM, float, IMA/MS ADPCM), gsm (GSM 06.10), amr/awb (AMR-NB/WB), wv (WavPack v5 lossless mono/stereo), aiff, au/snd, raw, flac, mp3, ogg/vorbis, opus, aac, alac, caf, mkv/webm, mp4/m4a"
    );
    println!(
        "write: wav (pcm8/16/24/32, float32/64, ima-adpcm, ms-adpcm), gsm (GSM 06.10), amr (AMR-NB), awb (AMR-WB), wv (WavPack v5 lossless mono/stereo float), aiff, flac, mp3, ogg/vorbis, aac/adts, au/snd, raw"
    );
    println!("wav: 16-bit PCM by default; convert --bits 8/16/24/32, or --bits 32/64 --float");
    println!(
        "wav ADPCM: convert INPUT OUTPUT.wav --wav-adpcm ima|ms; GSM is 8 kHz mono; AMR-NB/WB uses .amr/.awb"
    );
    println!(
        "codec limits: MP3 mono/stereo; Vorbis 1-2 channels at 44100/48000 Hz; AAC up to 6 channels"
    );
    println!(
        "stream: wav -> wav for gain, fade, limiter, and --loudness-target / --true-peak without loading the full file"
    );
    println!(
        "loudness: loudness INPUT (ITU-R BS.1770-4 / EBU R128: integrated, LRA, true peak); convert --loudness-target LUFS [--true-peak dBTP]"
    );
    println!(
        "device: devices, in-memory multi-file/repeat play, and finite/continuous WAV record use CPAL; select device, rate, and channels where supported"
    );
    println!(
        "effects: gain, normalize, trim, fade, reverse, speed, stretch, tempo, dither, compand, reverb, pad, silence, lowpass, highpass, bass, treble, allpass, bandpass, bandreject, equalizer, echo, tremolo, delay, dcshift, downsample, upsample, repeat, swap, limiter, rate, channels, stat"
    );
    Ok(())
}

fn run_info(args: InfoArgs) -> Result<()> {
    let mut reports = Vec::with_capacity(args.inputs.len());
    for input in args.inputs {
        let audio = AudioBuffer::read(&input)
            .with_context(|| format!("failed to read {}", input.display()))?;
        reports.push(stats::Report::from_audio(&input, &audio));
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        for report in reports {
            println!("{report}");
        }
    }

    Ok(())
}

/// Measure one file. WAV files are streamed with bounded memory; everything
/// else is decoded first.
fn measure_path(path: &std::path::Path) -> Result<loudness::LoudnessReport> {
    let is_wav = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"));
    if is_wav && let Ok(report) = streaming::measure_wav(path) {
        return Ok(report);
    }
    let audio =
        AudioBuffer::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    loudness::measure(&audio.samples, audio.spec.sample_rate, audio.spec.channels)
}

fn run_loudness(args: LoudnessArgs) -> Result<()> {
    let mut reports = Vec::with_capacity(args.inputs.len());
    for input in &args.inputs {
        let report = measure_path(input)
            .with_context(|| format!("failed to measure {}", input.display()))?;
        reports.push(loudness::FileReport::new(input, report));
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        for report in reports {
            println!("{report}");
        }
    }
    Ok(())
}

fn run_convert(args: ConvertArgs) -> Result<()> {
    let chain = args.effect_chain()?;
    let goal = args.loudness_goal()?;
    let mut audio = AudioBuffer::read_with_raw_options(
        &args.input,
        args.input_raw_rate,
        args.input_raw_channels,
        args.input_raw_encoding,
    )
    .with_context(|| format!("failed to read {}", args.input.display()))?;
    chain.apply(&mut audio)?;
    let loudness_outcome = match goal {
        Some(goal) => {
            let outcome = limiter::normalize_buffer(
                &mut audio.samples,
                audio.spec.sample_rate,
                audio.spec.channels,
                goal,
                args.stat_json,
            )?;
            if goal.target_lufs.is_some() && outcome.input.integrated_lufs.is_none() {
                eprintln!("warning: input is silent (below the -70 LUFS gate); no gain applied");
            }
            Some(outcome)
        }
        None => None,
    };
    encode::write_audio_with_options(
        &audio,
        &args.output,
        encode::EncodeOptions {
            wav_bits: args.bits,
            wav_float: args.float,
            wav_adpcm: args.wav_adpcm,
            aiff_bits: args.aiff_bits,
            bitrate_kbps: args.bitrate_kbps,
            au_encoding: args.au_encoding,
            raw_encoding: args.output_raw_encoding,
        },
    )?;
    if args.stat_json || chain.wants_stats() {
        let report = stats::Report::from_audio(&args.output, &audio);
        if args.stat_json {
            let mut value = serde_json::to_value(&report)?;
            if let Some(outcome) = &loudness_outcome {
                value["loudness"] = serde_json::to_value(outcome)?;
            }
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            println!("{report}");
        }
    }
    Ok(())
}

fn run_concat(args: ConcatArgs) -> Result<()> {
    let mut audio = mix::concat_inputs(&args.inputs)?;
    if args.normalize {
        EffectChain::new(vec![Effect::Normalize { target_db: -1.0 }]).apply(&mut audio)?;
    }
    io::write_output(&audio, &args.output, args.stat_json, args.stat_json)
}

fn run_mix(args: MixArgs) -> Result<()> {
    let mut audio = mix::mix_inputs(&args.inputs)?;
    if args.normalize {
        EffectChain::new(vec![Effect::Normalize { target_db: -1.0 }]).apply(&mut audio)?;
    }
    io::write_output(&audio, &args.output, args.stat_json, args.stat_json)
}

fn run_synth(args: SynthArgs) -> Result<()> {
    let mut audio = synth::render(&args)?;
    let chain = args.effect_chain()?;
    chain.apply(&mut audio)?;
    io::write_output(
        &audio,
        &args.output,
        args.stat_json || chain.wants_stats(),
        args.stat_json,
    )
}

fn run_stream(args: StreamArgs) -> Result<()> {
    streaming::stream_wav(&args)
}

fn run_batch(args: BatchArgs) -> Result<()> {
    let inputs = io::expand_inputs(&args.inputs)?;
    if inputs.is_empty() {
        bail!("no input files matched");
    }

    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("failed to create {}", args.out_dir.display()))?;

    let chain = args.effect_chain()?;
    let extension = args.ext.trim_start_matches('.').to_string();
    let failures: Vec<_> = inputs
        .par_iter()
        .filter_map(|input| {
            let output = io::output_for_batch(input, &args.out_dir, &extension);
            io::convert_one(input, &output, &chain, args.stat_json)
                .err()
                .map(|err| format!("{}: {err:#}", input.display()))
        })
        .collect();

    if !failures.is_empty() {
        for failure in &failures {
            eprintln!("{failure}");
        }
        bail!("{} batch item(s) failed", failures.len());
    }

    Ok(())
}

fn run_legacy(command: LegacyCommand) -> Result<()> {
    let chain = EffectChain::new(command.effects);
    io::convert_one(&command.input, &command.output, &chain, command.stat_json)
}

fn run_plan(args: RunPlanArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.plan)
        .with_context(|| format!("failed to read {}", args.plan.display()))?;
    let plan: ProcessingPlan = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", args.plan.display()))?;
    let (effects, effect_stat_json) = parse_effects(&plan.effects)?;
    let chain = EffectChain::new(effects);
    let stat_json = plan.stat_json || effect_stat_json;

    match plan.mode {
        PlanMode::Convert => {
            let input = plan
                .inputs
                .first()
                .ok_or_else(|| anyhow!("convert plan requires at least one input"))?;
            io::convert_one(input, &plan.output, &chain, stat_json)
        }
        PlanMode::Concat => {
            let mut audio = mix::concat_inputs(&plan.inputs)?;
            chain.apply(&mut audio)?;
            if plan.normalize {
                EffectChain::new(vec![Effect::Normalize { target_db: -1.0 }]).apply(&mut audio)?;
            }
            io::write_output(
                &audio,
                &plan.output,
                stat_json || chain.wants_stats(),
                stat_json,
            )
        }
        PlanMode::Mix => {
            let mut audio = mix::mix_inputs(&plan.inputs)?;
            chain.apply(&mut audio)?;
            if plan.normalize {
                EffectChain::new(vec![Effect::Normalize { target_db: -1.0 }]).apply(&mut audio)?;
            }
            io::write_output(
                &audio,
                &plan.output,
                stat_json || chain.wants_stats(),
                stat_json,
            )
        }
    }
}
