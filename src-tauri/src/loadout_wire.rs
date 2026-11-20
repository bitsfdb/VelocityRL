/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 *
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 *
 * B1 preset key -> fixed-width decimal loadout records.
 */

use base64::Engine;

/// Attribute keys for B1 records.
pub const ATTR_KEYS: [&str; 10] = [
    "Painted",
    "SpecialEdition",
    "TeamEdition",
    "UserColor",
    "TitleId",
    "Quality",
    "Certified",
    "Level",
    "Series",
    "NoNotify",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireRecord {
    pub team: u32,
    pub index: u32,
    pub product: u32,
    pub attributes: Vec<(String, String)>,
}

impl WireRecord {
    pub fn paint(&self) -> u32 {
        self.attributes
            .iter()
            .find(|(k, _)| k == "Painted")
            .and_then(|(_, v)| v.parse::<u32>().ok())
            .filter(|p| (1..=29).contains(p))
            .unwrap_or(0)
    }
}

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// Encode one record: varint(((product + 2) << 1) | has_attrs) + attributes.
pub fn encode_record(product: u32, attributes: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, (((product as u64) + 2) << 1) | u64::from(!attributes.is_empty()));
    if !attributes.is_empty() {
        out.push(attributes.len().min(32) as u8);
        for (key, value) in attributes.iter().take(32) {
            if let Some(idx) = ATTR_KEYS.iter().position(|k| *k == key) {
                out.push((idx + 1) as u8);
            } else {
                out.push(0);
                put_str(&mut out, key);
            }
            put_str(&mut out, value);
        }
    }
    out
}

/// Build a `B1:` preset key from 21 game slots.
/// `slots` is indexed by the game's product-slot index; a zero product means
/// "empty slot" and is encoded as the skip marker (0x00).
pub fn encode_loadout(slots: &[(u32, Vec<(String, String)>)], mode: u8) -> String {
    let count = slots.len().clamp(1, 64);
    let mut blob = Vec::with_capacity(3 + count * 8);
    blob.push(1); // version
    blob.push(count as u8);
    blob.push(if mode <= 1 { mode } else { 1 });

    for (product, attrs) in slots.iter().take(count) {
        if *product == 0 {
            blob.push(0);
        } else {
            blob.extend_from_slice(&encode_record(*product, attrs));
        }
    }
    if mode == 0 {
        for (product, attrs) in slots.iter().take(count) {
            if *product == 0 {
                blob.push(0);
            } else {
                blob.extend_from_slice(&encode_record(*product, attrs));
            }
        }
    }

    format!("B1:{}", base64::engine::general_purpose::STANDARD.encode(blob))
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let h = hex_val(bytes[i + 1]);
                let l = hex_val(bytes[i + 2]);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Pull the B1 key out of a URL query, a JSON-ish body, or a raw body.
pub fn find_key(query: &str, body: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if !query.is_empty() {
        candidates.push(percent_decode(query));
    }
    candidates.push(body.to_string());
    for text in candidates {
        if let Some(pos) = text.find("B1:") {
            let tail = &text[pos..];
            let key: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | ':' | '-' | '_'))
                .collect();
            if key.len() > 3 {
                return Some(key);
            }
        }
    }
    None
}

