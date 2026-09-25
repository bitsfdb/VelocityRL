fn serialize_fstring(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let len = (s.len() + 1) as i32;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out
}

fn i32_at(data: &[u8], off: usize) -> Result<i32, String> {
    let raw: [u8; 4] = data
        .get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| "truncated i32 in name table".to_string())?;
    Ok(i32::from_le_bytes(raw))
}

fn u64_at(data: &[u8], off: usize) -> Result<u64, String> {
    let raw: [u8; 8] = data
        .get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| "truncated u64 in name table".to_string())?;
    Ok(u64::from_le_bytes(raw))
}

fn i64_at(data: &[u8], off: usize) -> Result<i64, String> {
    let raw: [u8; 8] = data
        .get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| "truncated i64 in table".to_string())?;
    Ok(i64::from_le_bytes(raw))
}

fn serialize_name_entry(name: &str, flags: u64) -> Vec<u8> {
    let mut out = serialize_fstring(name);
    out.extend_from_slice(&flags.to_le_bytes());
    out
}

fn patch_export_serial_offsets(export_bytes: &mut [u8], threshold: i64, delta: i64) {
    let mut pos = 0usize;
    while pos + 72 <= export_bytes.len() {
        let serial_off_pos = pos + 36;
        if let Ok(serial_offset) = i64_at(export_bytes, serial_off_pos) {
            if serial_offset >= threshold {
                let new_val = serial_offset + delta;
                export_bytes[serial_off_pos..serial_off_pos + 8]
                    .copy_from_slice(&new_val.to_le_bytes());
            }
        }
        pos += 72;
    }
}

fn patch_chunk_table_uncompressed_offsets(beyond: &mut [u8], threshold: i64, delta: i64) {
    let found = super::parser::parse_chunks_with_stride(beyond, 0)
        .map(|(stride, chunks)| (0usize, stride, chunks))
        .or_else(|_| {
            for off in 1..beyond.len().saturating_sub(4) {
                if let Ok((stride, chunks)) = super::parser::parse_chunks_with_stride(beyond, off as i32) {
                    return Ok((off, stride, chunks));
                }
            }
            Err(())
        });

    if let Ok((table_off, stride, chunks)) = found {
        for (i, chunk) in chunks.iter().enumerate() {
            if chunk.uncompressed_offset >= threshold {
                let pos = table_off + 4 + i * stride;
                let new_unc_off = chunk.uncompressed_offset + delta;
                if pos + 8 <= beyond.len() {
                    beyond[pos..pos + 8].copy_from_slice(&new_unc_off.to_le_bytes());
                }
            }
        }
    }
}

struct NameSlotInfo {
    fstr_len_raw: i32,
    name: String,
}

fn parse_name_slots(data: &[u8], name_offset: i32, name_count: i32) -> Result<Vec<NameSlotInfo>, String> {
    let mut slots = Vec::with_capacity(name_count.max(0) as usize);
    let mut pos = name_offset as usize;
    for _ in 0..name_count.max(0) {
        if pos + 4 > data.len() { return Err("name table truncated".into()); }
        let fstr_len = i32_at(data, pos)?;
        let (capacity, name) = if fstr_len > 0 {
            let cap = fstr_len as usize;
            if pos + 4 + cap > data.len() { return Err("name entry overrun".into()); }
            let bytes = &data[pos+4..pos+4+cap];
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(cap);
            (cap, String::from_utf8_lossy(&bytes[..end]).into_owned())
        } else if fstr_len < 0 {
            let bc = (-fstr_len as usize) * 2;
            if pos + 4 + bc > data.len() { return Err("name entry overrun utf16".into()); }
            let bytes = &data[pos+4..pos+4+bc];
            let words: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0],b[1]])).collect();
            let end = words.iter().position(|&w| w == 0).unwrap_or(words.len());
            (bc, String::from_utf16_lossy(&words[..end]).to_owned())
        } else {
            (0, String::new())
        };
        let flags_off = pos + 4 + capacity;
        if flags_off + 8 > data.len() { return Err("name entry flags overrun".into()); }
        slots.push(NameSlotInfo {
            fstr_len_raw: fstr_len,
            name,
        });
        pos += 4 + capacity + 8;
    }
    Ok(slots)
}

