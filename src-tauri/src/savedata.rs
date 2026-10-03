/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use std::fs;
use std::path::Path;

pub const SAVEDATA_AES_KEY: [u8; 32] = [
    0xd7, 0x8c, 0x32, 0x4a, 0x94, 0x42, 0x94, 0x3c,
    0x6d, 0x65, 0xce, 0x98, 0x81, 0x85, 0x4c, 0x41,
    0x68, 0x99, 0x22, 0x0c, 0xc7, 0xa1, 0x46, 0x40,
    0x93, 0x9b, 0x96, 0x3c, 0x93, 0x2a, 0x6f, 0xaf,
];

pub const CRC_SEED: u32 = 4023120385;
pub const OBJHEADER: u32 = 0xFFFFFFFF;
pub const VELOCITY_UPPER_MAGIC: u32 = 0x56454C4F;

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc = crc >> 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

pub fn rl_crc32(data: &[u8], seed: u32) -> u32 {
    let mut crc = seed.reverse_bits() ^ 0xFFFFFFFF;
    for &b in data {
        let tb = b.reverse_bits();
        let idx = ((crc ^ (tb as u32)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    (crc ^ 0xFFFFFFFF).reverse_bits()
}

pub fn aes_decrypt(ciphertext: &[u8]) -> Vec<u8> {
    let cipher = Aes256::new(GenericArray::from_slice(&SAVEDATA_AES_KEY));
    let mut out = ciphertext.to_vec();
    for chunk in out.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }
    out
}

pub fn aes_encrypt(plaintext: &[u8]) -> Vec<u8> {
    let padded_len = (plaintext.len() + 15) & !15;
    let mut out = Vec::with_capacity(padded_len);
    out.extend_from_slice(plaintext);
    out.resize(padded_len, 0);

    let cipher = Aes256::new(GenericArray::from_slice(&SAVEDATA_AES_KEY));
    for chunk in out.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    out
}

pub fn read_ue3(data: &[u8], offset: usize) -> Result<(String, usize), String> {
    if offset + 4 > data.len() {
        return Err("Unexpected end of data reading UE3 string length".to_string());
    }
    let len = i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    if len == 0 {
        return Ok((String::new(), offset + 4));
    }
    if len > 0 {
        let ulen = len as usize;
        let str_end = offset + 4 + ulen;
        if str_end > data.len() {
            return Err("Unexpected end of data reading UTF-8 string".to_string());
        }
        let raw = &data[offset + 4..str_end - 1];
        let s = String::from_utf8_lossy(raw).to_string();
        Ok((s, str_end))
    } else {
        let char_count = (-len) as usize;
        let byte_count = char_count * 2;
        let str_end = offset + 4 + byte_count;
        if str_end > data.len() {
            return Err("Unexpected end of data reading UTF-16 string".to_string());
        }
        let raw = &data[offset + 4..str_end - 2];
        let u16s: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let s = String::from_utf16_lossy(&u16s);
        Ok((s, str_end))
    }
}

pub fn write_ue3(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let len = (bytes.len() + 1) as i32;
    let mut out = Vec::with_capacity(4 + bytes.len() + 1);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    out.push(0);
    out
}

#[derive(Debug, Clone)]
pub struct ObjectTableEntry {
    pub type_name: String,
    pub file_position: u32,
    pub object_index: u32,
}

#[derive(Debug, Clone)]
pub struct SaveEnvelope {
    pub header: Vec<u8>,
    pub sd: Vec<u8>,
    pub table: Vec<ObjectTableEntry>,
}

pub fn read_envelope(save_path: &Path) -> Result<SaveEnvelope, String> {
    let raw = fs::read(save_path).map_err(|e| format!("Failed to read save file: {}", e))?;
    if raw.len() < 8 {
        return Err("Save file is smaller than 8-byte header".to_string());
    }

    let enc_len = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
    let expected_crc = u32::from_le_bytes(raw[4..8].try_into().unwrap());

    if raw.len() < 8 + enc_len {
        return Err("Save file ciphertext is truncated".to_string());
    }

    let enc = &raw[8..8 + enc_len];
    let actual_crc = rl_crc32(enc, CRC_SEED);
    if actual_crc != expected_crc {
        return Err(format!(
            "Save envelope CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
            expected_crc, actual_crc
        ));
    }

    let dec = aes_decrypt(enc);
    if dec.len() < 24 {
        return Err("Decrypted save data is too short".to_string());
    }

    let save_len = i32::from_le_bytes(dec[20..24].try_into().unwrap()) as usize;
    let sd_start = 24;
    let sd_end = sd_start + save_len - 4;

    if sd_end > dec.len() {
        return Err("Save data end position exceeds decrypted stream".to_string());
    }

    let header = dec[..sd_start].to_vec();
    let sd = dec[sd_start..sd_end].to_vec();

    let count = i32::from_le_bytes(dec[sd_end..sd_end + 4].try_into().unwrap()) as usize;
    let mut cursor = sd_end + 4;
    let mut table = Vec::with_capacity(count);

    for _ in 0..count {
        let (type_name, next_cur) = read_ue3(&dec, cursor)?;
        cursor = next_cur;
        if cursor + 8 > dec.len() {
            return Err("Truncated object table entry".to_string());
        }
        let file_pos = u32::from_le_bytes(dec[cursor..cursor + 4].try_into().unwrap());
        let obj_idx = u32::from_le_bytes(dec[cursor + 4..cursor + 8].try_into().unwrap());
        cursor += 8;
        table.push(ObjectTableEntry {
            type_name,
            file_position: file_pos,
            object_index: obj_idx,
        });
    }

    Ok(SaveEnvelope { header, sd, table })
}

pub fn write_envelope(
    save_path: &Path,
    header: &mut [u8],
    sd: &[u8],
    table: &[ObjectTableEntry],
) -> Result<(), String> {
    if header.len() < 24 {
        return Err("Invalid header length".to_string());
    }

    let save_len = (sd.len() as i32) + 4;
    header[20..24].copy_from_slice(&save_len.to_le_bytes());

    let mut table_payload = Vec::new();
    for entry in table {
        table_payload.extend_from_slice(&write_ue3(&entry.type_name));
        table_payload.extend_from_slice(&entry.file_position.to_le_bytes());
        table_payload.extend_from_slice(&entry.object_index.to_le_bytes());
    }

    let mut payload = Vec::with_capacity(header.len() + sd.len() + 4 + table_payload.len());
    payload.extend_from_slice(header);
    payload.extend_from_slice(sd);
    payload.extend_from_slice(&(table.len() as i32).to_le_bytes());
    payload.extend_from_slice(&table_payload);

    let encrypted = aes_encrypt(&payload);
    let crc = rl_crc32(&encrypted, CRC_SEED);

    let mut output = Vec::with_capacity(8 + encrypted.len());
    output.extend_from_slice(&(encrypted.len() as u32).to_le_bytes());
    output.extend_from_slice(&crc.to_le_bytes());
    output.extend_from_slice(&encrypted);

    fs::write(save_path, &output).map_err(|e| format!("Failed to write save file: {}", e))?;
    Ok(())
}

pub fn find_online_products(sd: &[u8]) -> Result<(usize, usize, usize), String> {
    let mut cursor = 4;
    loop {
        if cursor >= sd.len() {
            return Err("Reached end of data without finding OnlineProducts".to_string());
        }
        let (name, after_name) = read_ue3(sd, cursor)?;
        if name == "None" {
            return Err("The save has no OnlineProducts inventory index".to_string());
        }
        let (tag, after_tag) = read_ue3(sd, after_name)?;
        if after_tag + 4 > sd.len() {
            return Err("Truncated property length".to_string());
        }
        let length = i32::from_le_bytes(sd[after_tag..after_tag + 4].try_into().unwrap()) as usize;
        let value_pos = after_tag + 8;

        if name == "OnlineProducts" && tag == "ArrayProperty" {
            return Ok((after_tag, value_pos, length));
        }

        let adv = if tag == "BoolProperty" { 1 } else { length };
        cursor = value_pos + adv;
    }
}

pub fn build_product_blob(
    product_id: i32,
    paint_object_index: Option<u32>,
    upper_bits: u64,
    lower_bits: u64,
    timestamp: u64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(400);

    buf.extend_from_slice(&OBJHEADER.to_le_bytes());

    buf.extend_from_slice(&write_ue3("ProductID"));
    buf.extend_from_slice(&write_ue3("IntProperty"));
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&product_id.to_le_bytes());

    buf.extend_from_slice(&write_ue3("InstanceID"));
    buf.extend_from_slice(&write_ue3("StructProperty"));
    buf.extend_from_slice(&137i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&write_ue3("TAGame.ProductInstanceID_TA"));

    buf.extend_from_slice(&write_ue3("UpperBits"));
    buf.extend_from_slice(&write_ue3("QWordProperty"));
    buf.extend_from_slice(&8i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&upper_bits.to_le_bytes());

    buf.extend_from_slice(&write_ue3("LowerBits"));
    buf.extend_from_slice(&write_ue3("QWordProperty"));
    buf.extend_from_slice(&8i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&lower_bits.to_le_bytes());

    buf.extend_from_slice(&write_ue3("None"));

    buf.extend_from_slice(&write_ue3("SeriesID"));
    buf.extend_from_slice(&write_ue3("IntProperty"));
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&1i32.to_le_bytes());

    buf.extend_from_slice(&write_ue3("AddedTimestamp"));
    buf.extend_from_slice(&write_ue3("QWordProperty"));
    buf.extend_from_slice(&8i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&timestamp.to_le_bytes());

    if let Some(paint_idx) = paint_object_index {
        buf.extend_from_slice(&write_ue3("Attributes"));
        buf.extend_from_slice(&write_ue3("ArrayProperty"));
        buf.extend_from_slice(&8i32.to_le_bytes());
        buf.extend_from_slice(&0i32.to_le_bytes());
        buf.extend_from_slice(&1i32.to_le_bytes());
        buf.extend_from_slice(&(paint_idx as i32).to_le_bytes());
    }

    buf.extend_from_slice(&write_ue3("None"));
    buf
}

