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

- Netease Cloud Music + QQ Music
- Mixed local playlists (both platforms in one list)
- High-quality streaming via external **mpv**
- Real proxy coverage for **API requests and audio streams** (local HTTP relay)
- Ships an [agent skill](#agent-skill) so Claude Code / Codex / Grok can drive it from the CLI

## Requirements

- Rust 1.75+ (edition 2021)
- [mpv](https://mpv.io/) on `PATH` (or set `player.mpv_path` in config)

## Build

```bash
cargo build --release
# binary: target/release/mixly
```

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
default_platform = "all"

[proxy]
enabled = false
url = "socks5://127.0.0.1:7890"

[player]
mpv_path = "mpv"
```

Tokens are stored separately as `netease_token.json` / `qq_token.json`.

### Proxy priority

```
--proxy  >  config.toml [proxy]  >  ALL_PROXY/HTTPS_PROXY/HTTP_PROXY  >  direct
```

Audio always goes through mixly’s local relay (`127.0.0.1:<ephemeral>/current`) so SOCKS5 and HTTPS CDN work even when mpv/FFmpeg cannot use SOCKS.

**Note:** `netease-qq-music-api` 0.1.0 builds its internal `reqwest::Client` with `.no_proxy()`, so `ALL_PROXY` does **not** affect upstream API calls until that crate changes. The audio relay (our client) still honors proxy settings — the critical path for campus SOCKS + HTTPS CDN.

## Usage

```bash
# Search
mixly search "海阔天空" --platform all
mixly search "晴天" --platform qq --limit 5

# Play one track
mixly play netease <song_id>
mixly play qq <song_id>

# Login (QR — scan with phone app)
mixly login --platform netease
mixly login --platform qq

# Playlists
mixly playlist create "My Mix"
mixly playlist list
mixly playlist add "My Mix" netease <id> --name "Song" --artist "Artist"
mixly playlist add "My Mix" qq <id>
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

## Tests

```bash
cargo test                 # unit + relay Range tests
cargo test -- --ignored    # live API smoke (needs network; QQ URL may need login)
```

## Architecture (short)

1. **API** — `netease-qq-music-api =0.1.0`, isolated in `src/api/client.rs`
2. **Proxy for API** — inject `ALL_PROXY` before Tokio; Cargo unifies `reqwest` with `socks`
3. **Proxy for audio** — local HTTP relay streams CDN via reqwest (Range / Referer / refresh)
4. **Playback** — mpv JSON IPC (`interprocess` local sockets / Windows named pipes)
5. **Queue** — owned by mixly (one `loadfile` per track; links expire)

## Non-goals

- Desktop floating lyrics / GUI
- Traffic obfuscation
- Default song download

## Agent skill

[`skills/mixly/SKILL.md`](skills/mixly/SKILL.md) lets an AI coding agent (Claude Code, Codex, Grok, etc.) drive the built binary via shell commands — search, play, and manage playlists — without touching mixly's source.

Copy or symlink it into your tool's skill directory, e.g.:

```bash
# Claude Code (project-level)
mkdir -p .claude/skills && cp -r skills/mixly .claude/skills/

# Grok CLI
mkdir -p .grok/skills && cp -r skills/mixly .grok/skills/
```

## License

MIT (see `license` in [Cargo.toml](Cargo.toml)).
