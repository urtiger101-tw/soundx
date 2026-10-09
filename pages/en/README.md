# soundx — A Rust-first, SoX-inspired Audio Processor

**Rust-native codecs and DSP · SoX-style CLI**

`soundx` is a Rust-native audio processor with a SoX-style command line. It aims
to implement SoX format, effect, and device compatibility incrementally; it is
currently a subset and is not fully SoX-compatible.

> **Status:** Active development. Some codecs, effects, parameters, and device
> behaviors supported by SoX are still missing.

---

## Agent integration and Windows audio (v0.2)

The Windows installer provides optional Skill and MCP installation for Codex,
Claude Code, OpenCode and AGY / Antigravity. The built-in stdio server runs with
`soundx mcp`; portable users can run `soundx integrate install --agent codex`.
`soundx windows devices`, `endpoint` and `sessions` expose native Windows audio
devices, master volume/mute and application sessions as JSON.
See [agent setup and tools](../../docs/AGENT_INTEGRATION.md) for all commands.

## Why soundx?

- **Rust-native DSP and codecs** — system audio I/O uses the host backend through CPAL.
- **Deterministic pipeline** — all effects operate on an interleaved `f32` sample buffer, making the signal path predictable and testable.
- **SoX-style CLI subset** — familiar `input output effect...` syntax for implemented workflows, plus subcommands.
- **Streaming** — process large files incrementally without loading everything into memory.
- **JSON plans** — repeatable, scriptable processing plans.
- **Parallel batch** — multi-core batch conversion with a single command.
- **Built-in synthesis** — generate tones and waveforms via `synth`.

---

## Quick Start

```bash
# Build from source
git clone https://github.com/urtiger101-tw/sox-rs.git
cd sox-rs
cargo build --release
./target/release/soundx --help
# Or install into Cargo's bin directory
cargo install --path .
```

```bash
# Info
soundx info input.wav
soundx info --json input.wav

# Basic conversion with effects
soundx convert input.wav out.wav --gain-db -3 --trim 0 10 --normalize

# SoX-style legacy syntax
soundx input.wav out.wav gain -3 trim 0 10 norm rate 48000 stat

# Concatenate
soundx concat -o album.wav intro.wav body.wav outro.wav

# Mix
soundx mix -o bed.wav voice.wav music.wav --normalize

# Synthesis
soundx synth tone.wav --duration 2 --freq 440 --waveform sine --fade 0.05

# Streaming (low-memory large file processing)
soundx stream huge.wav processed.wav --gain-db=-3 --fade-in 0.5 --fade-out 0.5

# Loudness (ITU-R BS.1770-4 / EBU R128): measure, then normalise to -18 LUFS with a -1.8 dBTP ceiling
soundx loudness track.wav --json
soundx convert in.wav out.wav --loudness-target=-18 --true-peak=-1.8
soundx stream huge.wav out.wav --loudness-target=-18 --true-peak=-1.8

# List supported codecs
soundx formats

# Parallel batch
soundx batch "samples/*.wav" --out-dir out --normalize --rate 48000

# Run from a JSON plan
soundx run-plan plan.json

# List devices, play a file, and record for a fixed duration
soundx devices
soundx play intro.wav chapter1.wav chapter2.wav --loop
soundx record take.wav --duration 10
soundx record live.wav --continuous

# Additional native Rust codecs
soundx convert input.wav speech.gsm
soundx convert input.wav speech.amr
soundx convert input.wav speech-wide.awb
soundx convert input.wav lossless.wv
```

---

## Installation

### Pre-built binaries

