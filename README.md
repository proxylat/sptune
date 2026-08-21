# sptune

```text
 ___ _ __ | |_ _   _ _ __   ___
/ __| '_ \| __| | | | '_ \ / _ \
\__ \ |_) | |_| |_| | | | |  __/
|___/ .__/ \__|\__,_|_| |_|\___|
    |_|
```

A Spotify client for those who really love the terminal and music.
Performance and privacy first — everything runs locally.
No bloat, no noise — just the essentials.

## Preview

<!-- add image -->

## Features

- Everything is mouse clickable: single-click a row to open or play it...
- Fully keyboard-driven with remappable keybindings (`?` shows them all in-app)
- Search, browse and play your library, playlists, albums, artists and podcasts
- CLI for playback control, playlists and search (`sptune --help`)
- User-configurable theme, behavior and keybindings (${HOME}/.config/sptune/config.yml)

## Compatibility

- Made only for spotify at the moment...
- No mac development at the moment.

> [!NOTE]
> This app uses the official Spotify Web API, which doesn't handle streaming
> itself — you need an official Spotify client or a thirdparty client like [spotifyd](https://github.com/Spotifyd/spotifyd)
> open to play music. Playing tracks requires a Spotify Premium account.

## Connecting to Spotify

Initial setup is one time and interactive: run `sptune` and it will walk you
through it. If you want to set it up manually:

1. Go to the [Spotify dashboard](https://developer.spotify.com/dashboard/applications)
   and click `Create an app`
1. Note your `Client ID` and `Client Secret`
1. Click `Edit Settings` → add `http://127.0.0.1:8888/callback` to Redirect URIs → `Save`
1. Run `sptune` and enter your `Client ID`, `Client Secret` and port (default 8888)
1. Accept the permissions on the Spotify page — you'll be redirected back and
   authenticated automatically

## Using

Running `sptune` with no arguments brings up the UI. Press `?` for the help
menu showing all key events and their actions. There is also a CLI that does
most of what the UI does:

```
sptune play --name "Your Playlist" --playlist --random # Plays a random song from "Your Playlist"
sptune playback --like --shuffle # Likes the current song and toggles shuffle mode
sptune list --liked --limit 50 # See your liked songs (50 is the max limit)
sptune search "An even cooler song" --tracks --format "%t from %b" --limit 30
```

## Configuration

A configuration file is located at `${HOME}/.config/sptune/config.yml`
(not to be confused with client.yml which handles Spotify authentication).
The ⚙️ gear in the header opens a settings menu with quick toggles: black
theme, library/playlists blocks, volume bar subcells, `enable_mouse` and the
theme preset (Spotify / Dracula / Custom). Mouse and theme choices persist
across restarts (stored in `${HOME}/.config/sptune/state.json`).

## Local Installation

Install [Rust](https://www.rust-lang.org/tools/install) and

```bash
cargo install sptune  # after `cargo publish` — for now use `cargo install --path .`
```

On Linux, the development packages for `libssl` and `pkg-config` are required
for compilation.

## Development

1. [Install Rust](https://www.rust-lang.org/tools/install)
1. `cargo run` to develop, `cargo test` to run the test suite

## Credits

This repo is based on [spotify-tui](https://github.com/Rigellute/spotify-tui), but heavily modified for sptune — mouse support, a
draggable playbar scrubber, synced lyrics and more. 
Special thanks:
[ratatui](https://github.com/ratatui/ratatui), [rspotify](https://github.com/ramsayleung/rspotify),
[crossterm](https://github.com/crossterm-rs/crossterm) and [tokio](https://github.com/tokio-rs/tokio).

## References

https://developer.spotify.com/documentation/web-api
https://developer.spotify.com/documentation/web-api/concepts/rate-limits
https://developer.spotify.com/documentation/web-api/concepts/playlists
https://developer.spotify.com/documentation/web-api/concepts/api-calls
https://developer.spotify.com/documentation/web-api/tutorials/building-with-ai

## License

All code unique to sptune is licensed under [MIT](LICENSE).
