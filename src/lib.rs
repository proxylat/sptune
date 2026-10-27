pub mod event;
pub mod lcg;
pub mod library_cache;
pub mod playlist_cache;
pub mod user_config;

// Render-bench bindings: these modules are ALSO compiled into the bin target
// (main.rs keeps its own `mod` tree). No `allow` attributes here by policy:
// fix warnings at their source instead of suppressing them for benches.
// CI gates benches with `-D warnings` (see ci.yml).
pub mod app;
pub mod backend;
pub mod client_creds;
// NOTE: `cover_art` and `visualizer` are WIP modules whose third-party deps
// (image, rustfft, cpal, libpulse) are not in Cargo.toml yet. They stay out
// of the bench bindings until that branch lands with its manifest entries;
// re-add the `pub mod` lines together with the deps.
pub mod platform;
pub mod soloist_ws;
pub mod tui;
