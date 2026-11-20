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
            name: root.file_name().map_or_else(
                || root.display().to_string(),
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

/// Ejects the drive (macOS: `diskutil eject`; elsewhere the system's own way is asked of the
/// player).
pub fn eject(card: &SdCard) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("diskutil")
            .arg("eject")
            .arg(&card.root)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    } else {
        Err("Eject the SD card from your system's file manager.".into())
    }
}
