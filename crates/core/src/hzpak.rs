//! `resources.hzpak`: the game's resources folder as one file, in the exact format the game's
//! SD card layer reads (horizon-nx `crates/horizon-nx/src/vlinks/pack.rs`, which this copies).
//!
//! On the Switch every open of a file costs about 10 ms and the resources are 28,000 files, so the
//! game reads them from this pack; it packs them itself on its first start, which takes it about
//! 20 minutes. Built here, the pack is ready from the first start.
//!
//! Layout: `HZPAK001`, the packed files' bytes one after another, the index, then a footer
//! (index offset u64, index length u64, `HZPAKEND`). The index lists every file and folder (path,
//! kind, offset, size, modification time) and the folder's top level (names, kinds, sizes): the
//! game uses the pack only while the folder's top level still matches.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Files larger than this stay in the folder (the game's `MAX_PACKED`).
pub const MAX_PACKED: u64 = 8 << 20;
const MAGIC: &[u8; 8] = b"HZPAK001";
const END: &[u8; 8] = b"HZPAKEND";
const FOOTER: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    /// Its bytes are in the pack at this offset.
    Packed(u64),
    /// Too large to pack: read from the folder.
    Loose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub size: u64,
    /// Seconds since 1970.
    pub mtime: i64,
}

/// One entry of the folder's top level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Top {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// `<folder>.hzpak`.
pub fn pack_path(folder: &Path) -> PathBuf {
    let mut name = folder.as_os_str().to_os_string();
    name.push(".hzpak");
    PathBuf::from(name)
}

/// `<folder>.hzpak.changed`: what the game changed under the folder since the pack was built.
pub fn changes_path(folder: &Path) -> PathBuf {
    let mut name = pack_path(folder).into_os_string();
    name.push(".changed");
    PathBuf::from(name)
}

/// Writes a pack: `begin`, `file` for each file of up to [`MAX_PACKED`] bytes (in any order),
/// `entry` for folders and larger files, then `finish` with the top level.
pub struct Writer<W: Write> {
    out: W,
    offset: u64,
    entries: Vec<(String, Entry)>,
}

impl<W: Write> Writer<W> {
    pub fn begin(mut out: W, root_mtime: i64) -> io::Result<Self> {
        out.write_all(MAGIC)?;
        Ok(Writer {
            out,
            offset: MAGIC.len() as u64,
            entries: vec![(
                String::new(),
                Entry {
                    kind: Kind::Dir,
                    size: 0,
                    mtime: root_mtime,
                },
            )],
        })
    }

    /// A packed file's bytes; `rel` uses `/`.
    pub fn file(&mut self, rel: &str, data: &[u8], mtime: i64) -> io::Result<()> {
        self.out.write_all(data)?;
        let at = self.offset;
        self.offset += data.len() as u64;
        self.entries.push((
            rel.to_owned(),
            Entry {
                kind: Kind::Packed(at),
                size: data.len() as u64,
                mtime,
            },
        ));
        Ok(())
    }

    /// A folder or a file left in the folder.
    pub fn entry(&mut self, rel: &str, entry: Entry) {
        self.entries.push((rel.to_owned(), entry));
    }

