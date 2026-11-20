//! Just enough of Minecraft's NBT for `servers.dat`: read a file into tags and write it back
//! unchanged (uncompressed, big-endian, a named root compound).

use std::io::{self, Read};

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    /// The element type (an empty list keeps it), and the elements.
    List(u8, Vec<Tag>),
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        match self {
            Tag::Compound(fields) => fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }
}

/// The root compound's name and value.
pub fn read(bytes: &[u8]) -> io::Result<(String, Tag)> {
    let mut r = bytes;
    let id = byte(&mut r)?;
    if id != 10 {
        return Err(bad("the root isn't a compound"));
    }
    let name = string(&mut r)?;
    let tag = payload(&mut r, id, 0)?;
    Ok((name, tag))
}

pub fn write(name: &str, tag: &Tag) -> Vec<u8> {
    let mut out = vec![tag.id()];
    put_string(&mut out, name);
    put_payload(&mut out, tag);
    out
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn take<const N: usize>(r: &mut &[u8]) -> io::Result<[u8; N]> {
    let mut b = [0; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn byte(r: &mut &[u8]) -> io::Result<u8> {
    Ok(take::<1>(r)?[0])
}

fn length(r: &mut &[u8]) -> io::Result<usize> {
    let n = i32::from_be_bytes(take(r)?);
    // No list in a server list comes near this; a bad file must not allocate gigabytes.
    if !(0..=1 << 24).contains(&n) {
        return Err(bad("a length is out of range"));
    }
    Ok(n as usize)
}

fn string(r: &mut &[u8]) -> io::Result<String> {
    let n = u16::from_be_bytes(take(r)?) as usize;
    if r.len() < n {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let (s, rest) = r.split_at(n);
    *r = rest;
    // Java's modified UTF-8 is UTF-8 for everything a server list holds.
    Ok(String::from_utf8_lossy(s).into_owned())
}

fn payload(r: &mut &[u8], id: u8, depth: u32) -> io::Result<Tag> {
    if depth > 64 {
        return Err(bad("nested too deep"));
    }
    Ok(match id {
        1 => Tag::Byte(byte(r)? as i8),
        2 => Tag::Short(i16::from_be_bytes(take(r)?)),
        3 => Tag::Int(i32::from_be_bytes(take(r)?)),
        4 => Tag::Long(i64::from_be_bytes(take(r)?)),
        5 => Tag::Float(f32::from_be_bytes(take(r)?)),
        6 => Tag::Double(f64::from_be_bytes(take(r)?)),
        7 => Tag::ByteArray(
            (0..length(r)?)
                .map(|_| byte(r).map(|b| b as i8))
                .collect::<io::Result<_>>()?,
        ),
        8 => Tag::String(string(r)?),
        9 => {
            let element = byte(r)?;
            let n = length(r)?;
            let items = (0..n)
                .map(|_| payload(r, element, depth + 1))
                .collect::<io::Result<_>>()?;
            Tag::List(element, items)
        }
        10 => {
            let mut fields = Vec::new();
            loop {
                let id = byte(r)?;
                if id == 0 {
                    break;
                }
                let key = string(r)?;
                fields.push((key, payload(r, id, depth + 1)?));
            }
            Tag::Compound(fields)
        }
        11 => Tag::IntArray(
            (0..length(r)?)
                .map(|_| take(r).map(i32::from_be_bytes))
                .collect::<io::Result<_>>()?,
        ),
        12 => Tag::LongArray(
            (0..length(r)?)
                .map(|_| take(r).map(i64::from_be_bytes))
                .collect::<io::Result<_>>()?,
        ),
        _ => return Err(bad("an unknown tag")),
    })
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&bytes[..n]);
}

fn put_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            out.extend(v.iter().map(|b| *b as u8));
        }
        Tag::String(s) => put_string(out, s),
        Tag::List(element, items) => {
            let element = items.first().map_or(*element, Tag::id);
            out.push(element);
            out.extend_from_slice(&(items.len() as i32).to_be_bytes());
            for item in items {
                put_payload(out, item);
            }
        }
        Tag::Compound(fields) => {
            for (key, value) in fields {
                out.push(value.id());
                put_string(out, key);
                put_payload(out, value);
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_list_reads_and_writes_back_the_same_bytes() {
        let servers = Tag::Compound(vec![(
            "servers".into(),
            Tag::List(
                10,
                vec![Tag::Compound(vec![
                    ("name".into(), Tag::String("Hypixel".into())),
                    ("ip".into(), Tag::String("mc.hypixel.net".into())),
                    ("acceptTextures".into(), Tag::Byte(1)),
                ])],
            ),
        )]);
        let bytes = write("", &servers);
        let (name, tag) = read(&bytes).unwrap();
        assert_eq!(name, "");
        assert_eq!(tag, servers);
        assert_eq!(write(&name, &tag), bytes);
    }
}
