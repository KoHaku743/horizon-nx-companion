//! The game files: finding the folder the Horizon launcher prepared, copying it to the SD card,
//! and packing it into `resources.hzpak` on the way, so the Switch doesn't spend its first start
//! packing them (about 20 minutes).

use crate::hzpak::{self, Entry, Kind, Top};
use crate::sd::{SdCard, is_mac_junk};
use crate::{Progress, size_text};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// The file the launcher's prepare writes at the top of the prepared folder.
pub const MANIFEST: &str = "horizon-resources.json";

/// A prepared folder found on this computer.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub folder: PathBuf,
    /// When it was prepared (its manifest's modification time).
    pub modified: Option<std::time::SystemTime>,
}

/// Prepared folders under the places the Horizon launcher keeps its data, newest first.
pub fn find_prepared() -> Vec<Prepared> {
    let mut found = Vec::new();
    // The Horizon launcher's: `<data>/horizon-launcher/versions/<tag>/.local/horizon-client/<game>`
    // (a hidden `.local` folder), and the servers' copy beside it.
    if let Some(data) = dirs::data_dir() {
        let launcher = data.join("horizon-launcher");
        for version in subfolders(&launcher.join("versions")) {
            for game in subfolders(&version.join(".local/horizon-client")) {
                search(&game, 0, &mut found);
            }
        }
        for game in subfolders(&launcher.join("server-runtime/game-files/horizon-client")) {
            search(&game, 0, &mut found);
        }
    }
    let mut roots = Vec::new();
    if let Some(data) = dirs::data_dir() {
        roots.push(data.clone());
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/share"));
        roots.push(home.join("Downloads"));
        roots.push(home.join("Desktop"));
    }
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains("horizon") && entry.path().is_dir() {
                search(&entry.path(), 6, &mut found);
            }
        }
    }
    found.sort_by_key(|p: &Prepared| std::cmp::Reverse(p.modified));
    let mut seen = std::collections::HashSet::new();
    found.retain(|p| seen.insert(p.folder.clone()));
    found
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

fn search(dir: &Path, depth: u32, found: &mut Vec<Prepared>) {
    let manifest = dir.join(MANIFEST);
    if let Ok(meta) = std::fs::metadata(&manifest) {
        found.push(Prepared {
            folder: dir.to_path_buf(),
            modified: meta.modified().ok(),
        });
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Not into the big folders that can't hold it.
            if !name.starts_with('.')
                && !matches!(
                    name.as_str(),
                    "assets" | "data" | "saves" | "versions" | "libraries"
                )
            {
                search(&entry.path(), depth - 1, found);
            }
        }
    }
}

/// Whether `folder` is a prepared folder.
pub fn is_prepared(folder: &Path) -> bool {
    folder.join(MANIFEST).is_file()
}

/// Whether today's game starts with the game files in a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Age {
    Current,
    /// Prepared by a Horizon from before 2.0: the game loads for minutes, then closes
    /// ("fluid_state_definitions Missing: re-run horizon-prepare").
    TooOld,
    /// Of another Minecraft version.
    OtherMinecraft,
    /// Files are missing or don't read.
    Damaged,
}

/// Checks `folder` by what the game itself refuses: the manifest's data version, then the fluid
/// table 2.0 needs (the manifest's `schema` didn't change when it came).
pub fn age(folder: &Path) -> Age {
    let read = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(folder.join(name)).ok()?).ok()
    };
    let Some(manifest) = read(MANIFEST) else {
        return Age::Damaged;
    };
    if manifest["data_version"].as_i64() != Some(i64::from(horizon_core::version::DATA_VERSION)) {
        return Age::OtherMinecraft;
    }
    match read("block-behaviours.json") {
        Some(blocks) if blocks.get("fluid_state_definitions").is_some() => Age::Current,
        Some(_) => Age::TooOld,
        None => Age::Damaged,
    }
}

/// One file or folder of the prepared folder, by its path under it (with `/`).
struct Item {
    rel: String,
    dir: bool,
    size: u64,
    mtime: i64,
}

/// Everything under `folder` that goes to the card, folders before what's in them.
fn walk(folder: &Path, progress: &Progress) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        progress.check()?;
        let path = if dir.is_empty() {
            folder.to_path_buf()
        } else {
            folder.join(&dir)
        };
        let mut names: Vec<String> = std::fs::read_dir(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            if is_mac_junk(&name) || name.contains('\n') {
                continue;
            }
            let rel = if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            let meta =
                std::fs::symlink_metadata(folder.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "{rel} is a link: the SD card can't hold links. Prepare the files again, or copy the real folder."
                ));
            }
            items.push(Item {
                rel: rel.clone(),
                dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                mtime: hzpak::mtime(&meta),
            });
            if meta.is_dir() {
                pending.push(rel);
            }
        }
    }
    Ok(items)
}

