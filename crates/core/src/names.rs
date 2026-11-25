//! Non-ASCII names on the Switch's SD card. The console can't list them, so Horizon NX (2.2.0 and
//! later) keeps each as `xn--` and its Punycode (RFC 3492): the game shows `Мир`, the card holds
//! `xn--o0a0dya`. What the companion puts on the card has to be named the same way, or the game
//! never sees it. The encoding is horizon-nx's `vlinks/names.rs`.

use crate::sd::{SdCard, is_mac_junk};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

const PREFIX: &str = "xn--";

/// `name` (one file or folder) as the card holds it. Composed first: macOS may hand over `й` as
/// two characters, which the game would keep as another name.
pub fn on_card(name: &str) -> String {
    if name.is_ascii() {
        return name.to_owned();
    }
    let name: String = name.nfc().collect();
    if name.is_ascii() {
        return name;
    }
    match encode(&name) {
        Some(ascii) => format!("{PREFIX}{ascii}"),
        None => name,
    }
}

/// The name a card's `name` stands for (itself, unless [`on_card`] made it).
pub fn shown(name: &str) -> String {
    name.get(..PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(PREFIX))
        .and_then(|_| decode(&name[PREFIX.len()..]))
        .filter(|n| !n.is_ascii())
        .unwrap_or_else(|| name.to_owned())
}

/// What's in the player's folders under a name the Switch can't see, what's inside a folder
/// before the folder.
pub fn hidden(card: &SdCard) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in ["saves", "resourcepacks", "horizon-mods", "screenshots"] {
        collect(&card.horizon().join(dir), &mut found);
    }
    found
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_mac_junk(&name) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            collect(&entry.path(), found);
        }
        if !name.is_ascii() {
            found.push(entry.path());
        }
    }
}

/// Renames what [`hidden`] finds so the game sees it. Never replaces anything.
pub fn reveal_hidden(card: &SdCard) -> Result<String, String> {
    let found = hidden(card);
    for path in &found {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let target = free_name(path.parent().unwrap_or(path), &name);
        std::fs::rename(path, &target).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(format!(
        "Renamed {} files or folders for the Switch.",
        found.len()
    ))
}

/// Where `name` goes in the card's folder `dir`: numbered ("Мир (2)") while it's taken.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut target = dir.join(on_card(name));
    let mut n = 2;
    while target.exists() {
        target = dir.join(on_card(&format!("{stem} ({n}){extension}")));
        n += 1;
    }
    target
}

const BASE: u32 = 36;
const T_MIN: u32 = 1;
const T_MAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 128;

fn adapt(delta: u32, points: u32, first: bool) -> u32 {
    let mut delta = if first { delta / DAMP } else { delta / 2 };
    delta += delta / points;
    let mut k = 0;
    while delta > ((BASE - T_MIN) * T_MAX) / 2 {
        delta /= BASE - T_MIN;
        k += BASE;
    }
    k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
}

fn threshold(k: u32, bias: u32) -> u32 {
    if k <= bias {
        T_MIN
    } else if k >= bias + T_MAX {
        T_MAX
    } else {
        k - bias
    }
}

fn digit(d: u32) -> char {
    (if d < 26 {
        b'a' + d as u8
    } else {
        b'0' + (d - 26) as u8
    }) as char
}

/// RFC 3492's encoding of `input`.
fn encode(input: &str) -> Option<String> {
    let points: Vec<u32> = input.chars().map(u32::from).collect();
    let mut out: String = input.chars().filter(char::is_ascii).collect();
    let basic = out.len() as u32;
    let mut handled = basic;
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (INITIAL_N, 0u32, INITIAL_BIAS);
    let total = points.len() as u32;
    while handled < total {
        let m = *points.iter().filter(|&&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(handled + 1)?)?;
        n = m;
        for &c in &points {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = threshold(k, bias);
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta = delta.checked_add(1)?;
        n += 1;
    }
    Some(out)
}

/// RFC 3492's decoding; `None` for anything that isn't a valid encoding.
fn decode(input: &str) -> Option<String> {
    let (basic, rest) = match input.rfind('-') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None => ("", input),
    };
    if !basic.is_ascii() {
        return None;
    }
    let mut out: Vec<char> = basic.chars().collect();
    let (mut n, mut i, mut bias) = (INITIAL_N, 0u32, INITIAL_BIAS);
    let mut bytes = rest.bytes().peekable();
    while bytes.peek().is_some() {
        let old = i;
        let mut w = 1u32;
        let mut k = BASE;
        loop {
            let d = match bytes.next()? {
                b @ b'a'..=b'z' => b - b'a',
                b @ b'A'..=b'Z' => b - b'A',
                b @ b'0'..=b'9' => b - b'0' + 26,
                _ => return None,
            } as u32;
            i = i.checked_add(d.checked_mul(w)?)?;
            let t = threshold(k, bias);
            if d < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k += BASE;
        }
        let len = out.len() as u32 + 1;
        bias = adapt(i - old, len, old == 0);
        n = n.checked_add(i / len)?;
        i %= len;
        out.insert(i as usize, char::from_u32(n)?);
        i += 1;
    }
    Some(out.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forms horizon-nx's own test expects (from Python's `str.encode("punycode")`).
    #[test]
    fn names_go_to_the_card_as_the_game_keeps_them_and_come_back() {
        for (name, card) in [
            ("Мир", "xn--o0a0dya"),
            ("Новый мир (1)", "xn--  (1)-use1mzbh5as3a2l"),
            ("日本語のワールド", "xn--u9j3jrdpa9d7726bo4bc84j"),
            ("Café", "xn--Caf-dma"),
            ("😀 emoji", "xn-- emoji-2v74e"),
        ] {
            assert_eq!(on_card(name), card);
            assert_eq!(shown(card), name);
        }
        // macOS's decomposed form is the same name.
        assert_eq!(on_card("Cafe\u{301}"), "xn--Caf-dma");
        for plain in ["New World", "xn--abc-", "xn--!!", "é"] {
            assert_eq!(shown(plain), plain);
        }
    }

    #[test]
    fn hidden_names_are_revealed_inside_out_and_nothing_is_replaced() {
        let root = std::env::temp_dir().join(format!("companion-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let saves = root.join("switch/horizon/saves");
        std::fs::create_dir_all(saves.join("Мир/datapacks/Пак")).unwrap();
        std::fs::write(saves.join("Мир/level.dat"), b"computer").unwrap();
        std::fs::create_dir_all(saves.join("xn--o0a0dya")).unwrap();
        std::fs::write(saves.join("xn--o0a0dya/level.dat"), b"switch").unwrap();
        let card = SdCard::at(&root).unwrap();
        assert_eq!(hidden(&card).len(), 2);
        reveal_hidden(&card).unwrap();
        assert!(hidden(&card).is_empty());
        let kept = std::fs::read(saves.join("xn--o0a0dya/level.dat")).unwrap();
        assert_eq!(kept, b"switch");
        let moved = saves.join(on_card("Мир (2)"));
        assert_eq!(std::fs::read(moved.join("level.dat")).unwrap(), b"computer");
        assert!(moved.join("datapacks").join(on_card("Пак")).is_dir());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
