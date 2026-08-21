use std::path::PathBuf;

pub const LEAN_FLAGS: &[&str] = &[
  "--disable-backgrounding-occluded-windows",
  "--disable-background-timer-throttling",
  "--disable-renderer-backgrounding",
  "--disable-features=CalculateNativeWinOcclusion",
  "--disable-ipc-flooding-protection",
  "--disable-hang-monitor",
  "--disable-breakpad",
  "--disable-crash-reporter",
  "--disable-dev-shm-usage",
  "--disable-extensions",
  "--disable-component-extensions-with-background-pages",
  "--disable-background-networking",
  "--disable-sync",
  "--disable-domain-reliability",
  "--disable-client-side-phishing-detection",
  "--disable-component-update",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-default-apps",
  "--disable-popup-blocking",
  "--disable-prompt-on-repost",
  "--disable-blink-features=AutomationControlled",
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
