<div align="center">

```text
 ___ _ __ | |_ _   _ _ __   ___ 
/ __| '_ \| __| | | | '_ \ / _ \
\__ \ |_) | |_| |_| | | | |  __/
|___| .__/ \__|\__,_|_| |_|\___|
    |_|                         
```

**A Spotify client for people who live in the terminal.**

Powered by Spotify — music data from the Spotify Web API. Spotify is a trademark of Spotify AB.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/built_with-Rust-orange.svg)](https://www.rust-lang.org)

</div>

---

## Why sptune

- **Official Spotify Web API only** — unlike some third party clients, sptune uses only official Spotify Web API methods and never relies on librespot or other unofficial libraries to ensure account safety and eliminate ban risks.
- **Spotify desktop app can be aggressively optimized** — sptune has optional features that apply system-level tweaks and custom startup flags to squeeze every ounce of performance from the official Spotify client: reclaiming memory, suspending unnecessary child processes, and disabling costly background features. Even as an Electron app, it uses significantly less RAM and CPU than stock Spotify, keeping the player more lightweight while you game or run demanding work.
- **Request-optimized by design** — sptune makes requests only when their results are needed, delays non-essential work, reuses cached data and paces background updates. Less API traffic means less unnecessary activity while you are listening, working or gaming.
- **A customizable "For You" surface** — curate the home around the playlists and mixes you actually use, including custom playlist entries you add yourself.
- **Search that understands Spotify links** — paste a Spotify playlist link into search and press <kbd>Enter</kbd> to open it directly, no need to hunt through the library.
- **A real terminal UI** — fully clickable with the mouse, fully keyboard-driven with vim-style keys (remappable), plus synced lyrics and a draggable playbar scrubber.
- **CLI included** — drive playback, search and playlists from scripts, keybindings or your window manager without opening the UI.

Requires a **Spotify Premium** account and an active Spotify player — the official desktop client, a third-party client such as [spotifyd](https://github.com/Spotifyd/spotifyd), the web player or Spotify on another device.

## Soloist (headless playback, Linux)

Soloist is Spotify's official headless client for Linux (Raspberry Pi etc.) — a real Spotify Connect device with local audio output. When enabled, sptune spawns `soloist --device-name <name> --api-key <key>` at startup, transfers playback to it by device name, and kills only its own child on exit. Requires Premium.

Generate your own per-user API key at https://developer.spotify.com/dashboard/soloist (sptune never ships one). Download the binary (do not redistribute) from https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates — builds expire after 90 days.

In `~/.config/sptune/config.yml`:

```yaml
soloist:
  enabled: true
  device_name: "sptune"  # Connect device name to transfer playback to
  binary_path: ""        # optional; otherwise `soloist` is looked up on PATH
```

Key resolution: `SOLOIST_API_KEY` env var, else `soloist_api_key:` in `~/.config/sptune/client.yml`. First run only: open the official Spotify **desktop or mobile app** on the same local network, open Devices, and select `sptune` once — after that it stays paired and playback transfer works automatically. The web player cannot pair it (browsers can't discover devices on your LAN). Enabled but no key, or binary missing, prints a hint and continues without soloist.

First run on Linux walks you through setup (API key, device name, binary download) automatically; existing installs can re-run it anytime with `sptune --setup-soloist`.

How it works: sptune launches soloist with its own data dir (`~/.config/sptune/soloist`, so it never collides with a manually-run Soloist) and the local WebSocket API enabled. Playback state (track, position, play/pause, volume, shuffle/repeat) is pushed over that socket, replacing the 5s Web API poll while Soloist is the active device. Control commands (play/pause/seek/volume) still go through the Spotify Web API; the socket is read-only. If you start playback on another device, or Soloist exits, sptune falls back to API polling automatically.

## Docs

- [Installation & auth](docs/index.html#install) · [Features](docs/features.html) · [Keyboard shortcuts](docs/features.html#keyboard) · [CLI](docs/features.html#cli) · [Configuration](docs/features.html#configuration)

## Credits

Based on [spotify-tui](https://github.com/Rigellute/spotify-tui), rebuilt for sptune — mouse support, draggable playbar scrubber, synced lyrics and more.

Special thanks to [ratatui](https://github.com/ratatui/ratatui), [rspotify](https://github.com/ramsayleung/rspotify), [crossterm](https://github.com/crossterm-rs/crossterm) and [tokio](https://github.com/tokio-rs/tokio).

## License

All code unique to sptune is licensed under [MIT](LICENSE).
