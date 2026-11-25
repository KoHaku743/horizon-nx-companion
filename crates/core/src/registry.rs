//! Horizon's mod registry (https://horizon-rust.github.io/horizon-mods/index.json) from the
//! computer: what the Switch build installs by itself (content mods and resource packs made for
//! 26.3), checked exactly as the game's Get Mods checks them (`horizon_mod_package::index::check_file`,
//! the same code), then put on the SD card.
//!
//! The computer checks a mod in a fraction of the time the Switch takes, so every mod can be
//! checked up front: the list says which ones this Switch build reads before anything is copied.

use crate::sd::SdCard;
use crate::{Progress, download, fetch, size_text};
use horizon_mod_package::index::{self, Index, Kind, Project, Version};
use std::path::{Path, PathBuf};

pub const INDEX_URL: &str = "https://horizon-rust.github.io/horizon-mods/index.json";
/// The Minecraft version the Switch build plays: the vendored Horizon's (its Get Mods' `GAME`).
pub const GAME: &str = horizon_core::version::VERSION_ID;
/// Where the game keeps mods (beside `options.txt`), and the file that turns them on.
pub const MODS_DIR: &str = "horizon-mods";
pub const MODS_FILE: &str = "horizon-mods.txt";
/// What Horizon's Get Mods (and the launcher) record of the registry files they installed, beside
/// `options.txt`: the game reads it for each listing's installed version.
pub const INSTALLED_FILE: &str = "mod-browser.json";
/// Why a mod that runs code is refused: the Switch build has no runtime for it.
pub const CODE_MOD: &str = "it runs code (a WebAssembly or native mod), which the Switch can't run";
/// The registry's pages, as the game links them.
const SITE: &str = "https://playhorizon.dev/mods";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferKind {
    Mod,
    ResourcePack,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OfferState {
    /// Not checked yet: installing checks it.
    Unchecked,
    /// Checked: this Switch build reads it.
    Available,
    /// Made for a newer Horizon than the Switch build (why).
    Unsupported(String),
    Installed,
    /// Installed, an older version.
    Update,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub id: String,
    pub kind: OfferKind,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub size: u64,
    pub downloads: u64,
    pub state: OfferState,
}

/// Reads the registry's index (entries this build can't read are left out).
pub fn load_index() -> Result<Index, String> {
    let body = fetch(INDEX_URL, 32 << 20)?;
    let text = String::from_utf8(body).map_err(|e| e.to_string())?;
    Index::parse(&text)
}

/// Where the game keeps its last copy of the index, in its home folder (`mod_registry::CACHE`).
pub const GAME_CACHE: &str = "mod-browser/horizon-index.json";

/// The companion's own last copy.
fn saved_index() -> PathBuf {
    checked_dir().with_file_name("index.json")
}

/// The index, as the game reads it: from the registry, kept for later; when the registry can't
/// be reached, the newest copy kept here, by Horizon on this computer or by the game on `card`.
/// The note says when it's a kept copy.
pub fn load_index_or_kept(card: Option<&SdCard>) -> Result<(Index, Option<String>), String> {
    let online = fetch(INDEX_URL, 32 << 20)
        .and_then(|body| String::from_utf8(body).map_err(|e| e.to_string()));
    let failed = match online.and_then(|text| Index::parse(&text).map(|i| (i, text))) {
        Ok((index, text)) => {
            let path = saved_index();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, text);
            return Ok((index, None));
        }
        Err(e) => e,
    };
    let mut kept: Vec<PathBuf> = vec![saved_index()];
    kept.extend(crate::saves::computer_home().map(|h| h.join(GAME_CACHE)));
    kept.extend(card.map(|c| c.horizon().join(GAME_CACHE)));
    newest_kept(&kept)
        .map(|(index, age)| {
            let note = format!(
                "Horizon's mod registry can't be reached ({failed}), so this is the copy saved {age}. Mods checked on this computer before can still be installed; the others can't be downloaded until the registry is back."
            );
            (index, Some(note))
        })
        .ok_or_else(|| {
            format!("{failed}. Horizon's mod registry can't be reached, and no copy of it was saved on this computer or the card.")
        })
}