pub fn build_paint_blob(paint_id: i32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(&OBJHEADER.to_le_bytes());
    buf.extend_from_slice(&write_ue3("PaintID"));
    buf.extend_from_slice(&write_ue3("IntProperty"));
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&paint_id.to_le_bytes());
    buf.extend_from_slice(&write_ue3("None"));
    buf
}

pub fn patch_equipped_loadouts_in_sd(
    sd: &mut [u8],
    table: &[ObjectTableEntry],
    slot_index: usize,
    base_product_id: i32,
    upper_bits: u64,
    lower_bits: u64,
) -> usize {
    let mut loadouts_patched = 0;

    for entry in table {
        if entry.type_name != "TAGame.Loadout_TA" {
            continue;
        }

        let start_pos = entry.file_position as usize;
        if start_pos >= sd.len() {
            continue;
        }

        let mut cursor = start_pos + 4;
        let mut patched_products = false;
        let mut patched_instances = false;

        while cursor < sd.len() {
            let (name, after_name) = match read_ue3(sd, cursor) {
                Ok(r) => r,
                Err(_) => break,
            };
            if name == "None" {
                break;
            }

            let (tag, after_tag) = match read_ue3(sd, after_name) {
                Ok(r) => r,
                Err(_) => break,
            };
            if after_tag + 4 > sd.len() {
                break;
            }

            let length = i32::from_le_bytes(sd[after_tag..after_tag + 4].try_into().unwrap()) as usize;
            let value_pos = after_tag + 8;

            if name == "Products" && tag == "ArrayProperty" {
                if value_pos + 4 <= sd.len() {
                    let count = i32::from_le_bytes(sd[value_pos..value_pos + 4].try_into().unwrap()) as usize;
                    let target_offset = value_pos + 4 + slot_index * 4;
                    if slot_index < count && target_offset + 4 <= sd.len() {
                        sd[target_offset..target_offset + 4].copy_from_slice(&base_product_id.to_le_bytes());
                        patched_products = true;
                    }
                }
            } else if name == "OnlineProducts128" && tag == "ArrayProperty" {
                if value_pos + 4 <= sd.len() {
                    let count = i32::from_le_bytes(sd[value_pos..value_pos + 4].try_into().unwrap()) as usize;
                    let mut elem_cursor = value_pos + 4;

                    for current_idx in 0..count {
                        if elem_cursor >= sd.len() {
                            break;
                        }

                        while elem_cursor < sd.len() {
                            let (field, field_name_end) = match read_ue3(sd, elem_cursor) {
                                Ok(r) => r,
                                Err(_) => break,
                            };
                            if field == "None" {
                                elem_cursor = field_name_end;
                                break;
                            }

                            let (field_tag, field_tag_end) = match read_ue3(sd, field_name_end) {
                                Ok(r) => r,
                                Err(_) => break,
                            };
                            if field_tag_end + 4 > sd.len() {
                                break;
                            }
                            let field_len = i32::from_le_bytes(
                                sd[field_tag_end..field_tag_end + 4].try_into().unwrap(),
                            ) as usize;
                            let field_val = field_tag_end + 8;

                            if current_idx == slot_index {
                                if field == "UpperBits" && field_val + 8 <= sd.len() {
                                    sd[field_val..field_val + 8].copy_from_slice(&upper_bits.to_le_bytes());
                                } else if field == "LowerBits" && field_val + 8 <= sd.len() {
                                    sd[field_val..field_val + 8].copy_from_slice(&lower_bits.to_le_bytes());
                                }
                            }

                            let adv = if field_tag == "BoolProperty" { 1 } else { field_len };
                            elem_cursor = field_val + adv;
                        }
                    }
                    patched_instances = true;
                }
            }

            if patched_products && patched_instances {
                loadouts_patched += 1;
                break;
            }

            let adv = if tag == "BoolProperty" { 1 } else { length };
            cursor = value_pos + adv;
        }
    }

    loadouts_patched
}