    /// The index and footer; returns the writer.
    pub fn finish(mut self, top: &[Top]) -> io::Result<W> {
        let block = encode(&self.entries, top);
        self.out.write_all(&block)?;
        self.out.write_all(&self.offset.to_le_bytes())?;
        self.out.write_all(&(block.len() as u64).to_le_bytes())?;
        self.out.write_all(END)?;
        Ok(self.out)
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn encode(entries: &[(String, Entry)], top: &[Top]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (rel, e) in entries {
        put_str(&mut out, rel);
        let (kind, offset) = match e.kind {
            Kind::Dir => (0u8, 0),
            Kind::Packed(offset) => (1, offset),
            Kind::Loose => (2, 0),
        };
        out.push(kind);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&e.size.to_le_bytes());
        out.extend_from_slice(&e.mtime.to_le_bytes());
    }
    out.extend_from_slice(&(top.len() as u32).to_le_bytes());
    for t in top {
        put_str(&mut out, &t.name);
        out.push(u8::from(t.dir));
        out.extend_from_slice(&t.size.to_le_bytes());
    }
    out
}

/// A pack read back: its entries and top level.
#[derive(Debug)]
pub struct Index {
    pub entries: Vec<(String, Entry)>,
    pub top: Vec<Top>,
}

/// Reads a pack's index (to check one: the game's own reader does the same).
pub fn read_index(path: &Path) -> io::Result<Index> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.seek(SeekFrom::End(0))?;
    if len < (MAGIC.len() + FOOTER) as u64 {
        return Err(bad("cut short"));
    }
    let mut tail = [0u8; FOOTER];
    file.seek(SeekFrom::Start(len - FOOTER as u64))?;
    file.read_exact(&mut tail)?;
    if &tail[16..] != END {
        return Err(bad("not a complete pack"));
    }
    let at = u64::from_le_bytes(tail[..8].try_into().unwrap());
    let n = u64::from_le_bytes(tail[8..16].try_into().unwrap());
    if at.checked_add(n).is_none_or(|end| end > len) {
        return Err(bad("index out of range"));
    }
    let mut block = vec![0u8; n as usize];
    file.seek(SeekFrom::Start(at))?;
    file.read_exact(&mut block)?;
    decode(&block)
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("pack: {why}"))
}

fn decode(block: &[u8]) -> io::Result<Index> {
    let mut at = 0usize;
    let get = |at: &mut usize, n: usize| -> io::Result<u64> {
        let b = block
            .get(*at..*at + n)
            .ok_or_else(|| bad("index cut short"))?;
        *at += n;
        let mut v = [0u8; 8];
        v[..n].copy_from_slice(b);
        Ok(u64::from_le_bytes(v))
    };
    let count = get(&mut at, 4)? as usize;
    let mut entries = Vec::with_capacity(count);
    let string = |at: &mut usize| -> io::Result<String> {
        let len = get(at, 2)? as usize;
        let b = block
            .get(*at..*at + len)
            .ok_or_else(|| bad("index cut short"))?;
        *at += len;
        String::from_utf8(b.to_vec()).map_err(|_| bad("name"))
    };
    for _ in 0..count {
        let rel = string(&mut at)?;
        let kind = get(&mut at, 1)?;
        let offset = get(&mut at, 8)?;
        let size = get(&mut at, 8)?;
        let mtime = get(&mut at, 8)? as i64;
        let kind = match kind {
            0 => Kind::Dir,
            1 => Kind::Packed(offset),
            2 => Kind::Loose,
            _ => return Err(bad("kind")),
        };
        entries.push((rel, Entry { kind, size, mtime }));
    }
    let tops = get(&mut at, 4)? as usize;
    let mut top = Vec::with_capacity(tops);
    for _ in 0..tops {
        let name = string(&mut at)?;
        let dir = get(&mut at, 1)? != 0;
        let size = get(&mut at, 8)?;
        top.push(Top { name, dir, size });
    }
    Ok(Index { entries, top })
}

/// The folder's top level as the game sees it on the card (one listing).
pub fn top_level(folder: &Path) -> io::Result<Vec<Top>> {
    let mut top = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        top.push(Top {
            name: entry.file_name().to_string_lossy().into_owned(),
            dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    Ok(top)
}

/// Whether a pack's top level is the folder's now (the game's `Index::fits`, with nothing
/// changed by the game yet).
pub fn fits(packed: &[Top], now: &[Top]) -> bool {
    let mut a: Vec<&Top> = packed.iter().collect();
    let mut b: Vec<&Top> = now.iter().collect();
    a.sort_by(|x, y| x.name.cmp(&y.name));
    b.sort_by(|x, y| x.name.cmp(&y.name));
    a == b
}

/// Seconds since 1970 of a file's modification time.
pub fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}