/// Decode a `B1:` key into per-team records.
pub fn decode_key(key: &str) -> Option<Vec<WireRecord>> {
    let b64 = key.strip_prefix("B1:")?;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if raw.len() < 3 {
        return None;
    }
    let (version, count, mode) = (raw[0], raw[1], raw[2]);
    if version != 1 || !(1..=64).contains(&count) || mode > 1 {
        return None;
    }
    let mut cur = 3usize;
    let mut records = Vec::new();

    let read_varint = |cur: &mut usize| -> Option<u32> {
        let mut v: u32 = 0;
        let mut shift = 0;
        loop {
            let b = *raw.get(*cur)?;
            *cur += 1;
            if shift == 28 && b > 7 {
                return None;
            }
            v |= u32::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 28 {
                return None;
            }
        }
    };
    let read_str = |cur: &mut usize| -> Option<String> {
        let n = read_varint(cur)? as usize;
        if n > 2048 || cur.checked_add(n)? > raw.len() {
            return None;
        }
        let s = String::from_utf8_lossy(&raw[*cur..*cur + n]).into_owned();
        *cur += n;
        Some(s)
    };

    let parse_list = |cur: &mut usize, team: u32, records: &mut Vec<WireRecord>| -> Option<()> {
        for index in 0..count as u32 {
            let v = read_varint(cur)?;
            if v == 0 {
                continue; // empty slot; keep the index
            }
            let product = v.checked_shr(1).unwrap_or(0).wrapping_sub(2);
            let has_attrs = v & 1 != 0;
            let mut attrs = Vec::new();
            if has_attrs {
                let n = *raw.get(*cur)? as usize;
                *cur += 1;
                if !(1..=32).contains(&n) {
                    return None;
                }
                for _ in 0..n {
                    let sel = *raw.get(*cur)?;
                    *cur += 1;
                    let key_idx = sel & 0x7F;
                    let key = if key_idx == 0 {
                        read_str(cur)?
                    } else {
                        ATTR_KEYS.get(key_idx as usize - 1).unwrap_or(&"Unknown").to_string()
                    };
                    let value = if sel & 0x80 == 0 {
                        read_str(cur)?
                    } else {
                        String::new()
                    };
                    attrs.push((key, value));
                }
            }
            if product > 0 && product <= 99999 && product != 3036 && index <= 99 {
                records.push(WireRecord { team, index, product, attributes: attrs });
            }
        }
        Some(())
    };

    parse_list(&mut cur, 0, &mut records)?;
    if mode == 1 {
        let first: Vec<WireRecord> = records
            .iter()
            .map(|r| WireRecord { team: 1, index: r.index, product: r.product, attributes: r.attributes.clone() })
            .collect();
        records.extend(first);
    } else {
        parse_list(&mut cur, 1, &mut records)?;
    }
    if cur != raw.len() {
        return None;
    }
    Some(records)
}

/// The `D...` decimal format parsed by the game bytecode: 13 chars per record.
pub fn encode_digits(records: &[WireRecord]) -> String {
    let mut out = String::with_capacity(1 + records.len() * 13);
    out.push('D');
    for r in records {
        out.push(char::from(b'0' + (r.team.min(9) as u8)));
        out.push_str(&(r.index + 100).to_string());
        out.push_str(&(r.product + 100_000).to_string());
        out.push_str(&(r.paint() + 100).to_string());
    }
    out
}

pub fn key_to_digits(key: &str) -> Option<String> {
    let records = decode_key(key)?;
    Some(encode_digits(&records))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fennec_roundtrip() {
        let slots = vec![
            (4284u32, vec![("Painted".into(), "29".into())]),
            (0, vec![]), (0, vec![]), (0, vec![]), (0, vec![]),
            (0, vec![]), (0, vec![]), (0, vec![]), (0, vec![]), (0, vec![]),
            (0, vec![]), (0, vec![]), (0, vec![]), (0, vec![]),
        ];
        let key = encode_loadout(&slots, 1);
        assert!(key.starts_with("B1:"));
        let digits = key_to_digits(&key).unwrap();
        assert!(digits.starts_with("D0100104284129"));
        assert!(digits.contains("1100104284129"));
    }

    #[test]
    fn skip_marker_preserves_slot_index() {
        let key = "B1:AQUB/UIBAQIyOcaTAdTKAes6AQEBMZwr";
        let rec = decode_key(key).unwrap();
        assert_eq!(rec.len(), 10);
        assert_eq!(rec[0].product, 4284);
        assert_eq!(rec[0].paint(), 29);
        assert_eq!(rec[3].index, 3);
        assert_eq!(rec[3].product, 3763);
        assert_eq!(rec[3].paint(), 1);
        assert!(encode_digits(&rec).starts_with("D01001042841290101109"));
    }

    #[test]
    fn key_extraction_from_query_and_json() {
        assert_eq!(find_key("B1%3AAQEB%2FEI%3D", "").unwrap(), "B1:AQEB/EI=");
        assert_eq!(find_key("", "{\"Loadout\":\"B1:AQEB/EI=\"}").unwrap(), "B1:AQEB/EI=");
    }
}
