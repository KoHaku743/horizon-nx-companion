//! Freeing space on the SD card: what the game can make again, listed with its size so the
//! player picks what goes. The game files, the worlds, the account and the settings are never in
//! the list.

use crate::sd::SdCard;
use crate::size_text;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, PartialEq)]
pub struct Cleanable {
    pub label: String,
    /// What happens when it goes.
    pub note: String,
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    /// Ticked by default: nothing is lost (old logs, leftovers, junk files).
    pub suggested: bool,
}

fn size(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => crate::saves::folder_size(path),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn days_ago(time: Option<SystemTime>) -> Option<u64> {
    SystemTime::now()
        .duration_since(time?)
        .ok()
        .map(|d| d.as_secs() / 86_400)
}

/// What could go, biggest first.
pub fn scan(card: &SdCard) -> Vec<Cleanable> {
    let dir = card.horizon();
    let mut out = Vec::new();
    let mut push = |label: String, note: &str, paths: Vec<PathBuf>, suggested: bool| {
        let bytes = paths.iter().map(|p| size(p)).sum();
        if !paths.is_empty() && bytes > 0 {
            out.push(Cleanable {
                label,
                note: note.into(),
                paths,
                bytes,
                suggested,
            });
        }
    };

    let listing = |d: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(d)
            .map(|e| e.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };

    push(
        "Old logs".into(),
        "Logs of earlier runs (logs/*.log.gz). The last run's log stays.",
        listing(&dir.join("logs"))
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "gz"))
            .collect(),
        true,
    );

    // Diagnostics a test run turned on (env.txt's BEVY_HORIZON_CPU_PROFILE): rewritten each run.
    push(
        "CPU profile".into(),
        "cpu-profile.txt, the profiler's last report. Written again only when profiling is on.",
        vec![dir.join("cpu-profile.txt")]
            .into_iter()
            .filter(|p| p.is_file())
            .collect(),
        true,
    );

    let mut leftovers: Vec<PathBuf> = listing(&dir.join("mod-browser/checked"))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".part"))
        })
        .collect();
    leftovers.extend(
        listing(&dir.join("mod-browser/downloads"))
            .into_iter()
            .filter(|p| p.is_file()),
    );
    leftovers.push(dir.join("horizon.nro.new"));
    leftovers.retain(|p| p.exists());
    push(
        "Unfinished downloads".into(),
        "Downloads that were stopped half way.",
        leftovers,
        true,
    );

    push(
        "Checked mod downloads".into(),
        "Get Mods' copies of mods it checked; it downloads them again when needed.",
        listing(&dir.join("mod-browser/checked"))
            .into_iter()
            .filter(|p| {
                !p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".part"))
            })
            .collect(),
        false,
    );

    // Server and resource pack bundles, one entry per pack, oldest use first.
    let packs = dir.join(".horizon-cache/resourcepacks");
    let mut bundles: Vec<(String, Vec<PathBuf>, Option<SystemTime>)> = Vec::new();
    for path in listing(&packs) {
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let used = [".hzpak", ".hzpak.changed", ".overlay-cache.json"]
            .iter()
            .filter_map(|s| {
                std::fs::metadata(packs.join(format!("{name}{s}")))
                    .and_then(|m| m.modified())
                    .ok()
            })
            .chain(std::fs::metadata(&path).and_then(|m| m.modified()).ok())
            .max();
        let mut paths = vec![path.clone()];
        for suffix in [
            ".hzpak",
            ".hzpak.tmp",
            ".hzpak.changed",
            ".overlay-cache.json",
            ".overlay-cache.lock",
        ] {
            let p = packs.join(format!("{name}{suffix}"));
            if p.exists() {
                paths.push(p);
            }
        }
        bundles.push((name, paths, used));
    }
    bundles.sort_by_key(|a| a.2);
    for (name, paths, used) in bundles {
        let age = days_ago(used);
        let label = match age {
            Some(d) if d >= 1 => format!("Resource pack bundle {name} (used {d} days ago)"),
            _ => format!("Resource pack bundle {name} (used today)"),
        };
        push(
            label,
            "A server's or your resource pack, prepared for the game; the next time it's used it's built again (a slow join).",
            paths,
            age.is_some_and(|d| d >= 30),
        );
    }

    let mut contents: Vec<(PathBuf, Option<SystemTime>)> = listing(&dir.join(".horizon-cache"))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("mod-content-"))
        })
        .map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, t)
        })
        .collect();
    contents.sort_by_key(|c| std::cmp::Reverse(c.1));
    if contents.len() > 1 {
        push(
            "Older mod data".into(),
            "Mod data built for earlier mod or pack choices; the newest stays.",
            contents.into_iter().skip(1).map(|(p, _)| p).collect(),
            false,
        );
    }

    let mut junk = Vec::new();
    collect_junk(&dir, &mut junk);
    push(
        "macOS metadata files".into(),
        "The ._ files macOS leaves on the card; the Switch doesn't need them.",
        junk,
        true,
    );

    out.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    out
}

fn collect_junk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            collect_junk(&entry.path(), out);
        } else if name.starts_with("._") || name == ".DS_Store" {
            out.push(entry.path());
        }
    }
}

/// Deletes the chosen items; returns what was freed.
pub fn clean(items: &[Cleanable]) -> Result<String, String> {
    let mut freed = 0;
    let mut failed = Vec::new();
    for item in items {
        for path in &item.paths {
            let gone = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            if let Err(e) = gone
                && path.exists()
            {
                failed.push(format!("{}: {e}", path.display()));
            }
        }
        freed += item.bytes;
    }
    if failed.is_empty() {
        Ok(format!("Freed {}.", size_text(freed)))
    } else {
        Err(format!(
            "Some files couldn't be removed: {}",
            failed.join("; ")
        ))
    }
}

/// How long until a bundle counts as old.
pub const OLD: Duration = Duration::from_secs(30 * 86_400);