fn is_unsafe_compensation_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.starts_with("textures") && (lower.len() == 8 || lower[8..].chars().all(|c| c.is_ascii_digit())))
        || lower.ends_with(".tfc")
        || lower.contains("tfc")
        || lower.starts_with("thumbnail")
        || matches!(
            lower.as_str(),
            "none"
                | "core"
                | "engine"
                | "tagame"
                | "package"
                | "object"
                | "class"
                | "component"
                | "group"
                | "model"
                | "polys"
        )
}

fn is_preferred_compensation_name(name: &str) -> bool {
    name.starts_with("WheelAttachments_Scene")
        || name.starts_with("Side_BPW")
        || name.starts_with("Default__")
        || name == "bRenderOnce"
        || name == "NeverStream"
        || name == "FunctionExpressions"
        || name == "CompressionNone"
        || name == "MipGenSettings"
        || name == "TextureMipGenSettings"
        || name.starts_with("bHas")
        || name.starts_with("bAuto")
        || name.starts_with("bPrecomputed")
        || name.starts_with("bUsed")
        || name.starts_with("bPerPixel")
        || name.starts_with("Include")
        || name.starts_with("SourceArt")
        || name.starts_with("Tire_")
}

pub fn apply_header_renames(
    header: Vec<u8>,
    import_off: usize,
    export_off: usize,
    depends_off: usize,
    base_depends_offset: i32,
    name_count: i32,
    pairs: &[(String, String)],
) -> Result<(Vec<u8>, i64), String> {

    let mut inplace = header.clone();
    match apply_name_pairs_inplace(&mut inplace, 0, name_count, pairs) {
        Ok(()) => return Ok((inplace, 0)),
        Err(e) if e.contains("already references") => return Err(e),
        Err(e) if e.contains("UTF-16") => return Err(e),
        Err(_) => {}
    }

    let orig_len = header.len();
    let mut cur = header;
    let mut cur_import = import_off;
    let mut cur_export = export_off;
    let mut cur_depends = depends_off;
    let mut cur_base_depends = base_depends_offset as i64;

    let mut effective_pairs = pairs.to_vec();
    if let Ok(slots) = parse_name_slots(&cur, 0, name_count) {
        let mut net_delta: i64 = 0;
        let mut matched_slot_indices = std::collections::HashSet::new();

        // Calculate net delta per matched slot (each slot is renamed at most once by the first matching pair)
        for (i, slot) in slots.iter().enumerate() {
            if let Some((_, new_str)) = pairs.iter().find(|(old_str, _)| slot.name.eq_ignore_ascii_case(old_str)) {
                matched_slot_indices.insert(i);
                let diff = (new_str.len() as i64) - (slot.name.len() as i64);
                net_delta += diff;
            }
        }

        if net_delta != 0 {
            let mut used_names = std::collections::HashSet::new();
            let mut pos = import_off;
            while pos + 28 <= export_off && pos + 28 <= cur.len() {
                if let Ok(pkg) = i32_at(&cur, pos) { used_names.insert(pkg as usize); }
                if let Ok(cls) = i32_at(&cur, pos + 8) { used_names.insert(cls as usize); }
                if let Ok(obj) = i32_at(&cur, pos + 20) { used_names.insert(obj as usize); }
                pos += 28;
            }
            let mut pos = export_off;
            while pos + 72 <= depends_off && pos + 72 <= cur.len() {
                if let Ok(obj) = i32_at(&cur, pos + 12) { used_names.insert(obj as usize); }
                pos += 72;
            }

            let min_len_needed = if net_delta > 0 { (net_delta as usize) + 4 } else { 0 };
            let is_eligible = |(i, s): &(usize, &NameSlotInfo)| -> bool {
                !matched_slot_indices.contains(i)
                    && !used_names.contains(i)
                    && !is_unsafe_compensation_name(&s.name)
                    && s.fstr_len_raw > 0
                    && s.name.len() > min_len_needed
            };

            let candidate = slots.iter().enumerate().rev()
                .find(|item| is_eligible(item) && is_preferred_compensation_name(&item.1.name))
                .or_else(|| slots.iter().enumerate().rev().find(is_eligible));

            if let Some((_, slot)) = candidate {
                let comp_new_str = if net_delta > 0 {
                    slot.name[..slot.name.len() - (net_delta as usize)].to_string()
                } else {
                    format!("{}{}", slot.name, "X".repeat((-net_delta) as usize))
                };
                effective_pairs.push((slot.name.clone(), comp_new_str));
            }
        }
    }

    for (old_str, new_str) in &effective_pairs {
        let slots = parse_name_slots(&cur, 0, name_count)?;
        let rename_idxs: Vec<usize> = slots.iter().enumerate()
            .filter(|(_, s)| s.name.eq_ignore_ascii_case(old_str))
            .map(|(i, _)| i)
            .collect();
        if rename_idxs.is_empty() { continue; }

        for &idx in &rename_idxs {
            if slots[idx].fstr_len_raw < 0 {
                return Err(format!("Name '{}' uses UTF-16; rename not supported.", old_str));
            }

            if cur_import > cur.len()
                || cur_export > cur.len()
                || cur_depends > cur.len()
                || cur_import > cur_export
                || cur_export > cur_depends
            {
                return Err("header table offsets OOB during rename rebuild".into());
            }
            let old_name_table = &cur[..cur_import];
            let import_table = cur[cur_import..cur_export].to_vec();
            let mut export_table = cur[cur_export..cur_depends].to_vec();
            let mut beyond = cur[cur_depends..].to_vec();

            let mut new_name_table: Vec<u8> = Vec::new();
            let mut pos = 0usize;
            let mut entry_i = 0usize;
            while pos < old_name_table.len() {
                if pos + 4 > old_name_table.len() {
                    break;
                }
                let Ok(flen) = i32_at(old_name_table, pos) else {
                    break;
                };
                let cb = if flen > 0 {
                    flen as usize
                } else if flen < 0 {
                    (-flen as usize).saturating_mul(2)
                } else {
                    0
                };
                let end = match pos.checked_add(4).and_then(|p| p.checked_add(cb)).and_then(|p| p.checked_add(8)) {
                    Some(e) => e,
                    None => break,
                };
                if end > old_name_table.len() {
                    break;
                }
                let Ok(flags) = u64_at(old_name_table, end - 8) else {
                    break;
                };
                if entry_i == idx {
                    new_name_table.extend_from_slice(&serialize_name_entry(new_str, flags));
                } else {
                    new_name_table.extend_from_slice(&old_name_table[pos..end]);
                }
                pos = end;
                entry_i += 1;
            }

            let delta = new_name_table.len() as i64 - old_name_table.len() as i64;
            if delta != 0 {
                patch_export_serial_offsets(&mut export_table, cur_base_depends, delta);
                patch_chunk_table_uncompressed_offsets(&mut beyond, cur_base_depends, delta);
                cur_base_depends += delta;
            }

            let mut rebuilt = Vec::with_capacity(cur.len() + delta.unsigned_abs() as usize);
            rebuilt.extend_from_slice(&new_name_table);
            rebuilt.extend_from_slice(&import_table);
            rebuilt.extend_from_slice(&export_table);
            rebuilt.extend_from_slice(&beyond);

            cur_import = (cur_import as i64 + delta) as usize;
            cur_export = (cur_export as i64 + delta) as usize;
            cur_depends = (cur_depends as i64 + delta) as usize;
            cur = rebuilt;
        }
    }

    let delta = cur.len() as i64 - orig_len as i64;
    Ok((cur, delta))
}

