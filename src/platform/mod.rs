pub mod chromium_flags;
pub mod spotify;
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;

pub const DEFAULT_SPOTIFY_MEMORY_LIMIT_MB: usize = 150;
