# Changelog

## 0.4.0

### Behaviour changes (read before upgrading)

- **`batch` effect order now matches `convert`**: speed, low-pass, high-pass and
  limiter run before `--rate` / `--channels` (previously after). Batch output
  can differ from 0.3.0 for the same flags.
- **`stream` keeps the input bit depth and sample format by default** instead of
  always writing 16-bit PCM. `--bits` / `--float` are accepted without the
  loudness options.
- **Floating-point input is no longer clamped to ±1.0 on read.** `info`,
  `loudness`, `normalize` and the limiters now see the real peaks; float-to-float
  conversion preserves values above full scale. Integer outputs still clamp.
- **`rate` and `speed` use a band-limited Kaiser-windowed sinc resampler**
  instead of linear interpolation. Output is alias-free and numerically
  different from 0.3.0.
- **Multichannel to stereo/mono uses ITU-R BS.775 downmix coefficients**
  (centre and surrounds at -3 dB, LFE dropped) instead of discarding every
  channel after the first two. This also applies before GSM/AMR encoding.
- **The MCP server answers out of order**: `soundx_run` / `soundx_loudness`
  responses are sent when the command finishes, so other requests are served
  meanwhile. Non-Windows platforms list 5 tools instead of 9.
- Inputs that used to be silently accepted are now errors: NaN/Inf samples or
  parameters, unknown fields in a `run-plan` file, a convert-mode plan with more
  than one input, `--normalize-db` without `--normalize`, a `batch` pattern that
  matches nothing, and `integrate list|remove --home` on a missing directory.

### Fixed

- `stream IN OUT` with the same path destroyed the input file. Every command
  now writes to a temporary sibling file and renames it into place only after a
  complete write; a failed write leaves an existing output untouched.
- `loudness` reported wrong sample peak, true peak and integrated loudness for
  float files above full scale.
- Negative option values were rejected (`--gain-db -6`, `--normalize-db -3`,
  `--silence -40 0.1`).
- `batch` silently overwrote outputs when two inputs shared a file stem, and
  could overwrite its own inputs. Collisions are detected before any processing.
- Existing files whose names contain `[`, `?` or `*` were treated as glob
  patterns and not found.
- `synth` accumulated phase in `f32`: a 1 kHz tone had about 40 dB SNR after
  60 s and 14 dB after 600 s. Phase is now computed in `f64`.
- `trim` / `pad` / `synth` with huge durations overflowed (wrapping to an empty
  file in release builds) or aborted on allocation; they now saturate or return
  a readable error.
- `--gain-db nan` wrote a silent file and exited 0.
- A mistyped subcommand fell into SoX-style mode with a misleading error; it now
  gets "unrecognized subcommand" with suggestions. Non-UTF-8 arguments no longer
  panic.
- `stream` fade-out started at the wrong position for multichannel files.
- `stream` and `convert` quantized differently; both now share one quantizer.
- MCP: a long `soundx_run` blocked `ping`, `soundx_jobs` and `soundx_stop` for
  up to 600 s. `notifications/cancelled` is now honoured; at most 8 finite runs
  may be in flight.
- `integrate` replaced symlinked agent config files with regular files. Links
  are now followed and preserved.
- Release CI: the tag must match the crate version; the first release no longer
  gets an empty changelog.

### Changed

- `integrate` keeps only the 5 newest config backups per agent.
- The stale tracked binary `dist/rust-sox.exe` is no longer in the repository.

### Known limitations

- The symlink handling in `integrate` is covered by tests that are skipped on
  Windows without symlink privilege; it has not been exercised on Unix yet.
- On Unix, play/record jobs can outlive an MCP server that is killed with
  SIGKILL.
- Mono Ogg Vorbis output fails in the encoder (pre-existing).
- `vol` takes dB (SoX `vol` is a linear factor) and `fade`'s second argument is
  the fade-out length (SoX: stop position); see `docs/SOX_COMPATIBILITY.md`.
- The repository does not yet contain licence text files.