pub fn apply_name_pairs_inplace(
    data: &mut Vec<u8>,
    name_offset: i32,
    name_count: i32,
    pairs: &[(String, String)],
) -> Result<(), String> {

    struct NameSlot {
        fstring_data_offset: usize,
        fstring_capacity: usize,
        name: String,
    }

    let mut slots: Vec<NameSlot> = Vec::with_capacity(name_count.max(0) as usize);
    {
        let mut pos = name_offset as usize;
        for _ in 0..name_count.max(0) {
            if pos + 4 > data.len() {
                return Err("name table truncated".into());
            }
            let fstr_len = i32_at(data, pos)?;
            let (capacity, name) = if fstr_len > 0 {
                let cap = fstr_len as usize;
                if pos + 4 + cap > data.len() {
                    return Err("name entry overrun".into());
                }
                let bytes = &data[pos + 4..pos + 4 + cap];
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(cap);
                (cap, String::from_utf8_lossy(&bytes[..end]).into_owned())
            } else if fstr_len < 0 {
                let char_count = (-fstr_len) as usize;
                let byte_count = char_count * 2;
                if pos + 4 + byte_count > data.len() {
                    return Err("name entry overrun (utf-16)".into());
                }
                let bytes = &data[pos + 4..pos + 4 + byte_count];
                let words: Vec<u16> = bytes.chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let end = words.iter().position(|&w| w == 0).unwrap_or(words.len());
                (byte_count, String::from_utf16_lossy(&words[..end]).to_owned())
            } else {
                (0, String::new())
            };
            slots.push(NameSlot {
                fstring_data_offset: pos + 4,
                fstring_capacity: capacity,
                name,
            });

            pos += 4 + capacity + 8;
        }
    }

    for (old_str, new_str) in pairs {

        let rename_indices: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.name.eq_ignore_ascii_case(old_str))
            .map(|(i, _)| i)
            .collect();

        if rename_indices.is_empty() {

            continue;
        }

        for &idx in &rename_indices {
            let slot = &slots[idx];

            if slot.fstring_capacity == 0 {
                return Err(format!("Name '{}' has zero capacity, can't rename", old_str));
            }
            if slot.name.len() == 0 && new_str.len() > 0 && slot.fstring_capacity == 0 {
                return Err(format!("Can't rename empty name to '{}'", new_str));
            }

            let fstr_start = slot.fstring_data_offset - 4;
            let fstr_len_raw = i32_at(data, fstr_start)?;
            if fstr_len_raw < 0 {
                return Err(format!(
                    "Name '{}' uses UTF-16 encoding; in-place rename not supported. \
                     Choose an item with ASCII-compatible names.",
                    old_str
                ));
            }

            let needed = new_str.len() + 1;
            if needed != slot.fstring_capacity {
                return Err(format!(
                    "In-place rename '{}' → '{}' changes length from {} to {}; rebuild required.",
                    old_str, new_str, slot.fstring_capacity, needed
                ));
            }

            let region = &mut data[slot.fstring_data_offset..slot.fstring_data_offset + slot.fstring_capacity];
            for (i, b) in region.iter_mut().enumerate() {
                *b = if i < new_str.len() { new_str.as_bytes()[i] } else { 0 };
            }

            slots[idx].name = new_str.clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortening_rename_does_not_corrupt_nametable() {
        // Build a mock header with 3 entries:
        // 0: "Wheel_SoccerBall" (17 bytes with \0)
        // 1: "Wheel_SoccerBall_SF" (20 bytes with \0)
        // 2: "SomeOtherName" (14 bytes with \0)
        let mut header = Vec::new();
        header.extend_from_slice(&serialize_name_entry("Wheel_SoccerBall", 0x11223344));
        header.extend_from_slice(&serialize_name_entry("Wheel_SoccerBall_SF", 0x55667788));
        header.extend_from_slice(&serialize_name_entry("SomeOtherName", 0x99AABBCC));

        let import_off = header.len();
        let export_off = header.len();
        let depends_off = header.len();

        let pairs = vec![
            ("Wheel_SoccerBall".to_string(), "WHEEL_Triad".to_string()),
            ("Wheel_SoccerBall_SF".to_string(), "WHEEL_Triad_SF".to_string()),
        ];

        let (new_header, delta) = apply_header_renames(
            header,
            import_off,
            export_off,
            depends_off,
            depends_off as i32,
            3,
            &pairs,
        ).expect("rebuild succeeds");

        // Verify parsing new_header succeeds without FString length errors
        let parsed = crate::upk::parser::parse_name_table(&new_header, 0, 3)
            .expect("parse name table succeeds");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name, "WHEEL_Triad");
        assert_eq!(parsed[0].flags, 0x11223344);
        assert_eq!(parsed[1].name, "WHEEL_Triad_SF");
        assert_eq!(parsed[1].flags, 0x55667788);
        assert_eq!(parsed[2].name, "SomeOtherNameXXXXXXXXXX");
        assert_eq!(parsed[2].flags, 0x99AABBCC);
        assert_eq!(delta, 0, "shortening rename should absorb delta to 0 via compensation");
    }

    #[test]
    fn test_expanding_rename_absorbs_delta_via_compensation() {
        let mut header = Vec::new();
        header.extend_from_slice(&serialize_name_entry("body_grain", 0x11223344));
        header.extend_from_slice(&serialize_name_entry("body_grain_SF", 0x55667788));
        header.extend_from_slice(&serialize_name_entry("Tire_Scorpion_Vesper_Textures", 0x99AABBCC));

        let import_off = header.len();
        let export_off = header.len();
        let depends_off = header.len();

        let pairs = vec![
            ("body_grain".to_string(), "Body_Octane".to_string()),
            ("Body_Grain".to_string(), "Body_Octane".to_string()),
            ("BODY_GRAIN".to_string(), "Body_Octane".to_string()),
            ("body_grain_SF".to_string(), "Body_Octane_SF".to_string()),
            ("body_grain_sf".to_string(), "Body_Octane_sf".to_string()),
        ];

        let (new_header, delta) = apply_header_renames(
            header,
            import_off,
            export_off,
            depends_off,
            depends_off as i32,
            3,
            &pairs,
        ).expect("rebuild succeeds");

        assert_eq!(delta, 0, "expanding rename should absorb delta to 0 via compensation");

        let parsed = crate::upk::parser::parse_name_table(&new_header, 0, 3)
            .expect("parse name table succeeds");
        assert_eq!(parsed[0].name, "Body_Octane");
        assert_eq!(parsed[1].name, "Body_Octane_SF");
        assert_eq!(parsed[2].name, "Tire_Scorpion_Vesper_Textur");
    }

    #[test]
    fn test_compensation_avoids_texture_cache_names() {
        let mut header = Vec::new();
        header.extend_from_slice(&serialize_name_entry("LongMaterialInstanceName", 0x11223344));
        header.extend_from_slice(&serialize_name_entry("bHasQualitySwitch", 0x55667788));
        header.extend_from_slice(&serialize_name_entry("Textures7", 0x99AABBCC));

        let import_off = header.len();
        let export_off = header.len();
        let depends_off = header.len();

        let pairs = vec![
            ("LongMaterialInstanceName".to_string(), "ShortMaterial".to_string()),
        ];

        let (new_header, delta) = apply_header_renames(
            header,
            import_off,
            export_off,
            depends_off,
            depends_off as i32,
            3,
            &pairs,
        ).expect("rebuild succeeds");

        assert_eq!(delta, 0);

        let parsed = crate::upk::parser::parse_name_table(&new_header, 0, 3)
            .expect("parse name table succeeds");
        assert_eq!(parsed[0].name, "ShortMaterial");
        assert_eq!(parsed[1].name, "bHasQualitySwitchXXXXXXXXXXX");
        assert_eq!(parsed[2].name, "Textures7");
    }

    #[test]
    fn test_compensation_avoids_thumbnail_names() {
        let mut header = Vec::new();
        header.extend_from_slice(&serialize_name_entry("LongMaterialInstanceName", 0x11223344));
        header.extend_from_slice(&serialize_name_entry("ThumbnailRenderers", 0x55667788));
        header.extend_from_slice(&serialize_name_entry("NeverStream", 0x99AABBCC));

        let import_off = header.len();
        let export_off = header.len();
        let depends_off = header.len();

        let pairs = vec![
            ("LongMaterialInstanceName".to_string(), "ShortMaterial".to_string()),
        ];

        let (new_header, delta) = apply_header_renames(
            header,
            import_off,
            export_off,
            depends_off,
            depends_off as i32,
            3,
            &pairs,
        ).expect("rebuild succeeds");

        assert_eq!(delta, 0);

        let parsed = crate::upk::parser::parse_name_table(&new_header, 0, 3)
            .expect("parse name table succeeds");
        assert_eq!(parsed[0].name, "ShortMaterial");
        assert_eq!(parsed[1].name, "ThumbnailRenderers");
        assert_eq!(parsed[2].name, "NeverStreamXXXXXXXXXXX");
    }
}