/// What [`copy_and_pack`] would do, for the window to show before it starts.
#[derive(Clone, Debug)]
pub struct Plan {
    pub files: u64,
    pub bytes: u64,
    /// Bytes of files already on the card as they are (not copied again).
    pub unchanged_bytes: u64,
    pub pack_bytes: u64,
    /// Free space on the card, and what the job needs of it.
    pub free: Option<u64>,
    pub needed: u64,
}

fn same_on_card(item: &Item, dest: &Path) -> bool {
    std::fs::metadata(dest).is_ok_and(|m| {
        m.is_file() && m.len() == item.size && (hzpak::mtime(&m) - item.mtime).abs() <= 2
    })
}

/// Measures the job.
pub fn plan(source: &Path, card: &SdCard, progress: &Progress) -> Result<Plan, String> {
    let items = walk(source, progress)?;
    let dest = card.resources();
    let (mut files, mut bytes, mut unchanged, mut pack) = (0, 0, 0, 0);
    for item in items.iter().filter(|i| !i.dir) {
        files += 1;
        bytes += item.size;
        if same_on_card(item, &dest.join(&item.rel)) {
            unchanged += item.size;
        }
        if item.size <= hzpak::MAX_PACKED {
            pack += item.size;
        }
    }
    let old_pack = std::fs::metadata(hzpak::pack_path(&dest)).map_or(0, |m| m.len());
    Ok(Plan {
        files,
        bytes,
        unchanged_bytes: unchanged,
        pack_bytes: pack,
        free: card.space().map(|(free, _)| free + old_pack),
        needed: (bytes - unchanged) + pack,
    })
}

