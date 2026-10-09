# SoX compatibility status

This matrix tracks the implemented `soundx` subset against SoX 14.4.2. It does
not claim command-for-command or sample-for-sample parity.

## Effects

| Effect | Implemented behavior | Remaining difference |
|---|---|---|
| `dither [bits]` | Deterministic TPDF noise followed by signed PCM quantization; defaults to 16 bits. | No SoX automatic output-depth negotiation, noise-shaping modes, or randomized seed option. |
| `compand attack,decay,... transfer-points [gain [initial-volume [delay]]]` | Per-channel envelope follower, piecewise dB transfer, optional soft knee, gain, initial envelope, and look-ahead control. | Simplified parameter grammar and interpolation; no exact SoX envelope or multiband behavior. |
| `reverb [-w|--wet-only] [reverberance [HF-damping [room-scale [stereo-depth [pre-delay-ms [wet-gain-dB]]]]]]` | Freeverb-style comb/all-pass network, dry/wet or wet-only output, and generated tail. | Parameter mapping, stereo spread, and tail are native approximations rather than SoX's exact Freeverb output. |
| `stretch factor [window-ms [search-ms [overlap-ms]]]` | WSOLA overlap search changes duration while retaining approximate pitch. | Simplified tuning grammar and no SoX quality-parity guarantee. |
| `tempo [-q|-m|-s|-l] factor [segment-ms [search-ms [overlap-ms]]]` | WSOLA changes playback tempo while retaining approximate pitch. | Quality flags select simplified window presets; results are not sample-exact to SoX. |
| Existing effects | `gain`/`vol`, `normalize`, `trim`, `fade`, `pad`, `delay`, `repeat`, `reverse`, `speed`, filters, `echo`, `tremolo`, `dcshift`, `limiter`, `silence`, `rate`, `channels`, and analysis. | Argument forms and DSP remain a partial SoX-compatible subset. `rate` is linear interpolation; `speed` changes pitch. |

Still unsupported are effects including `band`, `bend`, `biquad`, `chorus`,
`contrast`, `deemph`, `divide`, `earwax`, `echos`, `fir`, `flanger`,
`hilbert`, `ladspa`, `loudness` (the SoX equal-loudness effect; the soundx
`loudness` subcommand measures EBU R128 instead), `mcompand`, `noiseprof`, `noisered`, `oops`,
`overdrive`, `phaser`, `pitch`, `remix`, `riaa`, `sinc`, `spectrogram`,
`splice`, and `vad`.

## Formats and encodings

| Direction | Current support |
|---|---|
| Read | WAV PCM/float and IMA/MS ADPCM; AIFF; AU/SND; headerless RAW PCM/float/μ-law/A-law; FLAC, MP3, Ogg/Vorbis, Opus, AAC, ALAC, CAF, MKV/WebM, MP4/M4A via Symphonia; GSM 06.10 raw frames; AMR-NB `.amr` and AMR-WB `.awb`; WavPack v5 `.wv`. |
| Write | WAV PCM/float and IMA/MS ADPCM; AIFF PCM; FLAC, MP3, Ogg/Vorbis, AAC/ADTS, AU/SND, RAW; GSM 06.10; AMR-NB `.amr`; AMR-WB `.awb`; WavPack v5 `.wv`. |

Format-specific limits:

- WAV ADPCM output uses IMA ADPCM (1–8 channels) or Microsoft ADPCM (1–2
  channels); `--wav-adpcm` cannot be combined with `--bits` or `--float`.
- GSM 06.10 raw files use 8 kHz mono frames. The final partial frame is padded.
- AMR storage files are single-channel. Input detects NB/WB from the magic;
  output uses `.amr` for NB and `.awb` for WB, pads the final 20 ms frame, and
  disables DTX.
- WavPack support is limited to v5 lossless mono/stereo; hybrid/lossy, DSD,
  correction files, legacy streams, and multichannel are unsupported.
- RAW input needs `--input-raw-rate` and `--input-raw-channels`; output defaults
  to signed 16-bit little-endian PCM.
- FLAC, MP3, Vorbis, AAC, and WavPack retain codec-specific channel, rate, or
  bit-depth limits. WAV 64-bit float is stored through the app's 32-bit float
  processing buffer.

AMR is provided by the MIT-licensed `rvoip-codec-core` pure-Rust implementation;
WAV ADPCM encoding uses `oxideav-adpcm`, GSM uses `oxideav-gsm`, and WavPack v5
uses the in-repository MIT/Apache-2.0 `wavicle` copy. No native SoX/FFmpeg codec
backend is linked for these formats.

## Devices

`devices` lists devices on CPAL's default system host. `play INPUT...` decodes
each input, converts each to the selected device format, concatenates them in
memory, and plays the sequence; `--loop` repeats until Ctrl+C. `record` captures
for a fixed duration. `record OUTPUT.wav --continuous` streams 16-bit PCM to
disk until Ctrl+C through a bounded queue; if the writer falls behind, capture
stops with an error and the partial WAV is finalized. Both finite and continuous
recording use the selected device, sample rate, and channel configuration.

Continuous recording also stops with an error before RIFF's 32-bit size fields
overflow (approximately 4 GiB), finalizing the recorded prefix on a complete
frame. RF64 output is not implemented. PCM16 chunks reuse the writer's buffer.
Repeated playback has no single-pass deadline; finite playback uses the actual
playlist duration plus a device timeout allowance, including playlists longer
than 24 hours.

Device behavior still depends on CPAL and the host OS audio backend. The tool
does not implement SoX's backend-specific options, device hot switching,
simultaneous multi-device routing, or full `-d` semantics. Multi-file playback
currently buffers the combined audio in memory.

## Recheck after changes

Use `soundx --help` and `soundx formats` for the live command surface. Automated
tests use synthetic fixtures and local round trips; they do not establish
sample-exact parity with SoX or validate physical microphones and speakers.