/// The newest of `paths` that reads as an index, and how old it is ("3 days ago").
fn newest_kept(paths: &[PathBuf]) -> Option<(Index, String)> {
    let mut found: Vec<(std::time::SystemTime, &PathBuf)> = paths
        .iter()
        .filter_map(|p| Some((std::fs::metadata(p).ok()?.modified().ok()?, p)))
        .collect();
    found.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    found.into_iter().find_map(|(time, path)| {
        let index = Index::parse(&std::fs::read_to_string(path).ok()?).ok()?;
        let days = time.elapsed().map_or(0, |e| e.as_secs() / 86_400);
        let age = match days {
            0 => "in the last day".to_owned(),
            1 => "a day ago".to_owned(),
            n => format!("{n} days ago"),
        };
        Some((index, age))
    })
}

/// The version of `p` the Switch installs, if it's one it can, by the rule of Horizon NX 2.0's
/// Get Mods (`mod_registry::installable`): a content mod (its only capability registries) or a
/// resource pack (a `.zip`), made for this game, with no capability this Horizon doesn't know.
fn offered<'a>(index: &Index, p: &'a Project) -> Option<(OfferKind, &'a Version)> {
    let kind = match p.kind {
        Kind::Mod => OfferKind::Mod,
        Kind::ResourcePack => OfferKind::ResourcePack,
        _ => return None,
    };
    let v = index.installable(p, GAME)?;
    let fits = match kind {
        OfferKind::Mod => {
            v.capabilities.known() == [horizon_mod_package::Capability::Registries]
                && v.file.name.ends_with(".hmod")
        }
        OfferKind::ResourcePack => v.file.name.ends_with(".zip"),
    };
    fits.then_some((kind, v))
}

fn plain(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', ':', '\0', '\n'])
}

/// Where `p`'s file goes on the card.
fn destination(
    card: &SdCard,
    kind: OfferKind,
    p: &Project,
    v: &Version,
) -> Result<PathBuf, String> {
    match kind {
        OfferKind::Mod if index::valid_id(&p.id) && plain(&p.id) => {
            Ok(card.horizon().join(MODS_DIR).join(format!("{}.hmod", p.id)))
        }
        OfferKind::ResourcePack if plain(&v.file.name) => {
            Ok(card.horizon().join("resourcepacks").join(&v.file.name))
        }
        _ => Err(format!("{}: the registry names its file oddly", p.name)),
    }
}

/// This computer's copies of checked files (`<sha256>`) and why others were refused
/// (`<sha256>.refused`).
fn checked_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("horizon-nx-companion")
        .join("checked")
}

fn checked_path(v: &Version) -> PathBuf {
    checked_dir().join(&v.file.sha256)
}

fn refused_path(v: &Version) -> PathBuf {
    checked_dir().join(format!("{}.refused", v.file.sha256))
}

/// What was found of `v` before.
fn known(v: &Version) -> Option<Result<(), String>> {
    if checked_path(v).is_file() {
        return Some(Ok(()));
    }
    std::fs::read_to_string(refused_path(v)).ok().map(Err)
}

/// The registry's offers, with what the card holds and what was checked.
pub fn offers(index: &Index, card: Option<&SdCard>) -> Vec<Offer> {
    let mut projects: Vec<&Project> = index.projects.iter().collect();
    projects.sort_by(|a, b| b.downloads.cmp(&a.downloads).then(a.name.cmp(&b.name)));
    let mut out = Vec::new();
    for p in projects {
        let Some((kind, v)) = offered(index, p) else {
            continue;
        };
        let checked = match (kind, known(v)) {
            (_, Some(Err(why))) => OfferState::Unsupported(why),
            (OfferKind::ResourcePack, _) | (_, Some(Ok(()))) => OfferState::Available,
            (_, None) => OfferState::Unchecked,
        };
        let state = match card.and_then(|c| destination(c, kind, p, v).ok()) {
            Some(dest) if dest.is_file() => match kind {
                OfferKind::Mod => match crate::sha256_file(&dest) {
                    Ok(hash) if hash == v.file.sha256 => OfferState::Installed,
                    _ if matches!(checked, OfferState::Unsupported(_)) => checked,
                    _ => OfferState::Update,
                },
                OfferKind::ResourcePack => OfferState::Installed,
            },
            _ => checked,
        };
        out.push(Offer {
            id: p.id.clone(),
            kind,
            name: p.name.clone(),
            version: v.version.clone(),
            summary: p.summary.clone(),
            size: v.file.size,
            downloads: p.downloads,
            state,
        });
    }
    out
}

