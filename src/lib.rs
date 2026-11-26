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
pub mod cover_art;
pub mod log_file;
pub mod metadata;
pub mod mock;
pub mod platform;
pub mod soloist_ws;
pub mod tui;
pub mod visualizer;
