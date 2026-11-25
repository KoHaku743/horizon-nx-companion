//! Worlds: backing up the Switch's, and copying worlds between the Switch and Horizon on this
//! computer (Horizon plays vanilla 26.3 worlds; an older world is upgraded when it's opened, a
//! newer one may not open).

use crate::sd::{SdCard, is_mac_junk};
use crate::sync::Direction;
use crate::{Progress, names, size_text};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    pub folder: PathBuf,
    /// The folder's name, as the player reads it (see [`names`]).
    pub name: String,
    pub bytes: u64,
    /// When it was last played (level.dat's own record, else the file's time).
    pub modified: Option<std::time::SystemTime>,
    /// The name in the game's list.
    pub title: String,
    /// "Survival · 26.3".
    pub kind: String,
    /// `icon.png`'s bytes.
    pub icon: Option<Vec<u8>>,
}

/// What `level.dat` says of a world.
struct Level {
    title: Option<String>,
    played: Option<std::time::SystemTime>,
    kind: String,
}

fn level(folder: &Path) -> Option<Level> {
    use crate::nbt::{self, Tag};
    use std::io::Read;
    let file = std::fs::File::open(folder.join("level.dat")).ok()?;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(file)
        .take(64 << 20)
        .read_to_end(&mut bytes)
        .ok()?;
    let (_, root) = nbt::read(&bytes).ok()?;
    let data = root.get("Data")?;
    let on = |tag: Option<&Tag>| matches!(tag, Some(Tag::Byte(1)));
    let hardcore = on(data.get("hardcore"))
        || on(data
            .get("difficulty_settings")
            .and_then(|d| d.get("hardcore")));
    let mut kind = Vec::new();
    match data.get("GameType") {
        _ if hardcore => kind.push("Hardcore"),
        Some(Tag::Int(0)) => kind.push("Survival"),
        Some(Tag::Int(1)) => kind.push("Creative"),
        Some(Tag::Int(2)) => kind.push("Adventure"),
        Some(Tag::Int(3)) => kind.push("Spectator"),
        _ => {}
    }
    kind.extend(
        data.get("Version")
            .and_then(|v| v.get("Name"))
            .and_then(Tag::as_str),
    );
    Some(Level {
        title: data
            .get("LevelName")
            .and_then(Tag::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned),
        played: match data.get("LastPlayed") {
            Some(Tag::Long(ms)) if *ms > 0 => {
                std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_millis(*ms as u64))
            }
            _ => None,
        },
        kind: kind.join(" · "),
    })
}

fn icon(folder: &Path) -> Option<Vec<u8>> {
    let path = folder.join("icon.png");
    let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= 1 << 20);
    small.then(|| std::fs::read(path).ok())?
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
            let name = names::shown(&e.file_name().to_string_lossy());
            let level = level(&folder);
            let file_time = || {
                std::fs::metadata(folder.join("level.dat"))
                    .and_then(|m| m.modified())
                    .ok()
            };
            World {
                bytes: folder_size(&folder),
                modified: level.as_ref().and_then(|l| l.played).or_else(file_time),
                title: level
                    .as_ref()
                    .and_then(|l| l.title.clone())
                    .unwrap_or_else(|| name.clone()),
                kind: level.map(|l| l.kind).unwrap_or_default(),
                icon: icon(&folder),
                name,
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

/// Horizon's own `saves` folder on this computer: the game's home, as the game finds it
/// (`HORIZON_HOME`, else `horizon` in the system's data folder: macOS
/// `~/Library/Application Support/horizon`, Windows `%APPDATA%\horizon`, Linux
/// `~/.local/share/horizon`).
pub fn computer_saves() -> Option<PathBuf> {
    Some(computer_home()?.join("saves"))
}

/// Horizon's home folder on this computer (see [`computer_saves`]).
pub fn computer_home() -> Option<PathBuf> {
    match std::env::var_os("HORIZON_HOME").filter(|h| !h.is_empty()) {
        Some(home) => Some(PathBuf::from(home)),
        None => Some(dirs::data_dir()?.join("horizon")),
    }
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

/// A backup in [`backup_dir`].
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: Option<std::time::SystemTime>,
    /// The worlds in it.
    pub worlds: Vec<String>,
}

/// The backups on this computer, newest first.
pub fn backups() -> Vec<Backup> {
    backups_in(&backup_dir())
}

fn backups_in(dir: &Path) -> Vec<Backup> {
    let mut found: Vec<Backup> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "zip"))
        .filter_map(|e| {
            let file = std::fs::File::open(e.path()).ok()?;
            let zip = zip::ZipArchive::new(file).ok()?;
            let mut worlds: Vec<String> = zip
                .file_names()
                .filter_map(|n| n.strip_suffix("/level.dat"))
                .filter(|n| !n.is_empty() && !n.contains('/'))
                .map(str::to_owned)
                .collect();
            worlds.sort();
            let meta = e.metadata().ok()?;
            (!worlds.is_empty()).then(|| Backup {
                path: e.path(),
                bytes: meta.len(),
                modified: meta.modified().ok(),
                worlds,
            })
        })
        .collect();
    found.sort_by_key(|b| std::cmp::Reverse(b.modified));
    found
}

