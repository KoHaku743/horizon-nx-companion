//! The Switch's logs: reading them, and zipping them for a bug report.

use crate::sd::SdCard;
use std::io::Write;
use std::path::PathBuf;

/// The log files on the card: `log.txt` (the last run, complete) and `logs/*`, newest first.
pub fn files(card: &SdCard) -> Vec<PathBuf> {
    let dir = card.horizon();
    let mut files = Vec::new();
    for name in ["log.txt", "logs/latest.log"] {
        let path = dir.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("logs")) {
        let mut older: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "gz"))
            .collect();
        older.sort();
        older.reverse();
        files.extend(older);
    }
    files
}

/// A log's text (a `.gz` one unpacked); the last `max_lines` lines.
pub fn read(path: &std::path::Path, max_lines: usize) -> Result<String, String> {
    use std::io::Read;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = if path.extension().is_some_and(|e| e == "gz") {
        let mut out = String::new();
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut out)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        out
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].join("\n"))
}

/// The lines worth a look first: errors, warnings, crashes.
pub fn problems(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| {
            let l = l.to_lowercase();
            l.contains("error")
                || l.contains("warn")
                || l.contains("panic")
                || l.contains("crash")
                || l.contains("failed")
        })
        .map(str::to_owned)
        .collect()
}

/// Something a log says that has a known cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    OldGameFiles,
    StartedFromAlbum,
    OutOfMemory,
    MacJunk,
}

/// The known causes in a run's whole log.
pub fn causes(text: &str) -> Vec<Cause> {
    let mut found = Vec::new();
    if text.contains("re-run horizon-prepare") {
        found.push(Cause::OldGameFiles);
    }
    if text.contains("[Memory] applet type 2 ") {
        found.push(Cause::StartedFromAlbum);
    }
    if text
        .lines()
        .any(|l| l.contains("memory allocation of") && l.contains("failed"))
    {
        found.push(Cause::OutOfMemory);
    }
    if text.contains("expected value at line 1 column 1") {
        found.push(Cause::MacJunk);
    }
    found
}

/// What a bug report needs: the logs, the crash reports (the game's and Atmosphère's newest) and
/// `env.txt`. Never the account. Each with its name in the zip.
fn report_files(card: &SdCard) -> Vec<(PathBuf, String)> {
    let home = card.horizon();
    let mut paths = files(card);
    paths.extend(
        ["panic.txt", "env.txt"]
            .map(|n| home.join(n))
            .into_iter()
            .filter(|p| p.is_file()),
    );
    for dir in [
        home.join("crash-reports"),
        card.root.join("atmosphere/crash_reports"),
    ] {
        let mut reports: Vec<_> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !crate::sd::is_mac_junk(&e.file_name().to_string_lossy()))
            .filter_map(|e| {
                let meta = e.metadata().ok().filter(|m| m.is_file())?;
                Some((meta.modified().ok(), e.path()))
            })
            .collect();
        reports.sort();
        paths.extend(reports.into_iter().rev().take(5).map(|(_, p)| p));
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let name = p
                .strip_prefix(&home)
                .or_else(|_| p.strip_prefix(&card.root))
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some((p, name))
        })
        .collect()
}

/// Zips what a report needs into the Desktop (or home) folder, for a bug report; returns the file.
pub fn zip_for_report(card: &SdCard) -> Result<PathBuf, String> {
    let dir = dirs::desktop_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!(
        "Horizon Switch logs {}.zip",
        crate::saves::chrono_like_stamp()
    ));
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (log, name) in report_files(card) {
        let data = std::fs::read(&log).map_err(|e| format!("{name}: {e}"))?;
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_has_the_crash_files_and_never_the_account() {
        let root = std::env::temp_dir().join(format!("companion-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in [
            "switch/horizon/log.txt",
            "switch/horizon/panic.txt",
            "switch/horizon/account.json",
            "switch/horizon/crash-reports/crash-1-client.txt",
            "atmosphere/crash_reports/1_0100.log",
            "atmosphere/crash_reports/._1_0100.log",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        let card = SdCard::at(&root).unwrap();
        let names: Vec<String> = report_files(&card).into_iter().map(|f| f.1).collect();
        assert_eq!(
            names,
            [
                "log.txt",
                "panic.txt",
                "crash-reports/crash-1-client.txt",
                "atmosphere/crash_reports/1_0100.log"
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
