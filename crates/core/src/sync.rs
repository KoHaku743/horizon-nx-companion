//! Keeping Horizon on this computer and Horizon on the Switch alike: mods, resource packs and the
//! server list, copied one way at a time. Nothing is ever deleted: a copy adds what the other side
//! lacks and updates what differs; servers are added to the other side's list.
//!
//! Only mods the Switch build plays go to the Switch: content mods (no code), readable by its
//! version of the mod format, which is the vendored `horizon-mods`.

use crate::nbt::{self, Tag};
use crate::registry::{MODS_DIR, MODS_FILE};
use crate::sd::is_mac_junk;
use crate::{Progress, size_text};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const PACKS_DIR: &str = "resourcepacks";
pub const SERVERS: &str = "servers.dat";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ToSwitch,
    ToComputer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Mod,
    Pack,
}

/// A file or folder to copy.
#[derive(Clone, Debug, PartialEq)]
pub struct Copy {
    pub what: What,
    /// What the player reads ("Macaw's Doors", "Faithful.zip").
    pub name: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub bytes: u64,
    /// The other side has an older or different one.
    pub update: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub copies: Vec<Copy>,
    /// Mods' on/off lines to carry over (`mod.<id>:<on>`).
    pub mod_lines: Vec<(String, bool)>,
    /// Servers the other side's list lacks.
    pub servers: Vec<Tag>,
    /// What stays behind, and why.
    pub skipped: Vec<(String, String)>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.copies.is_empty() && self.servers.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.copies.iter().map(|c| c.bytes).sum()
    }

    /// "2 mods, 1 resource pack, 3 servers".
    pub fn summary(&self) -> String {
        let count = |w: What| self.copies.iter().filter(|c| c.what == w).count();
        let mut parts = Vec::new();
        let mut part = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        part(count(What::Mod), "mod", "mods");
        part(count(What::Pack), "resource pack", "resource packs");
        part(self.servers.len(), "server", "servers");
        if parts.is_empty() {
            "Nothing: both sides have the same.".into()
        } else {
            parts.join(", ")
        }
    }
}

/// A mod found in a `horizon-mods` folder.
struct FoundMod {
    path: PathBuf,
    /// The manifest, or why it can't be read by this version of the format.
    read: Result<(String, String, bool), String>,
}

fn mods_in(home: &Path) -> Vec<FoundMod> {
    horizon_mod_package::find(&home.join(MODS_DIR))
        .into_iter()
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| is_mac_junk(&n.to_string_lossy()))
        })
        .map(|path| {
            let read = horizon_mod_package::Package::read(&path).map(|p| {
                (
                    p.manifest.id.clone(),
                    p.manifest.name.clone(),
                    p.manifest.wasm.is_some() || p.manifest.native.is_some(),
                )
            });
            FoundMod { path, read }
        })
        .collect()
}

/// The on/off lines of a `horizon-mods.txt`.
fn mod_lines(home: &Path) -> HashMap<String, bool> {
    std::fs::read_to_string(home.join(MODS_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (id, on) = l.strip_prefix("mod.")?.split_once(':')?;
            Some((id.to_owned(), on.trim() == "true"))
        })
        .collect()
}

fn size_of(path: &Path) -> u64 {
    if path.is_dir() {
        crate::saves::folder_size(path)
    } else {
        std::fs::metadata(path).map_or(0, |m| m.len())
    }
}