/// Copies the prepared folder `source` to the card's `switch/horizon/resources/` (the card's
/// folder ends up exactly the prepared one: files not in it are removed) and writes
/// `resources.hzpak` beside it, as the game would build it. Files already on the card as they are
/// aren't copied again.
pub fn copy_and_pack(source: &Path, card: &SdCard, progress: &Progress) -> Result<String, String> {
    if !is_prepared(source) {
        return Err(format!(
            "{} isn't a prepared folder (no {MANIFEST}).",
            source.display()
        ));
    }
    progress.step("Reading the game files", 0);
    let items = walk(source, progress)?;
    let dest = card.resources();
    let pack = hzpak::pack_path(&dest);
    let total: u64 = items.iter().map(|i| i.size).sum();
    if let Some((free, _)) = card.space() {
        let old_pack = std::fs::metadata(&pack).map_or(0, |m| m.len());
        let packed: u64 = items
            .iter()
            .filter(|i| !i.dir && i.size <= hzpak::MAX_PACKED)
            .map(|i| i.size)
            .sum();
        let copying: u64 = items
            .iter()
            .filter(|i| !i.dir && !same_on_card(i, &dest.join(&i.rel)))
            .map(|i| i.size)
            .sum();
        if copying + packed > free + old_pack {
            return Err(format!(
                "The SD card needs {} free and has {}.",
                size_text(copying + packed),
                size_text(free + old_pack)
            ));
        }
    }
    // The old pack goes first: half-copied files must never be read from it.
    let _ = std::fs::remove_file(&pack);
    let _ = std::fs::remove_file(hzpak::changes_path(&dest));
    std::fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;

    progress.step("Copying the game files and packing them", total);
    let temporary = pack.with_extension("hzpak.tmp");
    let out =
        std::fs::File::create(&temporary).map_err(|e| format!("{}: {e}", temporary.display()))?;
    let root_mtime = std::fs::metadata(source).map_or(0, |m| hzpak::mtime(&m));
    let mut writer = hzpak::Writer::begin(BufWriter::with_capacity(4 << 20, out), root_mtime)
        .map_err(|e| e.to_string())?;
    let mut copied = 0u64;
    let mut data = Vec::new();
    let result = (|| -> Result<(), String> {
        for item in &items {
            progress.check()?;
            let to = dest.join(&item.rel);
            if item.dir {
                std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
                writer.entry(
                    &item.rel,
                    Entry {
                        kind: Kind::Dir,
                        size: 0,
                        mtime: item.mtime,
                    },
                );
                continue;
            }
            let unchanged = same_on_card(item, &to);
            let packed = item.size <= hzpak::MAX_PACKED;
            if unchanged && !packed {
                writer.entry(
                    &item.rel,
                    Entry {
                        kind: Kind::Loose,
                        size: item.size,
                        mtime: item.mtime,
                    },
                );
                progress.advance(item.size);
                continue;
            }
            data.clear();
            std::fs::File::open(source.join(&item.rel))
                .and_then(|mut f| f.read_to_end(&mut data))
                .map_err(|e| format!("{}: {e}", item.rel))?;
            if data.len() as u64 != item.size {
                return Err(format!(
                    "{} changed while copying: prepare the files again.",
                    item.rel
                ));
            }
            if !unchanged {
                // A plain copy: macOS's own copy would add `._` files on the card.
                let mut file =
                    std::fs::File::create(&to).map_err(|e| format!("{}: {e}", to.display()))?;
                file.write_all(&data)
                    .map_err(|e| format!("{}: {e}", to.display()))?;
                let when = std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(item.mtime.max(0) as u64);
                let _ = file.set_modified(when);
                copied += item.size;
            }
            if packed {
                writer
                    .file(&item.rel, &data, item.mtime)
                    .map_err(|e| format!("the pack: {e}"))?;
            } else {
                writer.entry(
                    &item.rel,
                    Entry {
                        kind: Kind::Loose,
                        size: item.size,
                        mtime: item.mtime,
                    },
                );
            }
            progress.advance(item.size);
        }
        Ok(())
    })();
    if let Err(e) = result {
        drop(writer);
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }

    // The card's folder is the prepared one: what isn't in it goes.
    progress.step("Removing old files", 0);
    let keep: std::collections::HashSet<&str> = items.iter().map(|i| i.rel.as_str()).collect();
    let removed = remove_others(&dest, &keep)?;

    let top: Vec<Top> = items
        .iter()
        .filter(|i| !i.rel.contains('/'))
        .map(|i| Top {
            name: i.rel.clone(),
            dir: i.dir,
            size: i.size,
        })
        .collect();
    progress.step("Finishing the pack", 0);
    let out = writer.finish(&top).map_err(|e| format!("the pack: {e}"))?;
    let file = out
        .into_inner()
        .map_err(|e| format!("the pack: {}", e.error()))?;
    file.sync_all().map_err(|e| format!("the pack: {e}"))?;
    drop(file);
    std::fs::rename(&temporary, &pack).map_err(|e| format!("the pack: {e}"))?;

    // As the game will see it: the card's top level must be the one packed.
    let now = hzpak::top_level(&dest).map_err(|e| e.to_string())?;
    let index = hzpak::read_index(&pack).map_err(|e| e.to_string())?;
    if !hzpak::fits(&index.top, &now) {
        let _ = std::fs::remove_file(&pack);
        return Err("The card's folder doesn't match what was packed (something else changed it): try again.".into());
    }
    crate::sd::remove_mac_junk(&card.horizon());
    let pack_size = std::fs::metadata(&pack).map_or(0, |m| m.len());
    Ok(format!(
        "{} files ready: {} copied ({} were already there), {} old files removed, resources.hzpak {}.",
        items.iter().filter(|i| !i.dir).count(),
        size_text(copied),
        size_text(total - copied),
        removed,
        size_text(pack_size)
    ))
}

