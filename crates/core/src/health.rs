//! Reading a whole SD card at once, and what's wrong with it, worst first.

use crate::cleanup::{self, Cleanable};
use crate::logs::{self, Cause};
use crate::releases::{self, Release};
use crate::resources::{self, Age, OnCard};
use crate::saves::{self, World};
use crate::sd::SdCard;
use crate::sync::{self, Direction};
use crate::{Progress, names, size_text};
use std::path::{Path, PathBuf};

/// Everything read from a card (slow: walks folders, so not on the window's thread).
#[derive(Clone, Debug)]
pub struct Reading {
    pub card: SdCard,
    pub installed: Option<String>,
    pub client_txt: bool,
    pub on_card: OnCard,
    /// Only when there are game files.
    pub age: Option<Age>,
    pub hidden: usize,
    pub causes: Vec<Cause>,
    pub switch_worlds: Vec<World>,
    pub computer_worlds: Vec<World>,
    pub to_switch: sync::Plan,
    pub to_computer: sync::Plan,
    pub log_files: Vec<PathBuf>,
    pub cleanables: Vec<Cleanable>,
}

pub fn read(card: &SdCard) -> Reading {
    let on_card = resources::on_card(card);
    let log = card.horizon().join("log.txt");
    let (to_switch, to_computer) = match saves::computer_home() {
        Some(home) => (
            sync::plan(&home, &card.horizon(), Direction::ToSwitch),
            sync::plan(&card.horizon(), &home, Direction::ToComputer),
        ),
        None => Default::default(),
    };
    Reading {
        installed: releases::installed_version(card),
        client_txt: card.horizon().join("client.txt").is_file(),
        age: (on_card != OnCard::Missing).then(|| resources::age(&card.resources())),
        on_card,
        hidden: names::hidden(card).len(),
        causes: logs::read(&log, usize::MAX)
            .map(|text| logs::causes(&text))
            .unwrap_or_default(),
        switch_worlds: saves::worlds_in(&saves::switch_saves(card)),
        computer_worlds: saves::computer_saves()
            .map(|s| saves::worlds_in(&s))
            .unwrap_or_default(),
        to_switch,
        to_computer,
        log_files: logs::files(card),
        cleanables: cleanup::scan(card),
        card: card.clone(),
    }
}

/// What the companion can do about a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fix {
    // In the order "Fix all" does them: the quick ones aren't held up by one that can't be done.
    ClientTxt,
    HiddenNames,
    MacJunk,
    InstallBuild,
    GameFiles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// The game doesn't start (or not as the game).
    Broken,
    /// It starts, with a catch.
    Warning,
    Good,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub level: Level,
    pub title: String,
    pub detail: String,
    pub fix: Option<Fix>,
}

