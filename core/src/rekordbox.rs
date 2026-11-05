//! The rekordbox xml: read colours, ratings, grids and cues; write predictions
//! back with one playlist holding them.
//!
//! Rekordbox watches one xml file and re-reads it on refresh, so this rewrites
//! that file in place, keeping a .bak. The rewrite is a stream over the file's
//! own tags: every tag it does not change is copied byte for byte.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use crate::palette;
use crate::store::{self, Label};
use crate::{Res, now};

// --- a tokenizer for the xml rekordbox writes -------------------------------

#[derive(Debug, Clone, Copy)]
enum Tok<'a> {
    Text(&'a str),
    Tag(&'a str),
}

fn tokens(s: &str) -> Res<Vec<Tok<'_>>> {
    let b = s.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        if b[i] != b'<' {
            let j = s[i..].find('<').map_or(b.len(), |k| i + k);
            out.push(Tok::Text(&s[i..j]));
            i = j;
            continue;
        }
        if s[i..].starts_with("<!--") {
            let j = s[i..].find("-->").ok_or("unterminated comment")? + i + 3;
            out.push(Tok::Tag(&s[i..j]));
            i = j;
            continue;
        }
        // a tag ends at the first '>' outside quotes
        let (mut j, mut q) = (i + 1, 0u8);
        while j < b.len() {
            match (q, b[j]) {
                (0, b'"') | (0, b'\'') => q = b[j],
                (0, b'>') => break,
                (c, x) if c != 0 && x == c => q = 0,
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() {
            return Err("unterminated tag".into());
        }
        out.push(Tok::Tag(&s[i..=j]));
        i = j + 1;
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct Tag<'a> {
    name: &'a str,
    /// unescaped values, in the file's order
    attrs: Vec<(&'a str, Cow<'a, str>)>,
    close: bool,
    empty: bool,
}

impl<'a> Tag<'a> {
    fn get(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_ref())
    }
    fn set(&mut self, k: &'a str, v: String) {
        match self.attrs.iter_mut().find(|a| a.0 == k) {
            Some(a) => a.1 = Cow::Owned(v),
            None => self.attrs.push((k, Cow::Owned(v))),
        }
    }
    fn map(&self) -> Vec<(String, String)> {
        self.attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
}

fn parse_tag(raw: &str) -> Option<Tag<'_>> {
    let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
    if inner.starts_with('?') || inner.starts_with('!') {
        return None;
    }
    let (close, inner) = match inner.strip_prefix('/') {
        Some(r) => (true, r),
        None => (false, inner),
    };
    let (empty, inner) = match inner.strip_suffix('/') {
        Some(r) => (true, r),
        None => (false, inner),
    };
    let inner = inner.trim();
    let n = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
    let (name, mut rest) = (&inner[..n], &inner[n..]);
    let mut attrs = vec![];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let eq = rest.find('=')?;
        let k = rest[..eq].trim();
        rest = rest[eq + 1..].trim_start();
        let q = rest.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = rest[1..].find(q)? + 1;
        attrs.push((k, unescape(&rest[1..end])));
        rest = &rest[end + 1..];
    }
    Some(Tag { name, attrs, close, empty })
}

fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';') else { break };
        let ent = &rest[1..j];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn escape_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            '\t' => o.push_str("&#09;"),
            c => o.push(c),
        }
    }
    o
}

fn write_tag(name: &str, attrs: &[(impl AsRef<str>, impl AsRef<str>)], empty: bool) -> String {
    let mut s = format!("<{name}");
    for (k, v) in attrs {
        s.push_str(&format!(" {}=\"{}\"", k.as_ref(), escape_attr(v.as_ref())));
    }
    s.push_str(if empty { " />" } else { ">" });
    s
}

// --- reading -------------------------------------------------------------------

/// The track's path relative to the collection, NFC; None if outside it.
fn rel_of(location: &str, collection: &Path) -> Option<String> {
    let loc = location.replace("file://localhost", "");
    let loc = percent_encoding::percent_decode_str(&loc).decode_utf8_lossy();
    let p = Path::new(loc.as_ref());
    let rel = p.strip_prefix(collection).ok()?;
    Some(store::norm(&rel.to_string_lossy()))
}

