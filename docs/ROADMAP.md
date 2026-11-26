# soundx Roadmap

## Implemented foundation (partial SoX compatibility)

- Implement portable WAV I/O.
- Represent samples as normalized interleaved `f32`.
- Add common effects: gain, normalize, trim, fade, reverse, resample, channel
  conversion.
- Provide SoX-style positional command parsing.
- Add JSON metadata and statistics for automation.
- Add parallel batch processing.
- Add multi-file concat/mix workflows.
- Add JSON processing plans for repeatable jobs.
- Add built-in synth generation for tones, noise, and silence.
- Add WAV/FLAC/MP3/Vorbis/AAC/AIFF/AU writers and AU/SND input.
- Add system device listing, playback, and duration-limited recording through CPAL.
- Add multi-file/repeat playback and continuous WAV capture through CPAL.
- Add initial echo and tremolo implementations.
- Add basic delay, DC shift, sample insertion/decimation, repeat, and channel swap.
- Add WAV IMA/MS ADPCM, GSM 06.10, AMR-NB/WB, and WavPack v5 mono/stereo.
- Add dither, compand, Freeverb-style reverb, and WSOLA stretch/tempo effects.

## Remaining compatibility work

- Expand codecs and containers to match SoX-supported formats and verify sample
  format, metadata, seeking, streaming, and malformed-input behavior.
- Add the missing SoX effects and option semantics. Current filters and effects
  are a smaller subset and are not bit-exact to upstream SoX.
- Add device hot switching, simultaneous multi-device routing, and broader
  host-specific device behaviors.
- Expand streaming support; most current decode/effect/encode paths load the
  complete input into memory.

## DSP compatibility validation

- Port SoX effects incrementally with golden tests against upstream output.
- Compare supported effects against upstream SoX with golden audio, including
  `vol`, `gain`, `trim`, `fade`, `reverse`, `speed`, `pad`, `rate`,
  `channels`, `silence`, `lowpass`, `highpass`, `limiter`, `echo`, `tremolo`,
  `delay`, `dcshift`, `downsample`, `upsample`, `repeat`, and `swap`.
- Add more complex filters after the test harness can compare spectra and
  sample tolerances.

## Milestone 4: New Rust-First Features

- Richer JSON plans with per-step names, comments, and batch targets.
- Parallel batch jobs with per-file reports.
- Safer clipping diagnostics and optional true-peak estimation.
- More streaming effects beyond gain/fade/limiter.
- WASM-compatible DSP core for future UI or browser use.
