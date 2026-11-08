//! The cover picture an mp3 carries in its ID3v2 tag, as the image file's
//! bytes (jpeg or png). Read only, never written.

use std::fs::File;
use std::io::Read;
use std::path::Path;

fn syncsafe(b: &[u8]) -> usize {
    b.iter().fold(0, |s, x| (s << 7) | (*x & 0x7f) as usize)
}

fn be(b: &[u8]) -> usize {
    b.iter().fold(0, |s, x| (s << 8) | *x as usize)
}

/// Undo ID3 unsynchronisation: every 0xFF 0x00 was written for a bare 0xFF.
fn unsync(b: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        o.push(b[i]);
        i += if b[i] == 0xFF && b.get(i + 1) == Some(&0) { 2 } else { 1 };
    }
    o
}

pub fn read(path: &Path) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    let mut h = [0u8; 10];
    f.read_exact(&mut h).ok()?;
    if &h[..3] != b"ID3" {
        return None;
    }
    let (ver, flags, size) = (h[3], h[5], syncsafe(&h[6..10]));
    if size > 64 << 20 {
        return None;
    }
    let mut tag = vec![0; size];
    f.read_exact(&mut tag).ok()?;
    if flags & 0x80 != 0 && ver < 4 {
        tag = unsync(&tag);
    }
    let mut i = 0;
    if flags & 0x40 != 0 && tag.len() >= 4 {
        i = if ver >= 4 { syncsafe(&tag[..4]) } else { be(&tag[..4]) + 4 };
    }
    let (id_len, head) = if ver == 2 { (3, 6) } else { (4, 10) };
    while i + head <= tag.len() && tag[i] != 0 {
        let id = &tag[i..i + id_len];
        let n = match ver {
            2 => be(&tag[i + 3..i + 6]),
            3 => be(&tag[i + 4..i + 8]),
            _ => syncsafe(&tag[i + 4..i + 8]),
        };
        let fmt = if ver >= 3 { tag[i + 9] } else { 0 };
        let body = tag.get(i + head..i + head + n)?;
        if (id == b"APIC" || id == b"PIC")
            && let Some(d) = picture(body, ver, fmt)
        {
            return Some(d);
        }
        i += head + n;
    }
    None
}

fn picture(body: &[u8], ver: u8, fmt: u8) -> Option<Vec<u8>> {
    let mut b = body.to_vec();
    match ver {
        // 0x08 compressed, 0x04 encrypted, 0x01 data length prefix, 0x02 unsynchronised
        4 => {
            if fmt & 0x0C != 0 {
                return None;
            }
            if fmt & 0x01 != 0 {
                b = b.get(4..)?.to_vec();
            }
            if fmt & 0x02 != 0 {
                b = unsync(&b);
            }
        }
        // 0x80 compressed, 0x40 encrypted, 0x20 a group byte
        3 => {
            if fmt & 0xC0 != 0 {
                return None;
            }
            if fmt & 0x20 != 0 {
                b = b.get(1..)?.to_vec();
            }
        }
        _ => {}
    }
    let enc = *b.first()?;
    // the mime type ("image/jpeg\0"), or a three letter format in v2.2
    let mut i = if ver == 2 { 4 } else { 1 + b.get(1..)?.iter().position(|x| *x == 0)? + 1 };
    i += 1; // picture type
    // the description, ended by one zero byte, or two in UTF-16
    if enc == 1 || enc == 2 {
        while b.get(i..i + 2)? != [0, 0] {
            i += 2;
        }
        i += 2;
    } else {
        i += b.get(i..)?.iter().position(|x| *x == 0)? + 1;
    }
    let d = b.get(i..)?;
    (d.starts_with(&[0xFF, 0xD8]) || d.starts_with(b"\x89PNG")).then(|| d.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(ver: u8, frame: &[u8]) -> Vec<u8> {
        let n = frame.len();
        let ss = |n: usize| [(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f];
        let mut v = b"ID3".to_vec();
        v.extend([ver, 0, 0]);
        v.extend(ss(n + 10));
        v.extend(b"APIC");
        v.extend(if ver == 4 { ss(n) } else { (n as u32).to_be_bytes() });
        v.extend([0, 0]);
        v.extend(frame);
        v
    }

    #[test]
    fn finds_the_jpeg_in_v3_and_v4() {
        let mut frame = vec![0];
        frame.extend(b"image/jpeg\0");
        frame.push(3);
        frame.extend(b"cover\0");
        frame.extend([0xFF, 0xD8, 1, 2, 3]);
        for ver in [3, 4] {
            let p = std::env::temp_dir().join(format!("vibecheck-cover-{ver}-{}.mp3", std::process::id()));
            std::fs::write(&p, tag(ver, &frame)).unwrap();
            assert_eq!(read(&p), Some(vec![0xFF, 0xD8, 1, 2, 3]));
            std::fs::remove_file(&p).unwrap();
        }
    }

    #[test]
    fn skips_a_utf16_description() {
        let mut frame = vec![1];
        frame.extend(b"image/png\0");
        frame.push(3);
        frame.extend([0xFF, 0xFE, b'a', 0, 0, 0]);
        frame.extend(b"\x89PNGxx");
        let p = std::env::temp_dir().join(format!("vibecheck-cover-u-{}.mp3", std::process::id()));
        std::fs::write(&p, tag(3, &frame)).unwrap();
        assert_eq!(read(&p), Some(b"\x89PNGxx".to_vec()));
        std::fs::remove_file(&p).unwrap();
    }
}