/// Deletes what is under `dest` and not in `keep`; returns how many went.
fn remove_others(dest: &Path, keep: &std::collections::HashSet<&str>) -> Result<usize, String> {
    let mut removed = 0;
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        let path = if dir.is_empty() {
            dest.to_path_buf()
        } else {
            dest.join(&dir)
        };
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if keep.contains(rel.as_str()) {
                if is_dir {
                    pending.push(rel);
                }
                continue;
            }
            let gone = if is_dir {
                std::fs::remove_dir_all(entry.path())
            } else {
                std::fs::remove_file(entry.path())
            };
            gone.map_err(|e| format!("{}: {e}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("companion-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_card_gets_the_files_and_a_pack_the_game_takes() {
        let base = scratch("pack");
        let source = base.join("prepared");
        std::fs::create_dir_all(source.join("assets/minecraft/textures")).unwrap();
        std::fs::write(source.join(MANIFEST), b"{}").unwrap();
        std::fs::write(source.join("assets/minecraft/textures/stone.png"), b"png!").unwrap();
        std::fs::write(source.join("assets/._stone.png"), b"mac junk").unwrap();
        std::fs::write(
            source.join("big.bin"),
            vec![7u8; (hzpak::MAX_PACKED + 1) as usize],
        )
        .unwrap();
        let card_root = base.join("card");
        std::fs::create_dir_all(card_root.join("switch/horizon/resources/stale")).unwrap();
        std::fs::write(
            card_root.join("switch/horizon/resources/stale/old.txt"),
            b"old",
        )
        .unwrap();
        let card = SdCard::at(&card_root).unwrap();

        let summary = copy_and_pack(&source, &card, &Progress::new()).unwrap();
        assert!(summary.contains("files ready"), "{summary}");
        let dest = card.resources();
        assert_eq!(
            std::fs::read(dest.join("assets/minecraft/textures/stone.png")).unwrap(),
            b"png!"
        );
        assert!(!dest.join("assets/._stone.png").exists());
        assert!(!dest.join("stale").exists());
        let index = hzpak::read_index(&hzpak::pack_path(&dest)).unwrap();
        assert!(hzpak::fits(&index.top, &hzpak::top_level(&dest).unwrap()));
        let find = |rel: &str| {
            index
                .entries
                .iter()
                .find(|(r, _)| r == rel)
                .map(|(_, e)| *e)
        };
        let stone = find("assets/minecraft/textures/stone.png").unwrap();
        let Kind::Packed(at) = stone.kind else {
            panic!("packed")
        };
        let bytes = std::fs::read(hzpak::pack_path(&dest)).unwrap();
        assert_eq!(&bytes[at as usize..at as usize + 4], b"png!");
        assert_eq!(find("big.bin").unwrap().kind, Kind::Loose);
        assert_eq!(find("assets").unwrap().kind, Kind::Dir);

        // Again: nothing to copy, the same pack.
        let again = copy_and_pack(&source, &card, &Progress::new()).unwrap();
        assert!(again.contains("0 B copied"), "{again}");
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// What the card holds of the game files.
#[derive(Clone, Debug, PartialEq)]
pub enum OnCard {
    /// No `switch/horizon/resources/` with a manifest.
    Missing,
    /// The files, with no pack: the game builds one when it starts (slow).
    Unpacked,
    /// The files and a pack that matches them.
    Packed,
    /// The pack doesn't match the files any more: the game builds it again.
    Stale,
}

pub fn on_card(card: &SdCard) -> OnCard {
    let dest = card.resources();
    if !dest.join(MANIFEST).is_file() {
        return OnCard::Missing;
    }
    let pack = hzpak::pack_path(&dest);
    if !pack.is_file() {
        return OnCard::Unpacked;
    }
    match (hzpak::read_index(&pack), hzpak::top_level(&dest)) {
        (Ok(index), Ok(now))
            if hzpak::fits(&index.top, &now) && !hzpak::changes_path(&dest).exists() =>
        {
            OnCard::Packed
        }
        _ => OnCard::Stale,
    }
}

/// The prepared folder in or under `dir` (a chosen folder may be the launcher's, a version's or
/// the prepared one itself; the `.local` folder is hidden in Finder).
pub fn find_in(dir: &Path) -> Option<PathBuf> {
    if is_prepared(dir) {
        return Some(dir.to_path_buf());
    }
    let mut found = Vec::new();
    let mut places = vec![dir.to_path_buf(), dir.join("versions")];
    places.extend(subfolders(&dir.join("versions")));
    places.extend(subfolders(dir));
    for place in places {
        for inner in [
            place.join(".local/horizon-client"),
            place.join("horizon-client"),
            place.clone(),
        ] {
            for game in subfolders(&inner) {
                search(&game, 0, &mut found);
            }
        }
    }
    found.sort_by_key(|p: &Prepared| std::cmp::Reverse(p.modified));
    found.into_iter().next().map(|p| p.folder)
}

#[cfg(test)]
mod find_tests {
    #[test]
    fn a_launcher_version_folder_leads_to_its_prepared_files() {
        let root = std::env::temp_dir().join(format!("companion-launcher-{}", std::process::id()));
        let game = root.join("versions/v2026.11.18.1/.local/horizon-client/26.3");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join(super::MANIFEST), "{}").unwrap();
        assert_eq!(super::find_in(&root).as_deref(), Some(game.as_path()));
        assert_eq!(
            super::find_in(&root.join("versions/v2026.11.18.1")).as_deref(),
            Some(game.as_path())
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
