//! `horizon.nro` from the fork's GitHub releases, onto the SD card; and which one is there.

use crate::sd::SdCard;
use crate::{Progress, download, fetch};
use serde::Deserialize;
use std::path::Path;

/// Where the Switch builds are published.
pub const REPOSITORY: &str = "KoHaku743/horizon-nx";

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// Whether it carries a Switch build.
    pub fn has_build(&self) -> bool {
        self.asset("horizon.nro").is_some()
    }

    /// "1.3.0" for `v1.3.0`.
    pub fn version(&self) -> &str {
        self.tag_name.trim_start_matches('v')
    }

    /// The day it was published ("2026-11-20").
    pub fn date(&self) -> &str {
        self.published_at
            .as_deref()
            .map_or("", |d| d.get(..10).unwrap_or(d))
    }
}

/// The releases with a Switch build, newest first.
pub fn list() -> Result<Vec<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30");
    let body = fetch(&url, 8 << 20)?;
    let releases: Vec<Release> =
        serde_json::from_slice(&body).map_err(|e| format!("GitHub's answer: {e}"))?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft && r.has_build())
        .collect())
}

/// Puts `release`'s `horizon.nro` on the card (checked against its `.sha256`), and `client.txt`
/// if it isn't there (without it the .nro starts a dedicated server).
pub fn install(release: &Release, card: &SdCard, progress: &Progress) -> Result<String, String> {
    let nro = release
        .asset("horizon.nro")
        .ok_or("This release has no horizon.nro.")?;
    let expected = match release.asset("horizon.nro.sha256") {
        Some(sum) => {
            let text = String::from_utf8(fetch(&sum.browser_download_url, 4096)?)
                .map_err(|_| "The release's checksum isn't text.")?;
            Some(
                text.split_whitespace()
                    .next()
                    .ok_or("The release's checksum is empty.")?
                    .to_lowercase(),
            )
        }
        None => None,
    };
    let dir = card.horizon();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if let Some((free, _)) = card.space()
        && free < nro.size + (16 << 20)
    {
        return Err(format!(
            "The SD card needs {} free.",
            crate::size_text(nro.size)
        ));
    }
    progress.step(
        format!("Downloading Horizon {}", release.version()),
        nro.size,
    );
    let fresh = dir.join("horizon.nro.new");
    download(&nro.browser_download_url, &fresh, progress)?;
    progress.step("Checking the download", 0);
    let got = crate::sha256_file(&fresh)?;
    if let Some(expected) = &expected
        && &got != expected
    {
        let _ = std::fs::remove_file(&fresh);
        return Err("The download doesn't match the release's checksum: try again.".into());
    }
    let target = dir.join("horizon.nro");
    let _ = std::fs::remove_file(&target);
    std::fs::rename(&fresh, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    let client = dir.join("client.txt");
    if !client.exists() {
        std::fs::write(
            &client,
            "This file makes horizon.nro start the game. Without it, it runs a dedicated server.\n",
        )
        .map_err(|e| format!("{}: {e}", client.display()))?;
    }
    crate::sd::remove_mac_junk(&dir);
    Ok(format!(
        "Horizon {} is on the SD card{}.",
        release.version(),
        if expected.is_some() {
            " (checksum verified)"
        } else {
            ""
        }
    ))
}

/// The version of the `.nro` on the card, from its NACP (the homebrew menu shows the same).
pub fn installed_version(card: &SdCard) -> Option<String> {
    nro_version(&card.horizon().join("horizon.nro"))
}

/// An NRO's display version: its asset section's NACP, `display_version` at 0x3060.
pub fn nro_version(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 0x80];
    file.read_exact(&mut header).ok()?;
    if &header[0x10..0x14] != b"NRO0" {
        return None;
    }
    let nro_size = u32::from_le_bytes(header[0x18..0x1c].try_into().ok()?) as u64;
    file.seek(SeekFrom::Start(nro_size)).ok()?;
    let mut asset = [0u8; 0x38];
    file.read_exact(&mut asset).ok()?;
    if &asset[..4] != b"ASET" {
        return None;
    }
    let nacp_offset = u64::from_le_bytes(asset[0x18..0x20].try_into().ok()?);
    let nacp_size = u64::from_le_bytes(asset[0x20..0x28].try_into().ok()?);
    if nacp_size < 0x3070 {
        return None;
    }
    file.seek(SeekFrom::Start(nro_size + nacp_offset + 0x3060))
        .ok()?;
    let mut version = [0u8; 0x10];
    file.read_exact(&mut version).ok()?;
    let end = version
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(version.len());
    let text = String::from_utf8_lossy(&version[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}
