//! Desktop track-change notifications (`org.freedesktop.Notifications` via
//! `zbus` on Linux; no-op stub on other targets).

#[cfg(target_os = "linux")]
mod imp {
  use std::collections::HashMap;
  use std::sync::atomic::{AtomicU32, Ordering};
  use zbus::OwnedVariant;

  static LAST_ID: AtomicU32 = AtomicU32::new(0);

  pub async fn notify_track(summary: String, body: String, icon_path: Option<String>) {
    let Ok(conn) = zbus::Connection::session().await else {
      return;
    };
    let icon = icon_path
      .filter(|p| std::path::Path::new(p).exists())
      .unwrap_or_else(|| "audio-x-generic".to_string());
    let replaces_id = LAST_ID.load(Ordering::Relaxed);
    let actions: Vec<&str> = Vec::new();
    let hints: HashMap<&str, OwnedVariant> = HashMap::new();
    let Ok(reply) = conn
      .call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
          "sptune",
          replaces_id,
          icon.as_str(),
          summary.as_str(),
          body.as_str(),
          actions,
          hints,
          3500i32,
        ),
      )
      .await
    else {
      return;
    };
    if let Ok(id) = reply.body().deserialize::<u32>() {
      LAST_ID.store(id, Ordering::Relaxed);
    }
  }
}

#[cfg(not(target_os = "linux"))]
mod imp {
  pub async fn notify_track(_summary: String, _body: String, _icon_path: Option<String>) {}
}

/// Fire-and-forget OS notification on the active Tokio runtime.
pub fn spawn_track_notification(summary: String, body: String, icon_path: Option<String>) {
  tokio::spawn(async move {
    imp::notify_track(summary, body, icon_path).await;
  });
}