/// Zips `worlds` into one file in [`backup_dir`].
pub fn backup(worlds: &[World], progress: &Progress) -> Result<String, String> {
    backup_to(&backup_dir(), worlds, progress).map(|path| {
        let total: u64 = worlds.iter().map(|w| w.bytes).sum();
        let what = match worlds {
            [one] => one.title.clone(),
            many => format!("{} worlds", many.len()),
        };
        format!(
            "{what} ({}) backed up to {}.",
            size_text(total),
            path.display()
        )
    })
}

fn backup_to(dir: &Path, worlds: &[World], progress: &Progress) -> Result<PathBuf, String> {
    let label = match worlds {
        [] => return Err("There are no worlds to back up.".into()),
        [one] => one.name.as_str(),
        _ => "Switch worlds",
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Never over an earlier backup: two in a minute have the same stamp.
    let stamp = chrono_like_stamp();
    let mut path = dir.join(format!("{label} {stamp}.zip"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{label} {stamp} ({n}).zip"));
        n += 1;
    }
    progress.step("Backing up", worlds.iter().map(|w| w.bytes).sum());
    // Under another name until it's whole, so a half one is never listed.
    let part = path.with_extension("zip.part");
    let file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let result = (|| -> Result<(), String> {
        for world in worlds {
            add_folder(&mut zip, &world.folder, &world.name, options, progress)?;
        }
        let mut out = zip.finish().map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    Ok(path)
}

/// Puts `world` of `backup` back into a `saves` folder, as a new world (one with the same name
/// stays).
pub fn restore(
    backup: &Path,
    world: &str,
    to_saves: &Path,
    direction: Direction,
    progress: &Progress,
) -> Result<String, String> {
    let file = std::fs::File::open(backup).map_err(|e| format!("{}: {e}", backup.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut total = 0;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // Nothing outside the world's folder, whatever the zip says.
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if let Ok(rel) = path.strip_prefix(world)
            && entry.is_file()
        {
            let parts: Vec<String> = rel
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(n) => Some(direction.name(&n.to_string_lossy())),
                    _ => None,
                })
                .collect();
            if !parts.is_empty() && parts.last().is_some_and(|n| n != "session.lock") {
                total += entry.size();
                files.push((i, parts));
            }
        }
    }
    if !files.iter().any(|(_, p)| *p == ["level.dat"]) {
        return Err(format!("{world} isn't in {}.", backup.display()));
    }
    std::fs::create_dir_all(to_saves).map_err(|e| format!("{}: {e}", to_saves.display()))?;
    if let Ok(free) = fs2::available_space(to_saves)
        && free < total
    {
        return Err(format!("Not enough room: {} needed.", size_text(total)));
    }
    let (name, target) = free_world_name(world, to_saves, direction);
    progress.step(format!("Restoring {world}"), total);
    let result = (|| -> Result<(), String> {
        for (i, parts) in &files {
            progress.check()?;
            let to = parts.iter().fold(target.clone(), |d, p| d.join(p));
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            let mut entry = zip.by_index(*i).map_err(|e| e.to_string())?;
            let mut out =
                std::fs::File::create(&to).map_err(|e| format!("{}: {e}", to.display()))?;
            let n = std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("{}: {e}", to.display()))?;
            progress.advance(n);
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&target);
        return Err(e);
    }
    Ok(format!("{world} restored as \"{name}\"."))
}

/// A world folder's name that isn't taken in `to_saves` ("World (2)"), and its path.
fn free_world_name(world: &str, to_saves: &Path, direction: Direction) -> (String, PathBuf) {
    let mut name = world.to_owned();
    let mut n = 2;
    while to_saves.join(direction.name(&name)).exists() {
        name = format!("{world} ({n})");
        n += 1;
    }
    let target = to_saves.join(direction.name(&name));
    (name, target)
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
            let child = format!(
                "{name}/{}",
                names::shown(&entry.file_name().to_string_lossy())
            );
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
pub fn copy_world(
    world: &World,
    to_saves: &Path,
    direction: Direction,
    progress: &Progress,
) -> Result<String, String> {
    std::fs::create_dir_all(to_saves).map_err(|e| format!("{}: {e}", to_saves.display()))?;
    let (name, target) = free_world_name(&world.name, to_saves, direction);
    progress.step(format!("Copying {}", world.name), world.bytes);
    let result = copy_tree(&world.folder, &target, direction, progress);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&target);
        return Err(e);
    }
    Ok(format!("{} copied as \"{name}\".", world.name))
}

pub(crate) fn copy_tree(
    from: &Path,
    to: &Path,
    direction: Direction,
    progress: &Progress,
) -> Result<(), String> {
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
            let target = to.join(direction.name(&name.to_string_lossy()));
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push((entry.path(), target));
            } else {
                let data = std::fs::read(entry.path())
                    .map_err(|e| format!("{}: {e}", entry.path().display()))?;
                std::fs::write(&target, &data).map_err(|e| format!("{}: {e}", target.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::{self, Tag};

    #[test]
    fn a_backup_comes_back_beside_the_world_it_was_made_of() {
        let root = std::env::temp_dir().join(format!("companion-saves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (computer, card, kept) = (root.join("c"), root.join("s"), root.join("b"));
        let world = computer.join("Мир");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("region/r.0.0.mca"), b"chunks").unwrap();
        std::fs::write(world.join("session.lock"), b"x").unwrap();
        let data = Tag::Compound(vec![(
            "Data".into(),
            Tag::Compound(vec![
                ("LevelName".into(), Tag::String("My World".into())),
                ("GameType".into(), Tag::Int(1)),
                ("LastPlayed".into(), Tag::Long(1_000_000)),
            ]),
        )]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&nbt::write("", &data)).unwrap();
        std::fs::write(world.join("level.dat"), gz.finish().unwrap()).unwrap();

        let worlds = worlds_in(&computer);
        let w = &worlds[0];
        assert_eq!(
            (w.name.as_str(), w.title.as_str(), w.kind.as_str()),
            ("Мир", "My World", "Creative")
        );
        let played = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        assert_eq!(w.modified, Some(played));

        // Twice in a minute: two backups.
        let first = backup_to(&kept, &worlds, &Progress::new()).unwrap();
        let second = backup_to(&kept, &worlds, &Progress::new()).unwrap();
        assert_ne!(first, second);
        let found = backups_in(&kept);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].worlds, ["Мир"]);

        for folder in ["xn--o0a0dya".to_owned(), names::on_card("Мир (2)")] {
            restore(&first, "Мир", &card, Direction::ToSwitch, &Progress::new()).unwrap();
            let back = card.join(folder);
            assert_eq!(
                std::fs::read(back.join("region/r.0.0.mca")).unwrap(),
                b"chunks"
            );
            assert!(!back.join("session.lock").exists());
        }
        assert!(
            restore(
                &first,
                "Other",
                &card,
                Direction::ToSwitch,
                &Progress::new()
            )
            .is_err()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_zip_that_points_outside_writes_nothing_there() {
        let root = std::env::temp_dir().join(format!("companion-slip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("x.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for name in ["W/level.dat", "W/../../../evil.txt"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        let saves = root.join("a/b/saves");
        restore(&path, "W", &saves, Direction::ToComputer, &Progress::new()).unwrap();
        assert!(saves.join("W/level.dat").is_file());
        for dir in ["", "a", "a/b", "a/b/saves"] {
            assert!(!root.join(dir).join("evil.txt").exists());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
