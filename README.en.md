<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
    <img src="assets/logo.png" alt="mixly logo" width="160">
  </picture>
</p>

<h1 align="center">mixly</h1>

<p align="center">Lightweight <strong>CLI / TUI</strong> music player written in Rust.</p>

<p align="center">
  <img alt="language" src="https://img.shields.io/badge/rust-2021-orange?logo=rust">
  <img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
</p>

<p align="center"><a href="README.md">简体中文</a> | <strong>English</strong></p>

## Install (Windows x86_64)

```powershell
irm https://raw.githubusercontent.com/Zyl0812/mixly/main/install.ps1 | iex
```

Verify:

```powershell
mixly --help
```

## Features

- Netease Cloud Music + QQ Music + Bilibili video audio (ordinary UGC videos only; no downloads)
- Mixed local playlists (any platform in one list)
- High-quality streaming via external **mpv**
- Real proxy coverage for **API requests and audio streams** (local HTTP relay)
- Ships an [agent skill](#agent-skill) so Claude Code / Codex / Grok can drive it from the CLI
- Claude Code playback plugin: live song / progress / lyric in the bottom status line (`mixly status --claude`)

## Usage

```bash
# Search
mixly search "海阔天空" --platform all
mixly search "晴天" --platform qq --limit 5
mixly search "周杰伦" --platform bilibili     # requires QR login first

# Play one track
mixly play netease <song_id>
mixly play qq <song_id>
mixly play bilibili BV1xx411c7mD            # BV id
mixly play bilibili av123456                # AV id
mixly play bilibili https://www.bilibili.com/video/BV1xx/?p=2   # URL + part

# Login (QR — scan with phone app)
mixly login --platform netease
mixly login --platform qq
mixly login --platform bilibili

# Logout (removes this platform's local credentials only)
mixly logout --platform bilibili

# Playlists
mixly playlist create "My Mix"
mixly playlist list
mixly playlist add "My Mix" netease <id> --name "Song" --artist "Artist"
mixly playlist add "My Mix" qq <id>
mixly playlist add "My Mix" bilibili BV1xx411c7mD --name "Video audio"
mixly playlist show "My Mix"
mixly playlist play "My Mix"
mixly playlist remove "My Mix" 0
mixly playlist rename "My Mix" "Mixed"
mixly playlist delete "Mixed"

# TUI
mixly tui

# Force proxy
mixly --proxy socks5://127.0.0.1:7890 search "test"
```

### Playback status (Claude Code status line)

`mixly play` writes the current song, position, pause state and lyric to `%APPDATA%\mixly\now-playing.json` every 400ms.

```bash
mixly status --json     # machine-readable state (empty when nothing is playing)
mixly status --claude   # Claude Code status line (two lines wide, three lines narrow)
```

Install the Claude Code plugin (requires the `claude` CLI; registers a local marketplace, installs the plugin, and configures the status line):

```powershell
mixly skill install claude --global
```

If an Orca, Starship, or other `statusLine` already exists, the installer prepends `mixly status --claude`: Mixly playback appears at the top of the status block and the original output remains below it. The original object is backed up to `~/.claude/mixly-statusline-backup.json` and restored exactly on uninstall. If the composed config is later edited, uninstall protects it by default; `--force` strips only the Mixly prefix while preserving later edits.

### Bilibili notes and limits

- Plays **audio from ordinary UGC videos only** (DASH audio stream): no video track, no download, no transcode, no export.
- Login is always QR with your own account; credentials stay local and never hit logs.
- Unsupported: Bangumi/PGC, courses, paid content, live streams; no bypassing of VIP/region/DRM limits; Dolby/Hi-Res tracks; auto-renewal (re-scan QR when expired).
- Picks the highest ordinary audio bandwidth your account is allowed; no quality-tier mapping.
- Bilibili tracks are never fully prefetched — streaming with Range only.
- The web API integration is experimental and may break as the platform changes; failures only affect Bilibili, not Netease/QQ/local.

### TUI keys

| Key | Action |
|---|---|
| `j` / `k` | Move |
| `Enter` | Play selection |
| `Space` | Pause / resume |
| `n` / `p` | Next / previous |
| `/` | Search |
| `a` | Add search hit to playlist |
| `+` / `-` | Volume |
| `h` / `l` | Seek ±5s |
| `m` | Cycle play mode |
| `q` | Quit |

## Agent skill

mixly ships [`skills/mixly/SKILL.md`](skills/mixly/SKILL.md). Drop it into an AI coding agent
(Claude Code, Codex, Grok, …) and you can drive the player in plain language — the agent runs the same
`mixly` commands under the hood and never touches mixly's source.

The Skill is embedded in `mixly.exe`, so no repository clone or manual copy is needed. Install it for every project:

```powershell
mixly skill install claude --global
mixly skill install codex --global
mixly skill install grok --global
```

Or install it only in the current directory:

```powershell
mixly skill install claude --project
mixly skill install codex --project
mixly skill install grok --project
```

A Claude global install writes the embedded plugin to `~/.claude/mixly-marketplace/`, registers the `mixly-local` marketplace through `claude plugin`, installs `mixly@mixly-local`, and configures `mixly status --claude`. Existing status lines are composed rather than overwritten, and their commands still receive Claude's stdin. A one-second refresh is added only for the installed composition and the original value is restored on uninstall. While the Skill is active, only `mixly` CLI commands are pre-approved so playback does not prompt every time; user-level `deny` and `ask` rules still take precedence.

Inspect the target path or uninstall:

```powershell
mixly skill path codex --global
mixly skill uninstall codex --global
mixly skill uninstall claude --global
```

Install and uninstall refuse to replace a modified file; pass `--force` only when you intend to overwrite or remove it. Install only for the agents you use, and restart the agent if it does not detect the new Skill automatically.

Then just ask:

> "Play 晴天 by Jay Chou" → the agent runs `search`, picks a hit, backgrounds `play`, and tells you what's on
>
> "I don't know what to listen to—play me something" → the agent uses recent context and existing playlists to recommend one new song, verifies the official version, and plays it in the background
>
> "Make a playlist called Commute with these five songs" → `playlist create` + `search` / `add` per track, then `show` to confirm
>
> "Import the music folder on my D: drive" → `local import`

The skill spells out the guardrails, so the agent won't make a mess:

- CLI only — it never launches `mixly tui` (the interactive UI would block it)
- Playback is a long-running process, so it's always backgrounded — no waiting out a whole song in the foreground
- Never runs two `play` commands at once (they'd fight over mpv's IPC pipe)
- Login requires *you* to scan the QR code; the agent won't pretend it's signed in
- Never pastes token file contents into the conversation
- Has a recovery path for the usual failures: empty stream URL → tells you to `login` or switches platform; no search hits → suggests `--proxy` or a shorter query; mpv errors → tells you to install mpv

## Configuration

Config directory (via `directories` crate):

| OS | Path |
|---|---|
| Windows | `%APPDATA%\mixly\` |
| Linux | `~/.config/mixly/` |
| macOS | `~/Library/Application Support/mixly/` |

`config.toml` example:

```toml
[general]
quality = "Exhigh"          # Standard | Higher | Exhigh | Lossless
preferred_platform = "qq"   # qq | netease | bilibili

[proxy]
enabled = false
url = "socks5://127.0.0.1:7890"

[player]
mpv_path = "mpv"
```

Tokens are stored separately as `netease_token.json` / `qq_token.json` / `bilibili_token.json` (Bilibili stores only minimal Cookie fields). The playback snapshot lives in `now-playing.json` and contains no tokens or URLs.

### Proxy priority

```
--proxy  >  config.toml [proxy]  >  ALL_PROXY/HTTPS_PROXY/HTTP_PROXY  >  direct
```

Audio always goes through mixly's local relay (`127.0.0.1:<ephemeral>/current`) so SOCKS5 and HTTPS CDN work even when mpv/FFmpeg cannot use SOCKS.

The API clients and audio relay use the same resolved proxy. mixly carries a minimal local patch for the locked `netease-qq-music-api` 0.1.0 that removes its forced `.no_proxy()` calls.

## Architecture (short)

1. **API** — Netease/QQ via `netease-qq-music-api =0.1.0` (isolated in `src/api/client.rs`); Bilibili private protocol lives in `src/api/bilibili.rs` (WBI signing, QR login, DASH audio)
2. **Proxy for API** — inject `ALL_PROXY` before Tokio; the local dependency patch makes upstream API clients honor it
3. **Proxy for audio** — local HTTP relay streams CDN via reqwest (Range / Referer / refresh; per-track exact Referer for Bilibili)
4. **Playback** — mpv JSON IPC (`interprocess` local sockets / Windows named pipes)
5. **Queue** — owned by mixly (one `loadfile` per track; links expire)
6. **Status snapshot** — the play loop writes `now-playing.json`; `mixly status --claude` renders it offline as the Claude Code status line

## License

MIT (see `license` in [Cargo.toml](Cargo.toml)).