pub fn inject_spawn_native(
    save_path: &Path,
    product_id: i32,
    paint_id: i32,
    equip_as_body: bool,
) -> Result<String, String> {
    let mut envelope = read_envelope(save_path)?;
    let (length_pos, value_pos, old_length) = find_online_products(&envelope.sd)?;

    let count = i32::from_le_bytes(envelope.sd[value_pos..value_pos + 4].try_into().unwrap());
    let insert_at = value_pos + old_length;

    let product_index = envelope.table.len() as u32;
    let paint_index = if paint_id > 0 {
        Some(product_index + 1)
    } else {
        None
    };

    let rand_upper_low = rand::random::<u32>();
    let upper_bits = ((VELOCITY_UPPER_MAGIC as u64) << 32) | (rand_upper_low as u64);
    let lower_bits = rand::random::<u64>();
    let instance_hex = format!("{:016x}{:016x}", upper_bits, lower_bits);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if equip_as_body {
        patch_equipped_loadouts_in_sd(
            &mut envelope.sd,
            &envelope.table,
            0,
            product_id,
            upper_bits,
            lower_bits,
        );
    }

    envelope.sd.splice(insert_at..insert_at, product_index.to_le_bytes());

    let new_len = (old_length as i32) + 4;
    envelope.sd[length_pos..length_pos + 4].copy_from_slice(&new_len.to_le_bytes());

    let new_count = count + 1;
    envelope.sd[value_pos..value_pos + 4].copy_from_slice(&new_count.to_le_bytes());

    for entry in &mut envelope.table {
        entry.file_position += 4;
    }

    let prod_blob = build_product_blob(product_id, paint_index, upper_bits, lower_bits, now);
    let prod_pos = (envelope.sd.len() as u32) + 4;
    envelope.sd.extend_from_slice(&prod_blob);
    envelope.table.push(ObjectTableEntry {
        type_name: "TAGame.OnlineProduct_TA".to_string(),
        file_position: prod_pos,
        object_index: product_index,
    });

    if let Some(paint_idx) = paint_index {
        let p_blob = build_paint_blob(paint_id);
        let p_pos = (envelope.sd.len() as u32) + 4;
        envelope.sd.extend_from_slice(&p_blob);
        envelope.table.push(ObjectTableEntry {
            type_name: "TAGame.ProductAttribute_Painted_TA".to_string(),
            file_position: p_pos,
            object_index: paint_idx,
        });
    }

    write_envelope(save_path, &mut envelope.header, &envelope.sd, &envelope.table)?;

    Ok(instance_hex)
}

