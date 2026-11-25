pub mod chromium_flags;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod media_watch;
pub mod notifications;
pub mod soloist;
pub mod spotify;
pub mod tray;
pub mod tray_ipc;
pub mod window;
#[cfg(windows)]
pub mod windows;

pub use crate::user_config::DEFAULT_SPOTIFY_MEMORY_LIMIT_MB;