/// Whether two copies of a mod are the same.
fn same_mod(a: &Path, b: &Path) -> bool {
    if a.is_dir() || b.is_dir() {
        return a.is_dir() && b.is_dir() && size_of(a) == size_of(b);
    }
    match (crate::sha256_file(a), crate::sha256_file(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Whether two copies of a resource pack are the same (size and time, as the copy keeps it).
fn same_pack(a: &Path, b: &Path) -> bool {
    if a.is_dir() || b.is_dir() {
        return a.is_dir() && b.is_dir() && size_of(a) == size_of(b);
    }
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => {
            x.len() == y.len() && (crate::hzpak::mtime(&x) - crate::hzpak::mtime(&y)).abs() <= 2
        }
        _ => false,
    }
}

fn server_key(server: &Tag) -> Option<String> {
    let ip = server.get("ip")?.as_str()?.trim().to_lowercase();
    (!ip.is_empty()).then_some(ip)
}

/// The servers of a `servers.dat` (none if it's missing or unreadable).
pub fn servers_in(home: &Path) -> Vec<Tag> {
    std::fs::read(home.join(SERVERS))
        .ok()
        .and_then(|bytes| nbt::read(&bytes).ok())
        .and_then(|(_, root)| match root.get("servers") {
            Some(Tag::List(_, items)) => Some(items.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn server_name(server: &Tag) -> String {
    let name = server.get("name").and_then(Tag::as_str).unwrap_or("");
    let ip = server.get("ip").and_then(Tag::as_str).unwrap_or("");
    if name.is_empty() || name == ip {
        ip.to_owned()
    } else {
        format!("{name} ({ip})")
    }
}

/// What copying `from` (a Horizon home folder) to `to` would do.
pub fn plan(from: &Path, to: &Path, direction: Direction) -> Plan {
    let mut plan = Plan::default();

    // Mods, matched by id (a registry install is `<id>.hmod`, a hand-copied one may be named
    // otherwise: never two copies of one mod).
    let theirs: HashMap<String, PathBuf> = mods_in(to)
        .into_iter()
        .filter_map(|m| m.read.ok().map(|(id, ..)| (id, m.path)))
        .collect();
    let lines = mod_lines(from);
    for found in mods_in(from) {
        let file_name = found
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (id, name, code) = match found.read {
            Ok(read) => read,
            Err(e) if direction == Direction::ToSwitch => {
                plan.skipped
                    .push((file_name, format!("the Switch build can't read it ({e})")));
                continue;
            }
            // The computer's Horizon may be newer: it decides.
            Err(_) => (String::new(), file_name.clone(), false),
        };
        if direction == Direction::ToSwitch && code {
            plan.skipped.push((
                name,
                "it runs code: the Switch build plays content mods only".into(),
            ));
            continue;
        }
        let target = theirs
            .get(&id)
            .cloned()
            .unwrap_or_else(|| to.join(MODS_DIR).join(&file_name));
        let exists = target.exists();
        if exists && same_mod(&found.path, &target) {
            continue;
        }
        if !id.is_empty()
            && let Some(on) = lines.get(&id)
        {
            plan.mod_lines.push((id.clone(), *on));
        }
        plan.copies.push(Copy {
            what: What::Mod,
            name,
            bytes: size_of(&found.path),
            from: found.path,
            to: target,
            update: exists,
        });
    }

    // Resource packs, matched by name.
    if let Ok(entries) = std::fs::read_dir(from.join(PACKS_DIR)) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_pack = path.is_dir() || name.to_lowercase().ends_with(".zip");
            if is_mac_junk(&name) || name.starts_with('.') || !is_pack {
                continue;
            }
            let target = to.join(PACKS_DIR).join(&name);
            let exists = target.exists();
            if exists && same_pack(&path, &target) {
                continue;
            }
            plan.copies.push(Copy {
                what: What::Pack,
                name,
                bytes: size_of(&path),
                from: path,
                to: target,
                update: exists,
            });
        }
    }

    // Servers, matched by address.
    let known: HashSet<String> = servers_in(to).iter().filter_map(server_key).collect();
    let mut added = HashSet::new();
    for server in servers_in(from) {
        if let Some(key) = server_key(&server)
            && !known.contains(&key)
            && added.insert(key)
        {
            plan.servers.push(server);
        }
    }
    plan
}

fn copy_file(from: &Path, to: &Path, progress: &Progress) -> Result<(), String> {
    let data = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let temporary = to.with_extension("copying");
    std::fs::write(&temporary, &data).map_err(|e| format!("{}: {e}", to.display()))?;
    // The time comes along, so the next comparison sees the same file.
    if let Ok(time) = std::fs::metadata(from).and_then(|m| m.modified())
        && let Ok(file) = std::fs::File::options().write(true).open(&temporary)
    {
        let _ = file.set_modified(time);
    }
    let _ = std::fs::remove_file(to);
    std::fs::rename(&temporary, to).map_err(|e| format!("{}: {e}", to.display()))?;
    progress.advance(data.len() as u64);
    Ok(())
}

/// Does `plan` into the home folder `to`.
pub fn apply(
    plan: &Plan,
    to: &Path,
    direction: Direction,
    progress: &Progress,
) -> Result<String, String> {
    progress.step("Copying", plan.bytes());
    for copy in &plan.copies {
        progress.check()?;
        progress.step(format!("Copying {}", copy.name), plan.bytes());
        if let Some(parent) = copy.to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        if copy.from.is_dir() {
            if copy.to.exists() {
                if copy.to.is_dir() {
                    std::fs::remove_dir_all(&copy.to)
                } else {
                    std::fs::remove_file(&copy.to)
                }
                .map_err(|e| format!("{}: {e}", copy.to.display()))?;
            }
            crate::saves::copy_tree(&copy.from, &copy.to, progress)?;
        } else {
            copy_file(&copy.from, &copy.to, progress)?;
        }
    }

    if !plan.mod_lines.is_empty() {
        let path = to.join(MODS_FILE);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let ids: HashSet<&str> = plan.mod_lines.iter().map(|(id, _)| id.as_str()).collect();
        let mut lines: Vec<String> = text
            .lines()
            .filter(|l| {
                l.strip_prefix("mod.")
                    .and_then(|r| r.split_once(':'))
                    .is_none_or(|(id, _)| !ids.contains(id))
            })
            .map(str::to_owned)
            .collect();
        lines.extend(
            plan.mod_lines
                .iter()
                .map(|(id, on)| format!("mod.{id}:{on}")),
        );
        let mut out = lines.join("\n");
        out.push('\n');
        std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    }

    if !plan.servers.is_empty() {
        let path = to.join(SERVERS);
        let (name, mut root) = std::fs::read(&path)
            .ok()
            .and_then(|b| nbt::read(&b).ok())
            .unwrap_or_else(|| (String::new(), Tag::Compound(Vec::new())));
        if root.get("servers").is_none()
            && let Tag::Compound(fields) = &mut root
        {
            fields.push(("servers".into(), Tag::List(10, Vec::new())));
        }
        if let Some(Tag::List(_, items)) = root.get_mut("servers") {
            items.extend(plan.servers.iter().cloned());
        }
        // As the game does: the list before is kept beside it.
        if path.is_file() {
            let _ = std::fs::copy(&path, to.join(format!("{SERVERS}_old")));
        }
        std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
        std::fs::write(&path, nbt::write(&name, &root))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }

    if direction == Direction::ToSwitch {
        crate::sd::remove_mac_junk(&to.join(MODS_DIR));
        crate::sd::remove_mac_junk(&to.join(PACKS_DIR));
    }
    let side = match direction {
        Direction::ToSwitch => "the Switch",
        Direction::ToComputer => "this computer",
    };
    let mut done = format!(
        "Copied to {side}: {} ({}).",
        plan.summary(),
        size_text(plan.bytes())
    );
    if plan.copies.iter().any(|c| c.what == What::Pack) {
        done.push_str(" Turn new resource packs on in Options > Resource Packs.");
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("companion-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn servers(list: &[(&str, &str)]) -> Vec<u8> {
        let items = list
            .iter()
            .map(|(name, ip)| {
                Tag::Compound(vec![
                    ("name".into(), Tag::String((*name).into())),
                    ("ip".into(), Tag::String((*ip).into())),
                ])
            })
            .collect();
        nbt::write(
            "",
            &Tag::Compound(vec![("servers".into(), Tag::List(10, items))]),
        )
    }

    #[test]
    fn packs_and_servers_go_over_once_and_nothing_is_deleted() {
        let (computer, switch) = (home("computer"), home("switch"));
        std::fs::create_dir_all(computer.join(PACKS_DIR)).unwrap();
        std::fs::write(computer.join(PACKS_DIR).join("Faithful.zip"), b"pack").unwrap();
        std::fs::write(computer.join(PACKS_DIR).join("._Faithful.zip"), b"junk").unwrap();
        std::fs::write(
            computer.join(SERVERS),
            servers(&[("Hypixel", "mc.hypixel.net"), ("Home", "10.0.0.2")]),
        )
        .unwrap();
        std::fs::write(
            switch.join(SERVERS),
            servers(&[("Mine", "MC.Hypixel.net "), ("Other", "play.example.org")]),
        )
        .unwrap();

        let plan = plan(&computer, &switch, Direction::ToSwitch);
        assert_eq!(plan.summary(), "1 resource pack, 1 server");
        apply(&plan, &switch, Direction::ToSwitch, &Progress::new()).unwrap();

        let names: Vec<String> = servers_in(&switch).iter().map(server_name).collect();
        assert_eq!(
            names,
            [
                "Mine (MC.Hypixel.net )",
                "Other (play.example.org)",
                "Home (10.0.0.2)"
            ]
        );
        assert!(switch.join("servers.dat_old").is_file());
        assert_eq!(
            std::fs::read(switch.join(PACKS_DIR).join("Faithful.zip")).unwrap(),
            b"pack"
        );
        assert!(!switch.join(PACKS_DIR).join("._Faithful.zip").exists());

        // Again: nothing left to do.
        assert!(super::plan(&computer, &switch, Direction::ToSwitch).is_empty());
        // And back: the Switch's own server goes to the computer.
        let back = super::plan(&switch, &computer, Direction::ToComputer);
        assert_eq!(back.summary(), "1 server");
        std::fs::remove_dir_all(&computer).unwrap();
        std::fs::remove_dir_all(&switch).unwrap();
    }

    #[test]
    fn only_content_mods_go_to_the_switch_with_their_on_off_line() {
        let (computer, switch) = (home("mods-computer"), home("mods-switch"));
        let mods = computer.join(MODS_DIR);
        std::fs::create_dir_all(&mods).unwrap();
        let content = "id = \"doors\"\nname = \"Doors\"\nversion = \"1.0.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n";
        // A wasm mod: its [wasm] section pins mod.wasm's SHA-512.
        let wasm: &[u8] = b"\0asm\x01\0\0\0";
        let sha512: String = {
            use sha2::Digest;
            sha2::Sha512::digest(wasm)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let code = format!(
            "id = \"hud\"\nname = \"HUD\"\nversion = \"1.0.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n[wasm]\nsha512 = '{sha512}'\ntoolchain = 'nightly'\nbuilt_with = 'horizon-mod-sdk test'\n"
        );
        match horizon_mod_package::Package::write_files(content, &[]) {
            Ok(bytes) => std::fs::write(mods.join("Doors-1.0.hmod"), bytes).unwrap(),
            Err(e) => panic!("{e}"),
        }
        // Packed by hand: its module isn't one Horizon would load, so the Switch side refuses it
        // whether it reads as code or not at all. Either way it must not reach the card.
        {
            use std::io::Write;
            let mut zip =
                zip::ZipWriter::new(std::fs::File::create(mods.join("hud.hmod")).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("horizon-mod.toml", options).unwrap();
            zip.write_all(code.as_bytes()).unwrap();
            zip.start_file("mod.wasm", options).unwrap();
            zip.write_all(wasm).unwrap();
            zip.finish().unwrap();
        }
        std::fs::write(computer.join(MODS_FILE), "mod.doors:true\nmod.hud:true\n").unwrap();
        std::fs::write(switch.join(MODS_FILE), "mod.other:false\n").unwrap();

        let plan = plan(&computer, &switch, Direction::ToSwitch);
        assert_eq!(plan.summary(), "1 mod");
        assert_eq!(plan.skipped.len(), 1, "{:?}", plan.skipped);
        apply(&plan, &switch, Direction::ToSwitch, &Progress::new()).unwrap();
        assert!(switch.join(MODS_DIR).join("Doors-1.0.hmod").is_file());
        assert_eq!(
            std::fs::read_to_string(switch.join(MODS_FILE)).unwrap(),
            "mod.other:false\nmod.doors:true\n"
        );
        assert!(super::plan(&computer, &switch, Direction::ToSwitch).is_empty());
        std::fs::remove_dir_all(&computer).unwrap();
        std::fs::remove_dir_all(&switch).unwrap();
    }
}
