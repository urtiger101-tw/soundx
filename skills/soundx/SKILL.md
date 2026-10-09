---
name: soundx
description: Process, inspect, convert, synthesize, play or record local audio with soundx; control Windows audio endpoint and application volume through Core Audio. Use for audio operations and soundx MCP tools.
---

# soundx audio tools

Use the installed `soundx` executable or its MCP tools. Run `soundx --version` and
`soundx formats` when the installed capabilities matter. This is a pure Rust,
SoX-style tool with a documented subset of SoX compatibility.

## Processing

- Inspect inputs with `soundx info INPUT --json` before choosing sample rate,
  channels, format or normalization.
- Measure loudness with `soundx loudness INPUT... --json` (ITU-R BS.1770-4 / EBU R128:
  `integrated_lufs`, `loudness_range_lu`, `true_peak_dbtp`, `sample_peak_dbfs`,
  momentary/short-term maxima). `integrated_lufs` is `null` for silence. WAV files are
  streamed with bounded memory. `soundx loudness --help` succeeding means the
  installed build supports it (absent before 0.3).
- Normalise loudness with `soundx convert IN OUT --loudness-target=-18 --true-peak=-1.8
  --stat-json` (mutually exclusive with `--normalize`; write negative values as
  `--opt=-18`). It measures, applies linear gain, and uses a lookahead true-peak limiter
  only when needed (never hard clipping; same frame count, no added delay). The JSON adds
  `loudness.{input,output,gain_db,limited,limiter_gain_reduction_max_db}`. `--true-peak`
  alone only limits peaks. For very long WAV files use `soundx stream IN.wav OUT.wav
  --loudness-target=... --true-peak=...` (bounded memory; keeps the input bit depth unless
  `--bits`/`--float` is given). Verify with `soundx loudness OUT --json`.
- Use `soundx convert INPUT OUTPUT --help`, `soundx synth --help`, or
  `soundx run-plan --help` for the exact installed options.
- For ordered effects including dither, compand, reverb, tempo and stretch, use
  a JSON processing plan and `soundx run-plan PLAN.json`; the plan has `mode`
  (`convert`, `concat`, `mix`), `inputs`, `output`, `effects` (an array of separate
  effect tokens), `normalize`, and `stat_json`.
- Use separate arguments and absolute paths. Preserve the original input by
  choosing a different output path. Only overwrite existing output when that
  matches the user's request.
- Inspect the produced file with `info --json`; a launched command is not proof
  of a successful conversion.

## MCP

Start the stdio server with `soundx mcp`; stdout is exclusively MCP JSON-RPC.
The installer can register it for Codex, Claude Code, OpenCode and AGY.

- `soundx_run`: finite CLI operations, with an `arguments` array and optional
  `timeout_seconds` (1–600, default 120). Example:
  `{"arguments":["convert","C:/audio/in.wav","C:/audio/out.flac"]}`.
  Read `state`, `exit_code`, `stderr` and `output_json`; `failed` or `timed_out`
  means the requested operation did not complete successfully.
- `soundx_loudness`: read-only BS.1770-4 / EBU R128 measurement. Arguments:
  `{"inputs":["C:/audio/a.wav"],"timeout_seconds":120}`; the result's `loudness` array has one
  object per input (same fields as `soundx loudness --json`). `soundx_run` also accepts
  `loudness` and `convert ... --loudness-target=-18 --true-peak=-1.8`.
- `soundx_start`: start `play` or `record`; retain the returned `job_id`.
- `soundx_jobs`: inspect job results. `started`/`running` does not mean completed.
- `soundx_stop`: stop by that MCP session's job id. Continuous WAV recording
  finalizes on graceful stop. A forced stop warns that finalization is uncertain.
  Keep the MCP connection open while a job is needed; closing it stops its jobs.
- `soundx_windows_devices`, `soundx_windows_endpoint`,
  `soundx_windows_sessions`, `soundx_windows_session`: Windows Core Audio tools.
  Read first. Set `volume` (0–100) or `mute` only when requested; return the actual
  readback. Use exact endpoint/session ids returned by enumeration. Omitting
  volume and mute is read-only. Individual sessions can disappear as apps close.

Read `soundx://compatibility` through MCP for the detailed codec/effect limits.

## Device operations and limits

`soundx devices` lists CPAL devices for `play`/`record`. Windows endpoint ids and
session ids belong to the `soundx windows` commands, not CPAL's device-name flag.
Use `soundx windows --help` for native controls; these controls require Windows.

Playback and recording need an available audio device. Recording additionally
needs microphone access. Perform microphone capture only when the user's task
calls for it; never infer recording permission from a request to inspect devices.
For continuous capture, use `record OUTPUT.wav --continuous`, then stop the job
to finalize the file. Finite recording stopped early does not save an output.

WavPack is v5 lossless mono/stereo. Multi-file playback preloads the playlist in
memory. Continuous recording is 16-bit PCM WAV, stops before the RIFF size limit
(about 4 GiB), and does not support RF64. Do not promise full SoX equivalence or
claim physical playback/capture from file-only tests.
