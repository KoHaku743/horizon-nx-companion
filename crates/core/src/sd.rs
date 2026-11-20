//! Finding the Switch's SD card among the computer's drives, and what is on it.

use std::path::{Path, PathBuf};

/// A drive that holds a Switch's SD card (an `atmosphere`, `Nintendo` or `switch` folder at its
/// root).
#[derive(Clone, Debug, PartialEq)]
pub struct SdCard {
    pub root: PathBuf,
    /// The drive's name ("SWITCH SD").
    pub name: String,
    /// Whether Horizon is on it (`switch/horizon/`).
    pub has_horizon: bool,
}

impl SdCard {
    pub fn at(root: &Path) -> Option<SdCard> {
        let looks_like = ["atmosphere", "Nintendo", "switch", "bootloader"]
            .iter()
            .any(|d| root.join(d).is_dir());
        if !looks_like {
            return None;
        }
        Some(SdCard {
            root: root.to_path_buf(),
            // A drive letter has no file name: "E:".
            name: root.file_name().map_or_else(
                || {
                    root.display()
                        .to_string()
                        .trim_end_matches(['\\', '/'])
                        .to_owned()
                },
                |n| n.to_string_lossy().into_owned(),
            ),
            has_horizon: root.join("switch/horizon").is_dir(),
        })
    }

    /// `switch/horizon/`, where the game keeps everything.
    pub fn horizon(&self) -> PathBuf {
        self.root.join("switch").join("horizon")
    }

    /// The game files the game reads (`switch/horizon/resources/`).
    pub fn resources(&self) -> PathBuf {
        self.horizon().join("resources")
    }

    /// The drive's free and total bytes.
    pub fn space(&self) -> Option<(u64, u64)> {
        Some((
            fs2::available_space(&self.root).ok()?,
            fs2::total_space(&self.root).ok()?,
        ))
    }
}

/// The drives that look like a Switch's SD card.
pub fn find() -> Vec<SdCard> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let list = |dir: &Path, roots: &mut Vec<PathBuf>| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            roots.extend(entries.flatten().map(|e| e.path()));
        }
    };
    if cfg!(target_os = "macos") {
        list(Path::new("/Volumes"), &mut roots);
    } else if cfg!(target_os = "windows") {
        for letter in b'D'..=b'Z' {
            roots.push(PathBuf::from(format!("{}:\\", letter as char)));
        }
    } else {
        let user = std::env::var("USER").unwrap_or_default();
        for base in ["/media", "/run/media"] {
            list(&Path::new(base).join(&user), &mut roots);
        }
        list(Path::new("/mnt"), &mut roots);
    }
    roots.iter().filter_map(|r| SdCard::at(r)).collect()
}

/// macOS writes `._name` files (and `.DS_Store`, `.Trashes`...) beside what it copies to a FAT
/// drive; the Switch's homebrew menu and the game then trip over them.
pub fn is_mac_junk(name: &str) -> bool {
    name.starts_with("._")
        || matches!(
            name,
            ".DS_Store" | ".Trashes" | ".fseventsd" | ".Spotlight-V100" | ".TemporaryItems"
        )
}

/// Deletes macOS's `._` and `.DS_Store` files under `dir` (as `dot_clean -m` does for
/// `switch/horizon`). Returns how many went.
pub fn remove_mac_junk(dir: &Path) -> usize {
    let mut removed = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if (name.starts_with("._") || name == ".DS_Store") && !is_dir {
                if std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            } else if is_dir {
                pending.push(path);
            }
        }
    }
    removed
}

/// Ejects the drive: macOS `diskutil eject`; Windows the Explorer's Eject; Linux `udisksctl`
/// (unmount, then power off).
pub fn eject(card: &SdCard) -> Result<(), String> {
    use std::process::Command;
    let run = |command: &mut Command| -> Result<String, String> {
        let out = command.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            Err(if err.is_empty() {
                "The system wouldn't eject it: close what uses it and try again.".into()
            } else {
                err
            })
        }
    };
    if cfg!(target_os = "macos") {
        run(Command::new("diskutil").arg("eject").arg(&card.root)).map(|_| ())
    } else if cfg!(target_os = "windows") {
        let drive = card
            .root
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_owned();
        if drive.len() != 2 || !drive.ends_with(':') {
            return Err("Eject the SD card from the Explorer.".into());
        }
        let script = format!(
            "(New-Object -ComObject Shell.Application).Namespace(17).ParseName('{drive}').InvokeVerb('Eject')"
        );
        run(Command::new("powershell").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]))?;
        // The verb returns before the drive is gone; it's ejected once the system lets it go.
        for _ in 0..20 {
            if !card.root.exists() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Err(
            "Windows didn't eject it yet: close what uses the card, or eject it from the Explorer."
                .into(),
        )
    } else {
        let device = run(Command::new("findmnt")
            .args(["-no", "SOURCE"])
            .arg(&card.root))
        .map_err(|_| "Eject the SD card from your file manager.".to_owned())?;
        run(Command::new("udisksctl").args(["unmount", "-b", &device]))?;
        // Powering off is a nicety (not every reader allows it): unmounted is safe to unplug.
        let _ = run(Command::new("udisksctl").args(["power-off", "-b", &device]));
        Ok(())
    }
}