Download from [GitHub Releases](https://github.com/stevenke1981/sox-rs/releases):

| Platform | Package | Binary |
|----------|---------|--------|
| Windows x86_64 | `soundx-<version>-setup.exe` (installer) or `soundx-<version>-x86_64-pc-windows-msvc.zip` | `soundx.exe` |
| Linux x86_64 | `soundx-<version>-x86_64-unknown-linux-gnu.tar.gz` | `soundx` |
| macOS x86_64 | `soundx-<version>-x86_64-apple-darwin.tar.gz` | `soundx` |

The Windows installer adds its installation folder to the current user's `PATH`
by default. Open a new terminal after installation, then run `soundx --version`.

### From source

```bash
cargo install --path .
# or
cargo build --release
# binary at target/release/soundx (or soundx.exe on Windows)
```

---

## Supported Effects

| Effect | Syntax | Description |
|--------|--------|-------------|
| `gain` | `gain <db>` | Apply gain in decibels |
| `norm` | `norm [target-db]` | Normalise peak amplitude (default −1 dBFS) |
| `trim` | `trim <start-sec> [duration-sec]` | Cut from start, optionally for a duration |
| `fade` | `fade <in-sec> [out-sec]` | Linear fade in/out |
| `reverse` | `reverse` | Reverse samples in place |
| `speed` | `speed <factor>` | Change playback speed (resamples) |
| `stretch` / `tempo` | `stretch <factor>` / `tempo [-q|-m|-s|-l] <factor>` | WSOLA time change with approximate pitch preservation |
| `dither` | `dither [bits]` | Deterministic TPDF dither and quantization |
| `compand` | `compand attack,decay,... transfer-points [gain [initial-volume [delay]]]` | Envelope-controlled dynamic range processing |
| `reverb` | `reverb [-w] [reverberance [HF-damping [room-scale ...]]]` | Freeverb-style room effect with optional wet-only output |
| `pad` | `pad <start-sec> [end-sec]` | Add silence at beginning and/or end |
| `silence` | `silence <threshold-db> [min-sec]` | Remove leading/trailing silence |
| `lowpass` | `lowpass [-1|-2] <hz> [width[q|o|h|k]]` | First- or second-order low-pass filter |
| `highpass` | `highpass [-1|-2] <hz> [width[q|o|h|k]]` | First- or second-order high-pass filter |
| `bass` / `treble` | `bass|treble <gain-db> [hz [width[s|h|k|q|o]]]` | Low- and high-shelf tone filters |
| `allpass` / `bandpass` / `bandreject` | `<effect> <hz> <width[h|k|q|o]>` | Two-pole all-pass, band-pass, and band-reject filters |
| `equalizer` | `equalizer <hz> <width[q|o|h|k]> <gain-db>` | Two-pole parametric peak EQ |
| `echo` | `echo <gain-in> <gain-out> <delay-ms> <decay> [...]` | Add one or more delay taps |
| `tremolo` | `tremolo <speed-hz> [depth-percent]` | Sinusoidal amplitude modulation |
| `delay` | `delay <position-sec> [...]` | Delay all channels or specify channel positions |
| `dcshift` | `dcshift <shift> [limitergain]` | Add or remove DC offset |
| `downsample` / `upsample` | `[factor (default 2)]` | Drop samples or insert zero samples and adjust rate |
| `repeat` | `repeat [count (default 1)]` | Repeat the complete input |
| `swap` | `swap` | Swap the first two channels |
| `limiter` | `limiter [threshold]` | Hard limiter (default 0.95) |
| `rate` | `rate <hz>` | Resample to a new sample rate |
| `channels` | `channels <count>` | Convert channel count |
| `stat` | `stat` | Print audio statistics |

## Synth Waveforms

- Waveforms: `sine`, `square`, `triangle`, `saw`, `noise`, `silence`
- Post-processing: `--gain-db`, `--normalize`, `--fade`

## Codec Support

| Direction | Formats |
|-----------|---------|
| **Read** | WAV (PCM/float, IMA/MS ADPCM), GSM 06.10, AMR-NB/WB, WavPack v5, AIFF, AU/SND, RAW, FLAC, MP3, Ogg/Vorbis, Opus, AAC, ALAC, CAF, MKV/WebM, M4A |
| **Write** | WAV (PCM/float, IMA/MS ADPCM), GSM 06.10, AMR-NB/WB, WavPack v5, FLAC, MP3, Ogg/Vorbis, AAC/ADTS, AIFF, AU/SND, RAW |
| **Stream** | WAV → WAV (gain, fade, limiter only) |
| **Devices** | List host devices, multi-file and repeat playback, duration-limited or continuous WAV recording, select device/rate/channels |

WavPack currently supports lossless v5 mono/stereo. GSM and AMR use mono speech frames. Multi-file playback buffers the playlist in memory; continuous recording writes 16-bit WAV through a bounded queue. This remains a SoX-style subset; see the [compatibility matrix](../../docs/SOX_COMPATIBILITY.md) for codec/effect limits. Recording needs an available input device and OS permission.

`play --loop` repeats until Ctrl+C without a single-pass timeout. Continuous recording stops with an error at the RIFF WAV size limit (approximately 4 GiB) and finalizes the recorded prefix; RF64 is unsupported.

RAW input needs `convert --input-raw-rate HZ --input-raw-channels N`; its default encoding is `pcm-s16le`. RAW output accepts `--output-raw-encoding` and defaults to the same encoding. AU/SND encoding can be selected with `convert --au-encoding pcm8|pcm16|pcm24|pcm32|float32|float64|mu-law|a-law`; the default is 16-bit PCM.
AIFF defaults to 16-bit PCM; select 8, 16, 24, or 32 bits with `convert --aiff-bits`.

---

## Project Layout

```
soundx/                    # crate root
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs            # Entry point, command dispatch
│   ├── cli.rs             # Clap argument definitions
│   ├── audio.rs           # AudioBuffer, WAV/AU/RAW and Symphonia input
│   ├── encode.rs          # Container and audio encoders
│   ├── codecs.rs          # ADPCM, GSM, AMR, and WavPack codecs
│   ├── device.rs          # Device listing, playback, timed/continuous recording
│   ├── effects.rs         # Effect enum, EffectChain, DSP
│   ├── parse.rs           # Effect token parser
│   ├── io.rs              # File I/O utilities
│   ├── mix.rs             # Concat and mix
│   ├── streaming.rs       # Incremental WAV pipeline
│   ├── synth.rs           # Waveform generation
│   ├── stats.rs           # Audio statistics
│   └── util.rs            # Shared utility functions
├── tests/
│   ├── cli.rs             # CLI integration tests
│   ├── effects.rs         # Effect unit tests
│   └── common/mod.rs      # Test helpers
├── docs/
│   ├── ARCHITECTURE.md
│   ├── KNOWLEDGE_MAP.md
│   └── ROADMAP.md
└── pages/
    ├── en/README.md
    └── zh-TW/README.md
```

---

## Build & Install

```powershell
# Debug build
cargo build

# Release build
cargo build --release

# binary at target/release/soundx (or soundx.exe)
./target/release/soundx --help
```

**Prerequisites:** [Rust](https://www.rust-lang.org/tools/install) 1.94.1 (pinned by `rust-toolchain.toml`; the AMR dependency requires Rust 1.91+).

---

## License

Dual-licensed under **LGPL-2.1-or-later** or **MIT** at your option.
