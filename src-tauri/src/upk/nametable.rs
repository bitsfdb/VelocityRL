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

fn serialize_name_entry(name: &str, flags: u64) -> Vec<u8> {
    let mut out = serialize_fstring(name);
    out.extend_from_slice(&flags.to_le_bytes());
    out
}

fn patch_export_serial_offsets(export_bytes: &mut Vec<u8>, threshold: i64, delta: i64) {
    let mut pos = 0usize;
    while pos + 72 <= export_bytes.len() {

        let serial_off_pos = pos + 36;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&export_bytes[serial_off_pos..serial_off_pos + 8]);
        let serial_offset = i64::from_le_bytes(buf);
        if serial_offset >= threshold {
            let new_val = serial_offset + delta;
            export_bytes[serial_off_pos..serial_off_pos + 8]
                .copy_from_slice(&new_val.to_le_bytes());
        }

        let noc_pos = pos + 68;
        if noc_pos + 4 > export_bytes.len() {
            break;
        }
        let noc = match i32_at(export_bytes, noc_pos) {
            Ok(n) => n,
            Err(_) => break,
        };
        let entry_size = 72 + (noc.max(0) as usize) * 4;
        pos += entry_size;
    }
}

pub fn rebuild_with_rename(
    data: &[u8],
    name_offset: i32,
    import_offset: i32,
    export_offset: i32,
    depends_offset: i32,
    name_idx: usize,
    new_name: &str,
    old_flags: u64,
) -> Result<Vec<u8>, String> {
    let no = name_offset as usize;
    let io = import_offset as usize;
    let eo = export_offset as usize;
    let dep = depends_offset as usize;

    if dep > data.len() || eo > dep || io > eo || no > io {
        return Err("header offsets OOB for rebuild".into());
    }

    let old_name_table = &data[no..io];
    let import_table = &data[io..eo];
    let export_table = &data[eo..dep];
    let body_and_beyond = &data[dep..];

    let mut new_name_table: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    let mut idx = 0usize;
    while pos < old_name_table.len() {
        if pos + 4 > old_name_table.len() { break; }
        let fstr_len = i32_at(old_name_table, pos)?;
        let (char_bytes, char_count) = if fstr_len > 0 {
            (fstr_len as usize, fstr_len as usize)
        } else if fstr_len < 0 {
            ((-fstr_len as usize) * 2, (-fstr_len as usize) * 2)
        } else {
            (0, 0)
        };
        let entry_end = pos + 4 + char_bytes + 8;
        if entry_end > old_name_table.len() { break; }
        let flags = u64_at(old_name_table, entry_end - 8)?;

        if idx == name_idx {
            new_name_table.extend_from_slice(&serialize_name_entry(new_name, flags));
        } else {
            new_name_table.extend_from_slice(&old_name_table[pos..entry_end]);
        }
        pos = entry_end;
        idx += 1;
        let _ = char_count;
    }

    let delta = new_name_table.len() as i64 - old_name_table.len() as i64;

    let mut new_export_table = export_table.to_vec();
    if delta != 0 {
        patch_export_serial_offsets(&mut new_export_table, depends_offset as i64, delta);
    }

    let mut out = Vec::with_capacity(data.len() + delta.unsigned_abs() as usize);
    out.extend_from_slice(&data[..no]);
    out.extend_from_slice(&new_name_table);
    out.extend_from_slice(import_table);
    out.extend_from_slice(&new_export_table);
    out.extend_from_slice(body_and_beyond);

    let new_import_offset = (io as i64 + (new_name_table.len() as i64 - old_name_table.len() as i64)) as i32;
    let new_export_offset = new_import_offset + (eo as i32 - io as i32);
    let new_depends_offset = new_export_offset + (dep as i32 - eo as i32);
    patch_prefix_offset(&mut out, import_offset, new_import_offset);
    patch_prefix_offset(&mut out, export_offset, new_export_offset);
    patch_prefix_offset(&mut out, depends_offset, new_depends_offset);

    Ok(out)
}

fn patch_prefix_offset(data: &mut Vec<u8>, old_val: i32, new_val: i32) {
    if old_val == new_val { return; }
    let old_bytes = old_val.to_le_bytes();
    let new_bytes = new_val.to_le_bytes();

    let limit = data.len().min(1024);
    let mut i = 0;
    while i + 4 <= limit {
        if data[i..i+4] == old_bytes {
            data[i..i+4].copy_from_slice(&new_bytes);
        }
        i += 1;
    }
}

pub fn apply_name_pairs(
    data: &mut Vec<u8>,
    name_offset: i32,
    import_offset: i32,
    export_offset: i32,
    depends_offset: i32,
    name_count: i32,
    pairs: &[(String, String)],
) -> Result<(), String> {

    let mut cur_import_offset = import_offset;
    let mut cur_export_offset = export_offset;
    let mut cur_depends_offset = depends_offset;

    for (old_str, new_str) in pairs {

        let slots = parse_name_slots(data, name_offset, name_count)?;
        let rename_indices: Vec<usize> = slots.iter().enumerate()
            .filter(|(_, s)| s.name.eq_ignore_ascii_case(old_str))
            .map(|(i, _)| i)
            .collect();
        if rename_indices.is_empty() { continue; }

        for &idx in &rename_indices {
            let slot = &slots[idx];
            if slot.fstr_len_raw < 0 {
                return Err(format!("Name '{}' uses UTF-16; in-place rename not supported.", old_str));
            }
            {

                let new_data = rebuild_with_rename(
                    data,
                    name_offset,
                    cur_import_offset,
                    cur_export_offset,
                    cur_depends_offset,
                    idx,
                    new_str,
                    slot.flags,
                )?;
                let delta = new_data.len() as i64 - data.len() as i64;
                *data = new_data;
                cur_import_offset = (cur_import_offset as i64 + delta) as i32;

                cur_depends_offset = (cur_depends_offset as i64 + delta) as i32;

                let name_table_growth = delta;
                cur_import_offset = (import_offset as i64 + name_table_growth) as i32;
                cur_export_offset = (export_offset as i64 + name_table_growth) as i32;
            }
        }
    }
    Ok(())
}

struct NameSlotInfo {
    fstring_data_offset: usize,
    fstr_len_raw: i32,
    flags: u64,
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
        let flags = u64_at(data, flags_off)?;
        slots.push(NameSlotInfo {
            fstring_data_offset: pos + 4,
            fstr_len_raw: fstr_len,
            flags,
            name,
        });
        pos += 4 + capacity + 8;
    }
    Ok(slots)
}

pub fn apply_header_renames(
    header: Vec<u8>,
    import_off: usize,
    export_off: usize,
    depends_off: usize,
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

    for (old_str, new_str) in pairs {
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
            let export_table = cur[cur_export..cur_depends].to_vec();
            let beyond = cur[cur_depends..].to_vec();

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
            if needed > slot.fstring_capacity {
                return Err(format!(
                    "Cannot rename '{}' → '{}': needs {} bytes but only {} available. \
                     Choose a visual item with a shorter name.",
                    old_str, new_str, needed, slot.fstring_capacity
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