/// A track as the xml has it: grid, length, cues, colour and rating.
#[derive(Debug, Clone, Default)]
pub struct Track {
    pub track_id: String,
    /// (start s, bpm, beat in bar 1-4)
    pub tempos: Vec<(f64, f64, i64)>,
    pub duration: f64,
    /// POSITION_MARK attributes, as strings, in order
    pub marks: Vec<Vec<(String, String)>>,
    pub label: Option<Label>,
}

/// Every collection track inside `collection`, by NFC relative path.
pub fn read(xml: &Path, collection: &Path) -> Res<HashMap<String, Track>> {
    let s = std::fs::read_to_string(xml)?;
    let toks = tokens(&s)?;
    let (mut out, mut in_coll, mut cur): (HashMap<String, Track>, bool, Option<(String, Track)>) =
        (HashMap::new(), false, None);
    for t in toks {
        let Tok::Tag(raw) = t else { continue };
        let Some(tag) = parse_tag(raw) else { continue };
        match (tag.name, tag.close) {
            ("COLLECTION", false) => in_coll = !tag.empty,
            ("COLLECTION", true) => in_coll = false,
            ("TRACK", false) if in_coll => {
                let rel = tag.get("Location").and_then(|l| rel_of(l, collection));
                let colour = tag.get("Colour").and_then(palette::by_rgb);
                let tr = Track {
                    track_id: tag.get("TrackID").unwrap_or("").to_string(),
                    duration: tag.get("TotalTime").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    label: Some(Label { colour, stars: Some(palette::stars_from_rating(tag.get("Rating"))) }),
                    ..Default::default()
                };
                match (rel, tag.empty) {
                    (Some(r), true) => {
                        out.insert(r, tr);
                    }
                    (Some(r), false) => cur = Some((r, tr)),
                    (None, _) => cur = None,
                }
            }
            ("TRACK", true) if in_coll => {
                if let Some((r, tr)) = cur.take() {
                    out.insert(r, tr);
                }
            }
            ("TEMPO", false) => {
                if let Some((_, tr)) = cur.as_mut() {
                    let f = |k| tag.get(k).and_then(|v| v.parse::<f64>().ok());
                    if let (Some(a), Some(b), Some(c)) = (f("Inizio"), f("Bpm"), f("Battito")) {
                        tr.tempos.push((a, b, c as i64));
                    }
                }
            }
            ("POSITION_MARK", false) => {
                if let Some((_, tr)) = cur.as_mut() {
                    tr.marks.push(tag.map());
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

// --- writing -------------------------------------------------------------------

/// One track's worth of writing. None anywhere means "leave that alone".
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub rel: String,
    /// how far the colour got; orders the playlist
    pub level: &'static str,
    pub label: Option<Label>,
    /// alphanumeric, "8A"
    pub key: Option<String>,
    pub energy: Option<u8>,
    /// every POSITION_MARK the track should have
    pub cues: Option<Vec<Vec<(String, String)>>>,
}

#[derive(Debug, Default)]
pub struct WriteStats {
    pub coloured: usize,
    pub rated: usize,
    pub keyed: usize,
    pub commented: usize,
    pub cued: usize,
    pub not_in_xml: usize,
    pub unchanged: usize,
    /// tracks per level in the playlist
    pub playlists: HashMap<&'static str, usize>,
    pub backup: Option<std::path::PathBuf>,
    pub written: bool,
}

/// The comment with its "<key> - Energy <n>" prefix replaced, the rest kept:
/// "5A - Energy 7 /* Atmo */" -> "8A - Energy 6 /* Atmo */".
pub fn comment_with(old: Option<&str>, key: &str, energy: u8) -> String {
    let rest = strip_key_energy(old.unwrap_or("")).trim().to_string();
    let head = format!("{key} - Energy {energy}");
    if rest.is_empty() { head } else { format!("{head} {rest}") }
}

/// Drops a leading `\s*\d{1,2}[AB]\s*-\s*Energy\s*\d{1,2}\b\s*`, whoever wrote it.
fn strip_key_energy(s: &str) -> &str {
    let t = s.trim_start();
    let b = t.as_bytes();
    let digits = |i: usize| (i..b.len()).take_while(|&j| b[j].is_ascii_digit()).count();
    let mut i = 0;
    let n = digits(i);
    if !(1..=2).contains(&n) {
        return s;
    }
    i += n;
    if i >= b.len() || !(b[i] == b'A' || b[i] == b'B') {
        return s;
    }
    i += 1;
    let ws = |i: usize| i + t[i..].len() - t[i..].trim_start().len();
    i = ws(i);
    if !t[i..].starts_with('-') {
        return s;
    }
    i = ws(i + 1);
    if !t[i..].starts_with("Energy") {
        return s;
    }
    i = ws(i + 6);
    let n = digits(i);
    if !(1..=2).contains(&n) {
        return s;
    }
    i += n;
    // \b: the number must end at a word boundary
    if i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        return s;
    }
    &t[ws(i)..]
}

fn marks_equal(a: &[Vec<(String, String)>], b: &[Vec<(String, String)>]) -> bool {
    let norm = |m: &Vec<(String, String)>| {
        let mut v = m.clone();
        v.sort();
        v
    };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| norm(x) == norm(y))
}

pub struct WriteOpts<'a> {
    pub name: &'a str,
    pub tonality: bool,
    pub comment: bool,
    /// keep the playlist to tracks something actually changed on
    pub changed_only: bool,
}

/// Apply entries and add one playlist holding them. Writes `xml_out` (and a
/// .bak when that is `xml_in`), unless `changed_only` finds nothing to change.
pub fn write(xml_in: &Path, xml_out: &Path, collection: &Path, entries: &[Entry], o: &WriteOpts) -> Res<WriteStats> {
    let src = std::fs::read_to_string(xml_in)?;
    let toks = tokens(&src)?;
    let by_rel: HashMap<String, usize> = entries.iter().enumerate().map(|(i, e)| (store::norm(&e.rel), i)).collect();
    let mut st = WriteStats::default();
    let mut seen = vec![false; entries.len()];
    // (entry index, TrackID, level) for the playlist
    let mut picked: Vec<(usize, String, &'static str)> = vec![];
    let mut out = String::with_capacity(src.len() + 4096);
    let mut i = 0;
    let mut in_coll = false;
    let mut changed_any = false;

    while i < toks.len() {
        let tok = toks[i];
        let Tok::Tag(raw) = tok else {
            if let Tok::Text(t) = tok {
                out.push_str(t);
            }
            i += 1;
            continue;
        };
        let tag = parse_tag(raw);
        let Some(mut tag) = tag else {
            out.push_str(raw);
            i += 1;
            continue;
        };
        if tag.name == "COLLECTION" {
            in_coll = !tag.close && !tag.empty;
        }
        if !(in_coll && tag.name == "TRACK" && !tag.close) {
            if tag.name == "PLAYLISTS" && !tag.close && !tag.empty {
                // the root NODE: copy it, then add ours and fix its Count
                out.push_str(raw);
                i += 1;
                i = playlists(&toks, i, &mut out, &mut picked, entries, o, &mut st, &mut changed_any)?;
                continue;
            }
            out.push_str(raw);
            i += 1;
            continue;
        }
        // a collection track: find its extent
        let start = i;
        let mut end = i;
        if !tag.empty {
            let mut j = i + 1;
            loop {
                match toks.get(j) {
                    Some(Tok::Tag(r)) if parse_tag(r).is_some_and(|t| t.name == "TRACK" && t.close) => break,
                    Some(_) => j += 1,
                    None => return Err("unterminated TRACK".into()),
                }
            }
            end = j;
        }
        let rel = tag.get("Location").and_then(|l| rel_of(l, collection));
        let ei = rel.as_ref().and_then(|r| by_rel.get(r)).copied();
        let Some(ei) = ei else {
            for t in &toks[start..=end] {
                out.push_str(match t {
                    Tok::Text(s) | Tok::Tag(s) => s,
                });
            }
            i = end + 1;
            continue;
        };
        seen[ei] = true;
        let e = &entries[ei];
        let mut changed = false;
        let put = |tag: &mut Tag, k: &'static str, v: String, n: &mut usize| {
            if tag.get(k) == Some(v.as_str()) {
                return false;
            }
            tag.set(k, v);
            *n += 1;
            true
        };
        if let Some(c) = e.label.and_then(|l| l.colour) {
            changed |= put(&mut tag, "Colour", c.rgb.to_string(), &mut st.coloured);
        }
        if let Some(s) = e.label.and_then(|l| l.stars).filter(|s| *s > 0) {
            changed |= put(&mut tag, "Rating", palette::rating_from_stars(s), &mut st.rated);
        }
        if let (Some(k), true) = (&e.key, o.tonality) {
            changed |= put(&mut tag, "Tonality", k.clone(), &mut st.keyed);
        }
        if let (Some(k), Some(en), true) = (&e.key, e.energy, o.comment) {
            let v = comment_with(tag.get("Comments"), k, en);
            changed |= put(&mut tag, "Comments", v, &mut st.commented);
        }
        // children, with POSITION_MARKs replaced if the cues differ
        let children = &toks[(start + 1).min(end)..end];
        let old: Vec<Vec<(String, String)>> = children
            .iter()
            .filter_map(|t| if let Tok::Tag(r) = t { parse_tag(r) } else { None })
            .filter(|t| t.name == "POSITION_MARK" && !t.close)
            .map(|t| t.map())
            .collect();
        let new_cues = e.cues.as_ref().filter(|c| !marks_equal(&old, c));
        if new_cues.is_some() {
            st.cued += 1;
            changed = true;
        }
        let head = if tag.attrs.iter().any(|a| matches!(a.1, Cow::Owned(_))) || (tag.empty && new_cues.is_some()) {
            write_tag(tag.name, &tag.attrs, tag.empty && new_cues.is_none())
        } else {
            raw.to_string()
        };
        out.push_str(&head);
        match new_cues {
            None => {
                for t in children.iter().chain(if tag.empty { &[][..] } else { &toks[end..=end] }) {
                    out.push_str(match t {
                        Tok::Text(s) | Tok::Tag(s) => s,
                    });
                }
            }
            Some(cues) => {
                // indentation: the text before the track's first child, or
                // the track's own plus two spaces
                let own = out[..out.len() - head.len()].rsplit('\n').next().unwrap_or("").to_string();
                let inner = children
                    .iter()
                    .find_map(|t| if let Tok::Text(s) = t { Some(s.to_string()) } else { None })
                    .unwrap_or_else(|| format!("\n{own}  "));
                let mut k = 0;
                while k < children.len() {
                    let t = children[k];
                    let is_mark = matches!(t, Tok::Tag(r) if parse_tag(r).is_some_and(|x| x.name == "POSITION_MARK"));
                    let next_is_mark = matches!(children.get(k + 1), Some(Tok::Tag(r)) if parse_tag(r).is_some_and(|x| x.name == "POSITION_MARK"));
                    if is_mark || (matches!(t, Tok::Text(_)) && next_is_mark) {
                        k += 1;
                        continue;
                    }
                    let (Tok::Text(s) | Tok::Tag(s)) = t;
                    // the whitespace before the closing tag is written after the new marks
                    if !(matches!(t, Tok::Text(_)) && k + 1 == children.len()) {
                        out.push_str(s);
                    }
                    k += 1;
                }
                for m in cues {
                    out.push_str(&inner);
                    out.push_str(&write_tag("POSITION_MARK", m, true));
                }
                out.push_str(&format!("\n{own}"));
                out.push_str("</TRACK>");
            }
        }
        if o.changed_only && !changed {
            st.unchanged += 1;
        } else {
            picked.push((ei, tag.get("TrackID").unwrap_or("").to_string(), e.level));
            changed_any |= changed;
        }
        i = end + 1;
    }
    st.not_in_xml = seen.iter().filter(|s| !**s).count();
    if o.changed_only && !changed_any {
        return Ok(st); // nothing to import: leave the file alone
    }
    if xml_out == xml_in {
        let backup = xml_in.with_extension("xml.bak");
        std::fs::copy(xml_in, &backup)?;
        st.backup = Some(backup);
    }
    let tmp = xml_out.with_extension("xml.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, xml_out)?;
    st.written = true;
    Ok(st)
}

/// Copies the PLAYLISTS root NODE and its children, then adds the batch
/// playlist as its last child and updates its Count. Returns the index after
/// the root NODE's end.
#[allow(clippy::too_many_arguments)]
fn playlists(
    toks: &[Tok],
    mut i: usize,
    out: &mut String,
    picked: &mut Vec<(usize, String, &'static str)>,
    _entries: &[Entry],
    o: &WriteOpts,
    st: &mut WriteStats,
    _changed: &mut bool,
) -> Res<usize> {
    // text up to the root NODE
    while let Some(Tok::Text(t)) = toks.get(i) {
        out.push_str(t);
        i += 1;
    }
    let Some(Tok::Tag(raw)) = toks.get(i) else { return Ok(i) };
    let Some(mut root) = parse_tag(raw).filter(|t| t.name == "NODE") else { return Ok(i) };
    // the playlist, ordered by how specific the prediction was, then as given
    picked.sort_by_key(|p| p.0);
    let mut keys = vec![];
    for level in ["colour", "hue", "tone", "none"] {
        for (_, id, l) in picked.iter() {
            if *l == level {
                keys.push(id.clone());
            }
        }
        let n = picked.iter().filter(|p| p.2 == level).count();
        if n > 0 {
            st.playlists.insert(level, n);
        }
    }
    let own_indent = out.rsplit('\n').next().unwrap_or("").to_string();
    let mut node = format!(
        "{}",
        write_tag(
            "NODE",
            &[("Name", o.name.to_string()), ("Type", "1".into()), ("KeyType", "0".into()), ("Entries", keys.len().to_string())],
            keys.is_empty()
        )
    );
    if !keys.is_empty() {
        for k in &keys {
            node.push_str(&format!("\n{own_indent}    {}", write_tag("TRACK", &[("Key", k.as_str())], true)));
        }
        node.push_str(&format!("\n{own_indent}  </NODE>"));
    }
    if root.empty {
        root.set("Count", "1".into());
        out.push_str(&write_tag("NODE", &root.attrs, false));
        out.push_str(&format!("\n{own_indent}  {node}\n{own_indent}</NODE>"));
        return Ok(i + 1);
    }
    // count direct children, copying them through
    let mut depth = 0;
    let mut kids = 0;
    let mut body = String::new();
    let mut j = i + 1;
    loop {
        let Some(t) = toks.get(j) else { return Err("unterminated playlist NODE".into()) };
        match t {
            Tok::Text(s) => body.push_str(s),
            Tok::Tag(r) => {
                if let Some(x) = parse_tag(r).filter(|x| x.name == "NODE") {
                    if x.close {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    } else {
                        if depth == 0 {
                            kids += 1;
                        }
                        if !x.empty {
                            depth += 1;
                        }
                    }
                }
                body.push_str(r);
            }
        }
        j += 1;
    }
    root.set("Count", (kids + 1).to_string());
    out.push_str(&write_tag("NODE", &root.attrs, false));
    // the batch goes after the last child, before the whitespace that closes the root
    let trimmed = body.trim_end_matches([' ', '\t', '\n', '\r']);
    let tail = &body[trimmed.len()..];
    out.push_str(trimmed);
    out.push_str(&format!("\n{own_indent}  {node}"));
    out.push_str(if tail.is_empty() { "" } else { tail });
    if tail.is_empty() {
        out.push_str(&format!("\n{own_indent}"));
    }
    out.push_str("</NODE>");
    Ok(j + 1)
}

/// When the xml was exported, for telling the user how fresh it is.
pub fn exported(xml: &Path) -> Option<i64> {
    let m = std::fs::metadata(xml).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64).filter(|t| *t <= now() + 86400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_energy_prefix_is_replaced_and_the_rest_kept() {
        assert_eq!(comment_with(Some("5A - Energy 7 /* Atmo / Rytm */"), "8A", 6), "8A - Energy 6 /* Atmo / Rytm */");
        assert_eq!(comment_with(Some("  12B-Energy 10"), "1A", 3), "1A - Energy 3");
        assert_eq!(comment_with(None, "1A", 3), "1A - Energy 3");
        assert_eq!(comment_with(Some("great track"), "2B", 5), "2B - Energy 5 great track");
        assert_eq!(comment_with(Some("5A - Energy 7x"), "8A", 6), "8A - Energy 6 5A - Energy 7x");
    }

    #[test]
    fn entities_round_trip() {
        assert_eq!(unescape("a &amp; b &#10;&#x41;&quot;"), "a & b \nA\"");
        assert_eq!(unescape(&escape_attr("x & \"y\" <z>\n")), "x & \"y\" <z>\n");
    }
}
