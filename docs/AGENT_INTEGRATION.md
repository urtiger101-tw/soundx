# Agent integration and Windows audio control

soundx 0.2 and later include a stdio MCP server, an installable Agent Skill, and native
Windows Core Audio controls in the same executable. No Node/Python runtime or
external SoX binary is required. The existing codec and effect limits in
[SOX_COMPATIBILITY.md](SOX_COMPATIBILITY.md) still apply.

## Install for an agent

The Windows installer offers independent, initially unchecked options for
Codex, Claude Code, OpenCode and AGY / Antigravity. Each selected option installs
the `soundx` Skill and registers the installed executable as a local MCP server.
Restart the selected agent to discover the new integration.

Portable installations can use:

```powershell
soundx integrate list
soundx integrate install --agent codex
soundx integrate install --agent claude
soundx integrate install --agent opencode
soundx integrate install --agent agy
soundx integrate remove --agent codex
```

| Target | Skill location relative to user home | MCP configuration |
| --- | --- | --- |
| Codex | `.agents/skills/soundx/SKILL.md` | `.codex/config.toml`, `mcp_servers.soundx` |
| Claude Code | `.claude/skills/soundx/SKILL.md` | `.claude.json`, `mcpServers.soundx` |
| OpenCode | `.config/opencode/skills/soundx/SKILL.md` | Existing `opencode.jsonc`, otherwise `opencode.json`, in `.config/opencode`; `mcp.soundx` |
| AGY CLI / Antigravity | `.gemini/antigravity-cli/skills/soundx/SKILL.md` and `.gemini/config/skills/soundx/SKILL.md` | `.gemini/config/mcp_config.json`, `mcpServers.soundx` |

Codex's `.agents` directory is shared: other agents that discover that standard
location can also see the Skill. MCP registration remains specific to the
selected target. These are default user-profile locations; use the agent's
own configuration tools for custom `CODEX_HOME` / XDG setups. `--home PATH`
selects a separate profile root for portable installations or testing.

Installation preserves other configuration entries and JSONC/TOML comments.
A conflicting `soundx` entry or customized Skill is not overwritten. Existing
configurations are backed up under `.soundx/backups`. Integration ownership is
recorded under `.soundx/integrations`; do not publish those private profile files.
Removal deletes only unchanged entries and files installed by soundx. Modified
items are reported and preserved with a nonzero exit status. `remove --all`
attempts every target even when one has an invalid configuration. OpenCode
removal also handles a managed config renamed from `.json` to `.jsonc`.
The Windows uninstaller removes registrations
only when they still refer to that installation's executable.

## MCP tools

Launch `soundx mcp` as the client's stdio command. Messages are one UTF-8 JSON-RPC
object per line. Logging goes to stderr. No TCP port is opened. Supported
protocol versions: 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05.

| Tool | Purpose |
| --- | --- |
| `soundx_run` | Finite CLI commands: info, loudness, formats, devices, convert, concat, mix, synth, stream, batch, run-plan and help |
| `soundx_loudness` | Read-only ITU-R BS.1770-4 / EBU R128 loudness, loudness range and true-peak measurement (`inputs`: 1-64 paths); returns a `loudness` array |
| `soundx_start` | Start playback/recording, returning `job_id` and `process_id` |
| `soundx_jobs` | Inspect running/completed jobs and captured results |
| `soundx_stop` | Gracefully stop a job belonging to this MCP connection |
| `soundx_windows_devices` | Enumerate native Windows audio endpoints |
| `soundx_windows_endpoint` | Read or set endpoint volume/mute |
| `soundx_windows_sessions` | Enumerate output audio sessions |
| `soundx_windows_session` | Read or set one exact session's volume/mute |

Example tool arguments for `soundx_loudness`:

```json
{"inputs":["C:/audio/input.wav"]}
```

and for `soundx_run`:

```json
{"arguments":["convert","C:/audio/input.wav","C:/audio/output.flac"]}
```

Arguments are passed directly to this executable, never a shell. Use absolute
paths, since relative paths resolve against the MCP client's working directory.
`soundx_run` returns `state`, `exit_code`, `stdout`, `stderr` and parsed
`output_json` when available. A command failure sets `isError: true`.
Timeouts range from 1 to 600 seconds (default 120). stdout/stderr capture is
limited to 256 KiB each with an explicit truncation flag; requests are limited
to 1 MiB. At most 32 job records are retained; completed records can be evicted
when starting more jobs. These tools have the launching user's file permissions.

Looping playback and continuous recording require `soundx_start`, followed by
`soundx_jobs` and `soundx_stop`. Normal stdin closure or a transport error stops
managed jobs. After 10 seconds without graceful exit, stop forcibly terminates
the process and reports that recording finalization is uncertain. Abruptly
killing the MCP process cannot guarantee recording finalization. On Windows,
each child is assigned to a Job Object before its start is acknowledged, so
Windows terminates it if the MCP process is forcibly killed. Stopping a
finite recording early does not write an output file.

Resources `soundx://guide` and `soundx://compatibility` expose the Skill and
compatibility matrix directly through MCP.

## Windows Core Audio API

```powershell
soundx windows devices
soundx windows endpoint
soundx windows endpoint --flow input
soundx windows endpoint --volume 35
soundx windows endpoint --mute true
soundx windows sessions
soundx windows session "EXACT_SESSION_ID" --volume 50 --mute false
```

All native commands return JSON. Volume is a percentage from 0 to 100. Omit
volume and mute to query without changes. `--device` accepts an exact endpoint
id from `windows devices`; it differs from the CPAL device name used by
`play --device` and `record --device`. The default is the multimedia endpoint.
Session operations require an output endpoint and select a stable instance id,
not a list index; sessions can disappear when an application exits.

These controls use documented `IMMDeviceEnumerator`, `IAudioEndpointVolume`,
`IAudioSessionManager2` and `ISimpleAudioVolume` interfaces. They do not change
the system's default device through undocumented interfaces. Non-Windows
platforms return an explicit unsupported error for native control commands.

## Reference documentation

- [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [Claude Code Skills](https://code.claude.com/docs/en/skills)
- [OpenCode Skills](https://opencode.ai/docs/skills/) and [MCP](https://opencode.ai/docs/mcp-servers/)
- [AGY Skills](https://antigravity.google/docs/skills) and [MCP](https://antigravity.google/docs/mcp)
- [Windows EndpointVolume API](https://learn.microsoft.com/en-us/windows/win32/coreaudio/endpointvolume-api)