/// Downloads and checks `v` of `p` (once: the result is kept on this computer).
fn fetch_checked(
    index: &Index,
    p: &Project,
    v: &Version,
    progress: &Progress,
) -> Result<PathBuf, String> {
    match known(v) {
        Some(Ok(())) => return Ok(checked_path(v)),
        Some(Err(why)) => return Err(why),
        None => {}
    }
    let url = &v.file.url;
    if !url.starts_with("https://github.com/") || !url.contains("/releases/download/") {
        return Err("not a GitHub release file".into());
    }
    if v.file.size > index::MAX_FILE || !plain(&v.file.name) || !plain(&v.file.sha256) {
        return Err("the registry lists an odd file".into());
    }
    let dir = checked_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.download", v.file.sha256));
    progress.step(format!("Downloading {}", p.name), v.file.size);
    download(url, &part, progress)?;
    progress.step(format!("Checking {}", p.name), 0);
    // The index doesn't say which mods run code (a WebAssembly mod is a `mod` like a content mod):
    // the package does. The Switch build has no code runtime, so a mod with code is refused here.
    let checked = index::check_file(index, p, v, &part).and_then(|trust| {
        if v.file.name.ends_with(".hmod") {
            let package = horizon_mod_package::Package::read(&part)?;
            if package.manifest.wasm.is_some() || package.manifest.native.is_some() {
                return Err(CODE_MOD.into());
            }
        }
        Ok(trust)
    });
    match checked {
        Ok(_) => {
            std::fs::rename(&part, checked_path(v)).map_err(|e| e.to_string())?;
            Ok(checked_path(v))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            // A bad download isn't the file's verdict.
            if !(e.contains("bytes, the index lists") || e.contains("SHA-256")) {
                let _ = std::fs::write(refused_path(v), &e);
            }
            Err(e)
        }
    }
}

/// Checks every mod that wasn't checked yet (on this computer, seconds each).
pub fn check_all(index: &Index, progress: &Progress) -> Result<String, String> {
    let todo: Vec<(&Project, &Version)> = index
        .projects
        .iter()
        .filter_map(|p| {
            offered(index, p)
                .filter(|(k, _)| *k == OfferKind::Mod)
                .map(|(_, v)| (p, v))
        })
        .filter(|(_, v)| known(v).is_none())
        .collect();
    let (mut ok, mut code, mut refused, mut failed) = (0, 0, 0, 0);
    for (i, (p, v)) in todo.iter().enumerate() {
        progress.check()?;
        let _ = i;
        match fetch_checked(index, p, v, progress) {
            Ok(_) => ok += 1,
            Err(e) if e == CODE_MOD => code += 1,
            Err(_) if refused_path(v).is_file() => refused += 1,
            Err(_) => failed += 1,
        }
    }
    let mut summary = format!(
        "Checked {} mods: {ok} work on the Switch build, {code} run code (which the Switch can't run), \
         {refused} need a newer Horizon.",
        todo.len()
    );
    if failed > 0 {
        summary.push_str(&format!(" {failed} couldn't be downloaded."));
    }
    Ok(summary)
}