/// The card's checks, worst first. `newest` is the newest full release, once it's known.
pub fn checks(r: &Reading, newest: Option<&Release>) -> Vec<Check> {
    let mut out = Vec::new();
    let mut add = |level, title: &str, detail: String, fix| {
        out.push(Check {
            level,
            title: title.into(),
            detail,
            fix,
        })
    };

    match (&r.installed, newest) {
        (None, _) if !r.card.horizon().join("horizon.nro").is_file() => add(
            Level::Broken,
            "Horizon isn't on this card",
            "Install the newest build.".into(),
            Some(Fix::InstallBuild),
        ),
        (None, _) => add(
            Level::Warning,
            "The build on the card has no version",
            "Its horizon.nro may be damaged: install it again if the game doesn't start.".into(),
            Some(Fix::InstallBuild),
        ),
        (Some(here), Some(new)) if releases::is_newer(new.version(), here) => add(
            Level::Warning,
            "A newer Horizon is out",
            format!("{here} is on the card, {} is the newest.", new.version()),
            Some(Fix::InstallBuild),
        ),
        (Some(here), _) => add(Level::Good, "Horizon", format!("Version {here}."), None),
    }
    if r.installed.is_some() && !r.client_txt {
        add(
            Level::Broken,
            "client.txt is missing",
            "Without it horizon.nro runs a dedicated server, not the game.".into(),
            Some(Fix::ClientTxt),
        );
    }

    let copy = Some(Fix::GameFiles);
    match (r.age, &r.on_card) {
        (None, _) => add(
            Level::Broken,
            "No game files",
            "Copy Minecraft's files, prepared by the Horizon launcher.".into(),
            copy,
        ),
        (Some(Age::TooOld), _) => add(
            Level::Broken,
            "The game files are too old",
            "They're from before Horizon 2.0: the game loads for minutes, then goes black and closes. Update the Horizon launcher, press Play once, then copy them again.".into(),
            copy,
        ),
        (Some(Age::OtherMinecraft), _) => add(
            Level::Broken,
            "The game files are of another Minecraft version",
            "Horizon needs Minecraft 26.3's: prepare them again with the Horizon launcher.".into(),
            copy,
        ),
        (Some(Age::Damaged), _) => add(
            Level::Broken,
            "The game files are incomplete",
            "Some are missing or don't read: copy them again.".into(),
            copy,
        ),
        (Some(Age::Current), OnCard::Packed) => add(
            Level::Good,
            "Game files",
            "Copied and packed: the game starts fast.".into(),
            None,
        ),
        (Some(Age::Current), _) => add(
            Level::Warning,
            "The game files aren't packed",
            "The Switch would pack them itself, which takes it about 20 minutes.".into(),
            copy,
        ),
    }

    if r.hidden > 0 {
        add(
            Level::Warning,
            "Names the Switch can't see",
            format!(
                "{} files or folders have letters outside plain Latin, so the game doesn't list them. They can be renamed the way Horizon NX 2.2 keeps such names; the game then shows them as they are.",
                r.hidden
            ),
            Some(Fix::HiddenNames),
        );
    }
    if let Some(junk) = r.cleanables.iter().find(|c| c.label == cleanup::MAC_JUNK) {
        add(
            Level::Warning,
            "macOS left hidden files on the card",
            format!(
                "{} of them: they can stop the game from starting.",
                junk.paths.len()
            ),
            Some(Fix::MacJunk),
        );
    }
    if let Some((free, _)) = r.card.space()
        && free < 500_000_000
    {
        add(
            Level::Warning,
            "The card is nearly full",
            format!("{} free: see Free up space.", size_text(free)),
            None,
        );
    }
    for cause in &r.causes {
        match cause {
            // The card itself was checked above.
            Cause::OldGameFiles | Cause::MacJunk => {}
            Cause::StartedFromAlbum => add(
                Level::Warning,
                "Last started from the Album",
                "The game gets too little memory there. Hold R while you start a game, then pick Horizon in the homebrew menu.".into(),
                None,
            ),
            Cause::OutOfMemory => add(
                Level::Warning,
                "The last run ran out of memory",
                "Try fewer or smaller resource packs, or a shorter render distance.".into(),
                None,
            ),
        }
    }
    out.sort_by_key(|c| c.level);
    out
}

/// Does `fixes` in order. `source` is the prepared folder for [`Fix::GameFiles`].
pub fn apply(
    fixes: &[Fix],
    card: &SdCard,
    newest: Option<&Release>,
    source: Option<&Path>,
    progress: &Progress,
) -> Result<String, String> {
    let mut done = Vec::new();
    for fix in fixes {
        progress.check()?;
        done.push(match fix {
            Fix::InstallBuild => releases::install(
                newest.ok_or("The newest release isn't known: check the internet connection.")?,
                card,
                progress,
            )?,
            Fix::ClientTxt => {
                releases::write_client_txt(card)?;
                "client.txt is back.".into()
            }
            Fix::GameFiles => {
                let source = source.ok_or("No prepared game files on this computer: start the Horizon launcher and press Play once.")?;
                if resources::age(source) != Age::Current {
                    return Err("The game files on this computer don't fit either: update the Horizon launcher and press Play once.".into());
                }
                resources::copy_and_pack(source, card, progress)?
            }
            Fix::HiddenNames => names::reveal_hidden(card)?,
            Fix::MacJunk => format!(
                "{} hidden files removed.",
                crate::sd::remove_mac_junk(&card.horizon())
            ),
        });
    }
    Ok(done.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_files_from_before_2_0_are_caught_though_their_manifest_looks_current() {
        let root = std::env::temp_dir().join(format!("companion-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let files = root.join("switch/horizon/resources");
        std::fs::create_dir_all(&files).unwrap();
        let card = SdCard::at(&root).unwrap();
        let game_files = || {
            let found = checks(&read(&card), None);
            let check = found.iter().find(|c| c.fix == Some(Fix::GameFiles));
            let check = check.unwrap();
            (check.level, check.title.clone())
        };
        assert_eq!(game_files().1, "No game files");
        std::fs::write(
            files.join(resources::MANIFEST),
            r#"{"schema":7,"data_version":5023}"#,
        )
        .unwrap();
        assert_eq!(game_files().1, "The game files are incomplete");
        let blocks = files.join("block-behaviours.json");
        std::fs::write(&blocks, "{}").unwrap();
        assert_eq!(
            game_files(),
            (Level::Broken, "The game files are too old".into())
        );
        std::fs::write(&blocks, r#"{"fluid_state_definitions":{}}"#).unwrap();
        assert_eq!(
            game_files(),
            (Level::Warning, "The game files aren't packed".into())
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(releases::is_newer("2.10.0", "2.9.1"));
        assert!(!releases::is_newer("2.2.1", "2.2.1"));
        assert!(!releases::is_newer("2.3.0", "2.2.1-test"));
    }
}