pub fn remove_spawns_native(
    save_path: &Path,
    known_instances: &[String],
    restore_octane_loadout: bool,
) -> Result<usize, String> {
    let mut envelope = read_envelope(save_path)?;
    let (length_pos, value_pos, old_length) = find_online_products(&envelope.sd)?;

    let count = i32::from_le_bytes(envelope.sd[value_pos..value_pos + 4].try_into().unwrap()) as usize;
    if count == 0 {
        return Ok(0);
    }

    let array_start = value_pos + 4;
    let array_end = array_start + count * 4;
    if array_end > envelope.sd.len() {
        return Err("OnlineProducts array range exceeds save data".to_string());
    }

    let mut table_map = std::collections::HashMap::new();
    for entry in &envelope.table {
        table_map.insert(entry.object_index, (entry.file_position as usize, entry.type_name.clone()));
    }

    let mut removed_positions = Vec::new();

    for i in 0..count {
        let offset = array_start + i * 4;
        let obj_idx = u32::from_le_bytes(envelope.sd[offset..offset + 4].try_into().unwrap());

        if let Some(&(pos, ref type_name)) = table_map.get(&obj_idx) {
            if type_name == "TAGame.OnlineProduct_TA" && pos < envelope.sd.len() {
                let obj_slice = &envelope.sd[pos..std::cmp::min(envelope.sd.len(), pos + 350)];
                let ub_pat = b"UpperBits\x00\x0e\x00\x00\x00QWordProperty\x00\x08\x00\x00\x00\x00\x00\x00\x00";
                let lb_pat = b"LowerBits\x00\x0e\x00\x00\x00QWordProperty\x00\x08\x00\x00\x00\x00\x00\x00\x00";

                let mut is_target = false;

                let ub = if let Some(p) = obj_slice.windows(ub_pat.len()).position(|w| w == ub_pat) {
                    let idx = p + ub_pat.len();
                    if idx + 8 <= obj_slice.len() {
                        u64::from_le_bytes(obj_slice[idx..idx + 8].try_into().unwrap_or([0; 8]))
                    } else { 0 }
                } else { 0 };

                let lb = if let Some(p) = obj_slice.windows(lb_pat.len()).position(|w| w == lb_pat) {
                    let idx = p + lb_pat.len();
                    if idx + 8 <= obj_slice.len() {
                        u64::from_le_bytes(obj_slice[idx..idx + 8].try_into().unwrap_or([0; 8]))
                    } else { 0 }
                } else { 0 };

                if (ub >> 32) == (VELOCITY_UPPER_MAGIC as u64) {
                    is_target = true;
                }

                let hex = format!("{:016x}{:016x}", ub, lb);
                if known_instances.iter().any(|k| k.eq_ignore_ascii_case(&hex)) {
                    is_target = true;
                }

                if is_target {
                    removed_positions.push(offset);
                }
            }
        }
    }

    if removed_positions.is_empty() && !restore_octane_loadout {
        return Ok(0);
    }

    for &pos in removed_positions.iter().rev() {
        envelope.sd.drain(pos..pos + 4);
    }

    let removed_count = removed_positions.len();
    let removed_bytes = removed_count * 4;

    let new_length = (old_length as i32) - (removed_bytes as i32);
    envelope.sd[length_pos..length_pos + 4].copy_from_slice(&new_length.to_le_bytes());

    let new_count = (count - removed_count) as i32;
    envelope.sd[value_pos..value_pos + 4].copy_from_slice(&new_count.to_le_bytes());

    let min_removed = removed_positions.first().copied().unwrap_or(0);
    for entry in &mut envelope.table {
        if (entry.file_position as usize) > min_removed {
            entry.file_position = entry.file_position.saturating_sub(removed_bytes as u32);
        }
    }

    if restore_octane_loadout {
        patch_equipped_loadouts_in_sd(
            &mut envelope.sd,
            &envelope.table,
            0,
            23,
            0,
            23,
        );
    }

    write_envelope(save_path, &mut envelope.header, &envelope.sd, &envelope.table)?;
    Ok(removed_count)
}