/// Installs `id` on the card: a mod into `horizon-mods/` and turned on, a resource pack into
/// `resourcepacks/`.
pub fn install(
    index: &Index,
    id: &str,
    card: &SdCard,
    progress: &Progress,
) -> Result<String, String> {
    let p = index.project(id).ok_or("Not in the registry.")?;
    let (kind, v) = offered(index, p).ok_or("The Switch build can't install this one.")?;
    let dest = destination(card, kind, p, v)?;
    let file = fetch_checked(index, p, v, progress).map_err(|e| {
        if e == CODE_MOD {
            format!("{} {CODE_MOD}.", p.name)
        } else if refused_path(v).is_file() {
            format!(
                "{} needs a newer Horizon than the Switch build: {e}",
                p.name
            )
        } else {
            format!("{}: {e}", p.name)
        }
    })?;
    progress.step(format!("Copying {} to the SD card", p.name), 0);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    plain_copy(&file, &dest)?;
    // Checked a moment ago (or before, and kept): this only reads whose signature it carries.
    let verified = index::check_file(index, p, v, &file) == Ok(index::Trust::Verified);
    let relative = dest
        .strip_prefix(card.horizon())
        .map_err(|_| "the destination left the game folder".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    record_installed(&card.horizon(), p, v, kind, &relative, verified)?;
    Ok(match kind {
        OfferKind::Mod => {
            set_mod(card, &p.id, true)?;
            format!("{} {} is on the SD card and turned on.", p.name, v.version)
        }
        OfferKind::ResourcePack => format!(
            "{} is on the SD card: turn it on in the game's Options > Resource Packs.",
            p.name
        ),
    })
}

/// Removes `id`'s file from the card (a mod is also turned off).
pub fn remove(index: &Index, id: &str, card: &SdCard) -> Result<String, String> {
    let p = index.project(id).ok_or("Not in the registry.")?;
    let (kind, v) = offered(index, p).ok_or("Not one the Switch build installs.")?;
    let dest = destination(card, kind, p, v)?;
    std::fs::remove_file(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    if kind == OfferKind::Mod {
        set_mod(card, &p.id, false)?;
    }
    forget_installed(&card.horizon(), &p.id)?;
    Ok(format!("{} is removed from the SD card.", p.name))
}

/// A copy with no macOS metadata (no `._` file on the card).
pub(crate) fn plain_copy(from: &Path, to: &Path) -> Result<(), String> {
    let data = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let temporary = to.with_extension("copying");
    std::fs::write(&temporary, &data).map_err(|e| format!("{}: {e}", to.display()))?;
    let _ = std::fs::remove_file(to);
    std::fs::rename(&temporary, to).map_err(|e| format!("{}: {e}", to.display()))
}

/// Turns a mod on (`mod.<id>:true` in `horizon-mods.txt`) or forgets it, keeping the file's other
/// lines.
fn set_mod(card: &SdCard, id: &str, on: bool) -> Result<(), String> {
    let path = card.horizon().join(MODS_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let prefix = format!("mod.{id}:");
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.starts_with(&prefix)).collect();
    let line = format!("mod.{id}:true");
    if on {
        lines.push(&line);
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
}

/// The entries of `home`'s `mod-browser.json` (none when it's missing or unreadable).
fn installed_entries(home: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(home.join(INSTALLED_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json["entries"].as_array().cloned())
        .unwrap_or_default()
}

fn save_installed(home: &Path, entries: Vec<serde_json::Value>) -> Result<(), String> {
    let path = home.join(INSTALLED_FILE);
    let text = serde_json::to_string_pretty(&serde_json::json!({ "entries": entries }))
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// Records `v` of `p` as installed at `relative` in `home`, as Horizon's Get Mods records its own
/// installs (`mod_registry.rs`), replacing an older entry of the project.
fn record_installed(
    home: &Path,
    p: &Project,
    v: &Version,
    kind: OfferKind,
    relative: &str,
    verified: bool,
) -> Result<(), String> {
    let mut entries = installed_entries(home);
    entries.retain(|e| e["project"].as_str() != Some(p.id.as_str()));
    entries.push(serde_json::json!({
        "source": "horizon",
        "project": p.id,
        "slug": p.id,
        "title": p.name,
        "kind": match kind {
            OfferKind::Mod => "mod",
            OfferKind::ResourcePack => "resource-pack",
        },
        "version": v.version,
        "number": v.version,
        "file": relative,
        "checksum": format!("SHA-256:{}", v.file.sha256),
        "page": format!("{SITE}/{}", p.id),
        "dependency": false,
        "verdict": if verified { "Verified" } else { "Unsigned" },
    }));
    save_installed(home, entries)
}

/// Drops `id`'s entry from `home`'s `mod-browser.json`, keeping the others.
fn forget_installed(home: &Path, id: &str) -> Result<(), String> {
    let mut entries = installed_entries(home);
    let before = entries.len();
    entries.retain(|e| e["project"].as_str() != Some(id));
    if entries.len() == before {
        return Ok(());
    }
    save_installed(home, entries)
}

/// A line for an offer's size and state, for lists.
pub fn describe(offer: &Offer) -> String {
    format!(
        "{} {} · {}",
        offer.name,
        offer.version,
        size_text(offer.size)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Horizon's sample index (horizon-mod-package's tests/index), with a content-only mod and a
    /// resource pack for 26.3, and its own mod given a capability this Horizon doesn't know.
    fn sample() -> Index {
        let text = include_str!("../tests/data/index-v1.json");
        let mut json: serde_json::Value = serde_json::from_str(text).unwrap();
        let projects = json["projects"].as_array_mut().unwrap();
        projects[0]["versions"][0]["capabilities"] = serde_json::json!(["hud"]);
        let mut content = projects[0].clone();
        content["id"] = "doors".into();
        content["name"] = "Doors".into();
        content["versions"][0]["capabilities"] = serde_json::json!(["registries"]);
        let mut pack = projects[0].clone();
        pack["id"] = "faithful".into();
        pack["kind"] = "resourcepack".into();
        pack["name"] = "Faithful".into();
        pack["versions"][0]["file"]["name"] = "Faithful.zip".into();
        pack["versions"][0]["capabilities"] = serde_json::json!([]);
        projects.push(content);
        projects.push(pack);
        Index::parse_local(&json.to_string()).unwrap()
    }

    #[test]
    fn the_newest_kept_index_that_reads_is_taken() {
        let dir = std::env::temp_dir().join(format!("companion-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = include_str!("../tests/data/index-v1.json");
        let (old, broken) = (dir.join("old.json"), dir.join("broken.json"));
        std::fs::write(&old, real).unwrap();
        std::fs::write(&broken, "<html>404</html>").unwrap();
        let missing = dir.join("missing.json");
        let (index, _) = super::newest_kept(&[missing, broken, old]).unwrap();
        assert_eq!(
            index.projects.len(),
            Index::parse(real).unwrap().projects.len()
        );
        assert!(super::newest_kept(&[dir.join("broken.json")]).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_what_the_switch_installs_is_offered_and_mods_turn_on_and_off() {
        let index = sample();
        let root = std::env::temp_dir().join(format!("companion-card-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("switch/horizon")).unwrap();
        let card = SdCard::at(&root).unwrap();

        let offers = offers(&index, Some(&card));
        let ids: Vec<&str> = offers.iter().map(|o| o.id.as_str()).collect();
        // coords_hud asks for a capability this Horizon doesn't know: not offered.
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.contains(&"doors") && ids.contains(&"faithful"));

        std::fs::write(
            card.horizon().join(MODS_FILE),
            "mod.other:true\nmod.doors:false\n",
        )
        .unwrap();
        set_mod(&card, "doors", true).unwrap();
        let text = std::fs::read_to_string(card.horizon().join(MODS_FILE)).unwrap();
        assert_eq!(text, "mod.other:true\nmod.doors:true\n");
        set_mod(&card, "doors", false).unwrap();
        let text = std::fs::read_to_string(card.horizon().join(MODS_FILE)).unwrap();
        assert_eq!(text, "mod.other:true\n");

        // A file on the card that isn't the listed one is an update.
        let p = index.project("doors").unwrap();
        let dest = destination(&card, OfferKind::Mod, p, &p.versions[0]).unwrap();
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"old").unwrap();
        let doors = offers_state(&index, &card, "doors");
        assert_eq!(doors, OfferState::Update);
        assert!(dest.ends_with("switch/horizon/horizon-mods/doors.hmod"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn offers_state(index: &Index, card: &SdCard, id: &str) -> OfferState {
        offers(index, Some(card))
            .into_iter()
            .find(|o| o.id == id)
            .unwrap()
            .state
    }

    /// Reads the live registry (network): `cargo test -p companion-core -- --ignored`.
    #[test]
    #[ignore]
    fn the_registry_lists_offers() {
        let index = super::load_index().unwrap();
        let offers = super::offers(&index, None);
        assert!(!offers.is_empty());
        for o in offers.iter().take(5) {
            println!("{} {:?} {:?}", super::describe(o), o.kind, o.state);
        }
    }
}
