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

/// Zips every log into the Desktop (or home) folder, for a bug report; returns the file.
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
    for log in files(card) {
        let name = log
            .strip_prefix(card.horizon())
            .map_or_else(|_| log.display().to_string(), |p| p.display().to_string());
        let data = std::fs::read(&log).map_err(|e| format!("{name}: {e}"))?;
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(path)
}
