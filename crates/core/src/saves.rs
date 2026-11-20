//! Worlds: backing up the Switch's, and copying worlds between the Switch and Minecraft on this
//! computer (Horizon plays vanilla 26.3 worlds; an older world is upgraded when it's opened, a
//! newer one may not open).

use crate::sd::{SdCard, is_mac_junk};
use crate::{Progress, size_text};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    pub folder: PathBuf,
    /// The folder's name.
    pub name: String,
    pub bytes: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// The worlds in a `saves` folder, newest first.
pub fn worlds_in(saves: &Path) -> Vec<World> {
    let Ok(entries) = std::fs::read_dir(saves) else {
        return Vec::new();
    };
    let mut worlds: Vec<World> = entries
        .flatten()
        .filter(|e| e.path().join("level.dat").is_file())
        .map(|e| {
            let folder = e.path();
            World {
                name: e.file_name().to_string_lossy().into_owned(),
                bytes: folder_size(&folder),
                modified: std::fs::metadata(folder.join("level.dat"))
                    .and_then(|m| m.modified())
                    .ok(),
                folder,
            }
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.modified));
    worlds
}

/// The Switch's `saves` folder.
pub fn switch_saves(card: &SdCard) -> PathBuf {
    card.horizon().join("saves")
}

/// Minecraft's own `saves` folder on this computer (the official launcher's).
pub fn computer_saves() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        dirs::data_dir()?.join("minecraft")
    } else if cfg!(target_os = "windows") {
        dirs::data_dir()?.join(".minecraft")
    } else {
        dirs::home_dir()?.join(".minecraft")
    };
    Some(dir.join("saves"))
}

/// Where backups go: `Documents/Horizon Switch Backups`.
pub fn backup_dir() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("Horizon Switch Backups")
}

pub fn folder_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.metadata() {
                Ok(m) if m.is_dir() => pending.push(entry.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// Zips every world of the Switch into one file in [`backup_dir`].
pub fn backup(card: &SdCard, progress: &Progress) -> Result<String, String> {
    let worlds = worlds_in(&switch_saves(card));
    if worlds.is_empty() {
        return Err("There are no worlds on the SD card.".into());
    }
    let total: u64 = worlds.iter().map(|w| w.bytes).sum();
    let dir = backup_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = chrono_like_stamp();
    let path = dir.join(format!("Switch worlds {stamp}.zip"));
    progress.step("Backing up the worlds", total);
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let result = (|| -> Result<(), String> {
        for world in &worlds {
            add_folder(&mut zip, &world.folder, &world.name, options, progress)?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(format!(
        "{} worlds ({}) backed up to {}.",
        worlds.len(),
        size_text(total),
        path.display()
    ))
}

fn add_folder<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    options: zip::write::SimpleFileOptions,
    progress: &Progress,
) -> Result<(), String> {
    let mut pending = vec![(dir.to_path_buf(), prefix.to_owned())];
    while let Some((dir, name)) = pending.pop() {
        progress.check()?;
        zip.add_directory(format!("{name}/"), options)
            .map_err(|e| e.to_string())?;
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let child = format!("{name}/{}", entry.file_name().to_string_lossy());
            if is_mac_junk(&entry.file_name().to_string_lossy()) {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push((entry.path(), child));
            } else {
                let data = std::fs::read(entry.path()).map_err(|e| format!("{child}: {e}"))?;
                zip.start_file(&child, options).map_err(|e| e.to_string())?;
                zip.write_all(&data).map_err(|e| e.to_string())?;
                progress.advance(data.len() as u64);
            }
        }
    }
    Ok(())
}

/// Copies a world folder into a `saves` folder, under a name that isn't taken.
pub fn copy_world(world: &World, to_saves: &Path, progress: &Progress) -> Result<String, String> {
    std::fs::create_dir_all(to_saves).map_err(|e| format!("{}: {e}", to_saves.display()))?;
    let mut name = world.name.clone();
    let mut n = 2;
    while to_saves.join(&name).exists() {
        name = format!("{} ({n})", world.name);
        n += 1;
    }
    let target = to_saves.join(&name);
    progress.step(format!("Copying {}", world.name), world.bytes);
    let result = copy_tree(&world.folder, &target, progress);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&target);
        return Err(e);
    }
    Ok(format!("{} copied as \"{name}\".", world.name))
}

fn copy_tree(from: &Path, to: &Path, progress: &Progress) -> Result<(), String> {
    let mut pending = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((from, to)) = pending.pop() {
        progress.check()?;
        std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        for entry in std::fs::read_dir(&from)
            .map_err(|e| format!("{}: {e}", from.display()))?
            .flatten()
        {
            let name = entry.file_name();
            if is_mac_junk(&name.to_string_lossy()) || name == "session.lock" {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push((entry.path(), to.join(&name)));
            } else {
                let data = std::fs::read(entry.path())
                    .map_err(|e| format!("{}: {e}", entry.path().display()))?;
                std::fs::write(to.join(&name), &data)
                    .map_err(|e| format!("{}: {e}", to.join(&name).display()))?;
                progress.advance(data.len() as u64);
            }
        }
    }
    Ok(())
}

/// "2026-11-20 21-45" in local time, without a date library: the system's clock as UTC.
pub(crate) fn chrono_like_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}-{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}
