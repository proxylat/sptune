//! Interactive Soloist setup: API key + device name + binary auto-install.
//!
//! Runs on fresh installs (Linux only) and on demand via `--setup-soloist`
//! (all platforms — key/device config works for a remote Soloist on the LAN;
//! only the binary download is Linux-gated). Writes client.yml + config.yml
//! and updates the in-memory configs so startup launches Soloist directly.

use crate::client_creds::ClientConfig;
use crate::platform::soloist::{binary_found, DOWNLOADS_URL};
use crate::user_config::UserConfig;
use anyhow::{Context, Result};
use std::io::{stdin, Write};
use std::path::PathBuf;

pub const API_KEY_DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard/soloist";

fn suffix_for_arch(arch: &str) -> Option<&'static str> {
  match arch {
    "x86_64" => Some("x86_64"),
    "aarch64" => Some("arm64"),
    "arm" => Some("arm32"),
    _ => None,
  }
}

fn prompt_line(label: &str) -> Result<String> {
  print!("{label}");
  std::io::stdout().flush()?;
  let mut s = String::new();
  stdin().read_line(&mut s)?;
  Ok(s.trim().to_string())
}

fn prompt_yes(label: &str) -> Result<bool> {
  Ok(matches!(
    prompt_line(label)?.to_lowercase().as_str(),
    "y" | "yes"
  ))
}

/// `~/.local/bin` when it is on PATH, else `~/.config/sptune/bin` (with a
/// `needs_binary_path` flag so the caller auto-fills `soloist.binary_path`).
fn install_dir() -> (PathBuf, bool) {
  if let Some(home) = dirs::home_dir() {
    let local_bin = home.join(".local").join("bin");
    let on_path =
      std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == local_bin));
    if on_path {
      return (local_bin, false);
    }
    return (home.join(".config").join("sptune").join("bin"), true);
  }
  (PathBuf::from("."), true)
}

/// Download the Soloist tarball from Spotify's CDN and extract the single
/// `soloist` binary into the install dir. Returns the binary path.
async fn auto_install() -> Result<Option<PathBuf>> {
  let Some(suffix) = suffix_for_arch(std::env::consts::ARCH) else {
    println!(
      "No auto-install for this CPU ({}). Download it manually from {}",
      std::env::consts::ARCH,
      DOWNLOADS_URL
    );
    return Ok(None);
  };
  if std::process::Command::new("tar")
    .arg("--version")
    .output()
    .is_err()
  {
    println!(
      "No `tar` found. Download it manually from {}",
      DOWNLOADS_URL
    );
    return Ok(None);
  }
  let (dir, _) = install_dir();
  std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
  let url = format!(
    "https://soloist-builds.spotifycdn.com/soloist_release_{}.tar.gz",
    suffix
  );
  println!("Downloading {} ...", url);
  let bytes = reqwest::get(&url)
    .await
    .with_context(|| format!("download failed: {}", url))?
    .error_for_status()?
    .bytes()
    .await?;
  let tmp = std::env::temp_dir().join(format!("sptune_soloist_{}.tar.gz", std::process::id()));
  std::fs::write(&tmp, &bytes)?;
  let status = std::process::Command::new("tar")
    .args(["-xzf", &tmp.to_string_lossy(), "-C", &dir.to_string_lossy()])
    .status();
  let _ = std::fs::remove_file(&tmp);
  status?;
  let bin = dir.join("soloist");
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
  }
  let ok = std::process::Command::new(&bin)
    .arg("--version")
    .output()
    .is_ok_and(|o| o.status.success());
  if ok {
    Ok(Some(bin))
  } else {
    println!("Install verification (`soloist --version`) failed — try the manual download.");
    Ok(None)
  }
}

pub async fn run_setup(
  client_config: &mut ClientConfig,
  user_config: &mut UserConfig,
) -> Result<()> {
  println!("\nSoloist is Spotify's official headless client for Linux — a real Spotify Connect device with local audio output (Premium required).");
  if !prompt_yes("Enable headless playback with Soloist? [y/N]: ")? {
    return Ok(());
  }

  println!(
    "\nGenerate your own per-user API key at {}\n(never shipped with sptune).",
    API_KEY_DASHBOARD_URL
  );
  let key = loop {
    let k = prompt_line("Enter your Soloist API key: ")?;
    if !k.is_empty() {
      break k;
    }
    println!("Key can't be empty.");
  };

  let name = prompt_line("Connect device name [sptune]: ")?;
  let device_name = if name.is_empty() {
    "sptune".to_string()
  } else {
    name
  };

  // Binary: manual download page first, auto-install on consent.
  let mut binary_path = user_config.soloist.binary_path.clone();
  if !binary_found(&user_config.soloist) {
    println!(
      "\nDownload the Soloist binary (per-user, do not redistribute) from {}\nBuilds expire after 90 days — re-download when it stops starting.",
      DOWNLOADS_URL
    );
    #[cfg(target_os = "linux")]
    if prompt_yes("Download and install it automatically now? [y/N]: ")? {
      match auto_install().await {
        Ok(Some(bin)) => {
          println!("Installed to {}", bin.display());
          let (_, needs_path) = install_dir();
          binary_path = if needs_path {
            bin.to_string_lossy().into_owned()
          } else {
            String::new()
          };
        }
        Ok(None) => {}
        Err(e) => println!("Auto-install failed ({:#}); install it manually.", e),
      }
    }
    #[cfg(not(target_os = "linux"))]
    println!("(Auto-install is Linux-only; on this machine point it at a remote Soloist or install manually on your Linux box.)");
  }

  client_config.save_soloist_api_key(&key)?;
  user_config.save_soloist_config(true, &device_name, &binary_path)?;

  println!(
    "\nSetup saved. Pair once: open the official Spotify app on the same network, find \"{}\" under Devices, and connect — after that it persists.",
    device_name
  );
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::suffix_for_arch;

  #[test]
  fn arch_suffixes() {
    assert_eq!(suffix_for_arch("x86_64"), Some("x86_64"));
    assert_eq!(suffix_for_arch("aarch64"), Some("arm64"));
    assert_eq!(suffix_for_arch("arm"), Some("arm32"));
    assert_eq!(suffix_for_arch("riscv64"), None);
  }
}
