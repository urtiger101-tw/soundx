# Architecture

`soundx` is split around three processing paths:

- **AudioBuffer path**: decodes supported inputs into interleaved `f32` samples,
  applies `EffectChain`, and sends audio to an extension-selected encoder.
- **Codec path**: WAV/AU/RAW and SoX-specific GSM, AMR, and WavPack readers and
  writers handle formats that need dedicated parsing or encoding; Symphonia
  provides the general compressed/container decode path.
- **Streaming path**: processes WAV samples incrementally for large-file gain,
  fade, and limiter workflows.
- **Synth path**: generates built-in waveforms into `AudioBuffer`, then reuses the
  same effect chain and format encoders.
- **Device path**: CPAL enumerates the system host, plays converted multi-file
  playlists (optionally repeating), and records finite or continuous WAV input.

WAV, AU/SND, RAW, GSM, AMR, and WavPack use dedicated readers; remaining
compressed/container inputs use Symphonia. Writers include WAV PCM/float and
ADPCM, GSM, AMR, WavPack, FLAC, MP3, Vorbis, AAC/ADTS, AIFF, AU, and RAW.
Codec-specific channel, sample-rate, and bitrate constraints are validated by
the encoder layer. Effects include deterministic dither, compand, Freeverb-style
reverb, and WSOLA stretch/tempo, but behavior is still a SoX-compatible subset.

## Module Layout

```
src/
├── main.rs        Entry point + command dispatch
├── cli.rs         CLI args, subcommands, legacy parser
├── audio.rs       AudioBuffer, WAV/AU/RAW and Symphonia I/O
├── codecs.rs      ADPCM, GSM, AMR, and WavPack codecs
├── encode.rs      Container and audio format encoders
├── device.rs      Device listing, multi-file playback, timed/continuous recording
├── effects.rs     Effect enum, EffectChain, DSP
├── parse.rs       Effect token parser
├── io.rs          File I/O utilities
├── mix.rs         Concat and mix
├── streaming.rs   Incremental WAV pipeline and streaming loudness normalisation
├── loudness.rs    ITU-R BS.1770-4 / EBU R128 meter (K-weighting, gating, LRA, true peak)
├── limiter.rs     Lookahead true-peak limiter and loudness normalisation flow
├── synth.rs       Waveform generation
├── stats.rs       Audio statistics (peak, RMS, clipping)
└── util.rs        Shared utility functions
```
