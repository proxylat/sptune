use std::path::PathBuf;

pub const LEAN_FLAGS: &[&str] = &[
  "--headless=new",
  "--force-prefers-reduced-motion",
  "--intensive-wake-up-throttling-policy=0",
  "--renderer-process-limit=1",
  "--js-flags=--max-old-space-size=512 --optimize-for-size",
  "--disable-v8-idle-tasks",
  "--disable-background-networking",
  "--disable-domain-reliability",
  "--disable-component-update",
  "--no-pings",
  "--disable-breakpad",
  "--disable-crash-reporter",
  "--disable-in-process-stack-traces",
  "--disable-stack-profiler",
  "--disable-extensions",
  "--disable-sync",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-renderer-accessibility",
  "--disable-speech-api",
  "--disable-speech-synthesis-api",
  "--disable-notifications",
  "--disable-site-isolation-trials",
  "--disable-logging",
  "--disable-threaded-animation",
  "--disable-threaded-compositing",
  "--disable-remote-fonts",
  "--disable-remote-playback-api",
  "--disable-webgl2",
  "--num-raster-threads=1",
  "--disable-video-capture-use-gpu-memory-buffer",
  "--disable-threaded-scrolling",
  "--disable-component-extensions-with-background-pages",
  "--disable-accelerated-video-decode",
  "--disable-accelerated-mjpeg-decode",
  "--disable-3d-apis",
  "--disable-2d-canvas-clip-aa",
  "--disable-gpu",
  "--disable-gpu-watchdog",
  "--disable-gpu-compositing",
  "--disable-gpu-memory-buffer-compositor-resources",
  "--disable-gpu-memory-buffer-video-frames",
  "--disable-accelerated-video-encode",
  "--disable-accelerated-2d-canvas",
  "--disable-accelerated-jpeg-decoding",
  "--blink-settings=imagesEnabled=false",
  "--disable-webgl",
  "--autoplay-policy=no-user-gesture-required",
  "--touch-events=disabled",
  "--disable-pinch",
  "--disable-default-apps",
  "--disable-translate",
  "--disable-spell-checking",
  "--disable-spell-check-service",
  "--disable-hang-monitor",
  "--disable-prompt-on-repost",
  "--disable-client-side-phishing-detection",
  "--disable-popup-blocking",
  "--disable-gpu-rasterization",
  "--disable-gpu-program-cache",
  "--disable-gpu-shader-disk-cache",
  "--disable-software-rasterizer",
  "--disable-skia-graphite",
  "--disable-lcd-text",
  "--disable-smooth-scrolling",
  "--disable-canvas-aa",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  "--disable-infobars",
  "--disk-cache-size=0",
  "--media-cache-size=0",
  "--enable-features=AudioWorkletRealtimeThread,NetworkServiceInProcess,RestrictThreadPoolInBackground,ThrottleUnimportantFrameTimers",
  "--disable-features=AudioServiceOutOfProcess,AudioServiceSandbox,AutofillAssistant,BackForwardCache,BackgroundFetch,CalculateNativeWinOcclusion,CompositeAfterPaint,DialMediaRouteProvider,GetDisplayMedia,HardwareMediaKeyHandling,HeavyAdIntervention,InterestFeedContentSuggestions,LayoutNG,MediaSession,MediaStreamTrackTransfer,NetworkServiceSandbox,OptimizationGuideModelDownloading,OptimizationHints,PaintHolding,PictureInPicture,PointerLock,RTCEncodedVideoFrames,SharedStorage,SpareRendererForSitePerProcess,StorageBuckets,StorageServiceOutOfProcess,TabHoverCards,TabHoverCardImages,Translate,TranslateUI,VideoCapture,WebBluetooth,WebHID,WebNFC,WebOTP,WebRtcRemoteEventLog,WebShare,WebUSB",
];

pub fn preset_lean() -> Vec<&'static str> {
  LEAN_FLAGS.to_vec()
}

pub fn chromium_flags_path() -> PathBuf {
  dirs::home_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join(".config")
    .join("sptune")
    .join("chromium_flags.yml")
}

#[derive(serde::Deserialize, Debug)]
struct YmlFlags {
  flags: Vec<String>,
}

pub fn load_flags() -> Vec<String> {
  let p = chromium_flags_path();
  if let Ok(s) = std::fs::read_to_string(&p) {
    if let Ok(v) = serde_yml::from_str::<YmlFlags>(&s) {
      let filtered: Vec<String> = v
        .flags
        .into_iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
      if !filtered.is_empty() {
        return filtered;
      }
    }
  }
  preset_lean().into_iter().map(|s| s.to_string()).collect()
}

pub fn flags_to_arg_string(flags: &[String]) -> String {
  flags.join(" ")
}

pub fn is_network_service(cmd: &str) -> bool {
  cmd.contains("--utility-sub-type=network.mojom.NetworkService")
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn flags_join_no_empty() {
    let flags: Vec<String> = preset_lean().into_iter().map(|s| s.to_string()).collect();
    for f in &flags {
      assert!(!f.trim().is_empty(), "empty flag");
      assert!(f.starts_with("--"), "flag must start with --: {f}");
    }
    let joined = flags_to_arg_string(&flags);
    assert!(!joined.contains("  "));
    assert!(joined.starts_with("--"));
    // single flag
    assert_eq!(flags_to_arg_string(&["--foo".to_string()]), "--foo");
    // no empty join
    assert_eq!(flags_to_arg_string(&[]), "");
  }
  #[test]
  fn network_filter() {
    assert!(is_network_service("foo --utility-sub-type=network.mojom.NetworkService bar"));
    assert!(!is_network_service("foo --utility-sub-type=audio.mojom.AudioService bar"));
  }
}
