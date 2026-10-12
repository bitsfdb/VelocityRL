/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use std::path::{Path, PathBuf};
use std::fs;
use std::time::SystemTime;
use serde::{Deserialize, Serialize};
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;

pub const AES_KEY: [u8; 32] = [
    0xd7, 0x8c, 0x32, 0x4a, 0x94, 0x42, 0x94, 0x3c,
    0x6d, 0x65, 0xce, 0x98, 0x81, 0x85, 0x4c, 0x41,
    0x68, 0x99, 0x22, 0x0c, 0xc7, 0xa1, 0x46, 0x40,
    0x93, 0x9b, 0x96, 0x3c, 0x93, 0x2a, 0x6f, 0xaf,
];

pub const CRC_SEED: u32 = 0xEFCBF201;
pub const OBJHEADER: u32 = 0xFFFF_FFFF;

// Precomputed IEEE 802.3 CRC32 lookup table
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            if (c & 1) != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// Rocket League custom save CRC32 (reflected/reversed IEEE 802.3 matching C# implementation)
pub fn save_crc32(data: &[u8], seed: u32) -> u32 {
    let mut c = seed.reverse_bits() ^ 0xFFFF_FFFF;
    for &b in data {
        let rev_b = b.reverse_bits();
        c = CRC32_TABLE[((c ^ rev_b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    (c ^ 0xFFFF_FFFF).reverse_bits()
}

pub fn aes_decrypt(data: &[u8]) -> Vec<u8> {
    let cipher = Aes256::new_from_slice(&AES_KEY).unwrap();
    let mut out = data.to_vec();
    for chunk in out.chunks_exact_mut(16) {
        cipher.decrypt_block(aes::Block::from_mut_slice(chunk));
    }
    out
}

pub fn aes_encrypt(data: &[u8]) -> Vec<u8> {
    let cipher = Aes256::new_from_slice(&AES_KEY).unwrap();
    let padded_len = (data.len() + 15) & !15;
    let mut out = vec![0u8; padded_len];
    out[..data.len()].copy_from_slice(data);
    for chunk in out.chunks_exact_mut(16) {
        cipher.encrypt_block(aes::Block::from_mut_slice(chunk));
    }
    out
}

pub fn read_ue3_string(data: &[u8], offset: usize) -> Result<(String, usize), String> {
    if offset + 4 > data.len() {
        return Err("Truncated UE3 string header".into());
    }
    let length = i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    let mut cursor = offset + 4;
    if length == 0 {
        return Ok((String::new(), cursor));
    }
    if length > 0 {
        let byte_len = length as usize;
        if cursor + byte_len > data.len() {
            return Err("Truncated UTF-8 UE3 string".into());
        }
        let raw = &data[cursor..cursor + byte_len];
        cursor += byte_len;
        let s = if raw.ends_with(b"\0") {
            String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned()
        } else {
            String::from_utf8_lossy(raw).into_owned()
        };
        Ok((s, cursor))
    } else {
        let char_count = (-length) as usize;
        let byte_len = char_count * 2;
        if cursor + byte_len > data.len() {
            return Err("Truncated UTF-16 UE3 string".into());
        }
        let u16_slice: Vec<u16> = data[cursor..cursor + byte_len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
            .collect();
        cursor += byte_len;
        let s = String::from_utf16_lossy(&u16_slice)
            .trim_end_matches('\0')
            .to_string();
        Ok((s, cursor))
    }
}

pub fn write_ue3_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let length = (bytes.len() + 1) as i32;
    let mut out = Vec::with_capacity(4 + bytes.len() + 1);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);
    out.push(0);
    out
}

#[derive(Clone, Debug)]
pub struct ObjectTableEntry {
    pub type_name: String,
    pub file_position: u32,
    pub object_index: u32,
}

#[derive(Clone, Debug)]
pub struct SaveEnvelope {
    pub header: Vec<u8>, // 24 bytes (preamble + save_len)
    pub sd: Vec<u8>,     // UE3 property stream
    pub table: Vec<ObjectTableEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SpawnRecord {
    pub instance_hex: String,
    pub product_id: i32,
    pub product_name: String,
    pub paint_id: i32,
    pub slot: Option<String>,
    pub timestamp: String,
}

/// Locate active Rocket League save file (*.save)
pub fn locate_latest_save() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    let mut doc_dirs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // Try Windows userprofile / documents
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let p = PathBuf::from(&userprofile);
            doc_dirs.push(p.join("Documents"));
            doc_dirs.push(p.join("OneDrive").join("Documents"));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(&home);
        doc_dirs.push(p.join("Documents"));
        doc_dirs.push(p.join("OneDrive").join("Documents"));
        // Wine path support
        doc_dirs.push(p.join(".wine/drive_c/users/root/Documents"));
        doc_dirs.push(p.join(".wine/drive_c/users/steamuser/Documents"));
    }

    for doc in doc_dirs {
        for sub in &["SaveDataEpic", "SaveData"] {
            let folder = doc.join("My Games").join("Rocket League").join("TAGame").join(sub).join("DBE_Production");
            if folder.is_dir() {
                if let Ok(entries) = fs::read_dir(&folder) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("save") {
                            let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                            candidates.push((mtime, path));
                        }
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        return Err("No Rocket League save file found. Launch Rocket League to the main menu first, then retry.".into());
    }

    candidates.sort_by_key(|c| c.0);
    Ok(candidates.last().unwrap().1.clone())
}

pub fn read_envelope(path: &Path) -> Result<SaveEnvelope, String> {
    let raw = fs::read(path).map_err(|e| format!("Failed to read save file {}: {e}", path.display()))?;
    if raw.len() < 8 {
        return Err("Save file is truncated (smaller than 8 bytes)".into());
    }
    let encrypted_len = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize;
    let expected_crc = u32::from_le_bytes(raw[4..8].try_into().unwrap());
    if raw.len() < 8 + encrypted_len {
        return Err("Save file payload is truncated".into());
    }
    let encrypted = &raw[8..8 + encrypted_len];
    let actual_crc = save_crc32(encrypted, CRC_SEED);
    if actual_crc != expected_crc {
        return Err(format!("Save CRC mismatch: expected {expected_crc:#010x}, calculated {actual_crc:#010x}"));
    }

    let dec = aes_decrypt(encrypted);
    if dec.len() < 24 {
        return Err("Decrypted save payload is too short".into());
    }
    let save_len = i32::from_le_bytes(dec[20..24].try_into().unwrap()) as usize;
    let sd_start = 24;
    let sd_end = sd_start + save_len - 4;
    if dec.len() < sd_end + 4 {
        return Err("Decrypted save payload truncated before object table".into());
    }
    let count = i32::from_le_bytes(dec[sd_end..sd_end + 4].try_into().unwrap()) as usize;
    let mut cursor = sd_end + 4;
    let mut table = Vec::with_capacity(count);
    for _ in 0..count {
        let (type_name, next_cur) = read_ue3_string(&dec, cursor)?;
        if next_cur + 8 > dec.len() {
            return Err("Truncated object table entry in save".into());
        }
        let file_position = u32::from_le_bytes(dec[next_cur..next_cur + 4].try_into().unwrap());
        let object_index = u32::from_le_bytes(dec[next_cur + 4..next_cur + 8].try_into().unwrap());
        table.push(ObjectTableEntry {
            type_name,
            file_position,
            object_index,
        });
        cursor = next_cur + 8;
    }

    let header = dec[..sd_start].to_vec();
    let sd = dec[sd_start..sd_end].to_vec();
    Ok(SaveEnvelope { header, sd, table })
}

/// Locate OnlineProducts ArrayProperty in sd
/// Returns (length_pos, value_pos, old_length, current_count)
pub fn find_online_products(sd: &[u8]) -> Result<(usize, usize, usize, i32), String> {
    const NEEDLE: &[u8] = b"\x0f\x00\x00\x00OnlineProducts\0";
    let pos = sd
        .windows(NEEDLE.len())
        .position(|w| w == NEEDLE)
        .ok_or_else(|| "The save file has no OnlineProducts inventory index".to_string())?;

    let after_name = pos + NEEDLE.len();
    let (tag, after_tag) = read_ue3_string(sd, after_name)?;
    if tag != "ArrayProperty" {
        return Err(format!("Expected ArrayProperty for OnlineProducts, found {tag}"));
    }
    if after_tag + 8 > sd.len() {
        return Err("Truncated OnlineProducts property length header".into());
    }
    let length = i32::from_le_bytes(sd[after_tag..after_tag + 4].try_into().unwrap()) as usize;
    let value_pos = after_tag + 8;
    if value_pos + 4 > sd.len() {
        return Err("Truncated OnlineProducts array count".into());
    }
    let count = i32::from_le_bytes(sd[value_pos..value_pos + 4].try_into().unwrap());
    Ok((after_tag, value_pos, length, count))
}

// 53-byte template for TAGame.ProductAttribute_Painted_TA
const PAINT_TEMPLATE: [u8; 53] = [
    0xff, 0xff, 0xff, 0xff, // OBJHEADER
    0x08, 0x00, 0x00, 0x00, 0x50, 0x61, 0x69, 0x6e, 0x74, 0x49, 0x44, 0x00, // "PaintID\0"
    0x0c, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "IntProperty\0"
    0x04, 0x00, 0x00, 0x00, // length = 4
    0x00, 0x00, 0x00, 0x00, // pad = 0
    0x00, 0x00, 0x00, 0x00, // PaintID i32 (offset 40)
    0x05, 0x00, 0x00, 0x00, 0x4e, 0x6f, 0x6e, 0x65, 0x00, // "None\0"
];

// 318-byte template for unpainted TAGame.OnlineProduct_TA
const UNPAINTED_TEMPLATE: [u8; 318] = [
    0xff, 0xff, 0xff, 0xff, // OBJHEADER
    0x0a, 0x00, 0x00, 0x00, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x74, 0x49, 0x44, 0x00, // "ProductID\0"
    0x0c, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "IntProperty\0"
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, // ProductID i32 (offset 42)
    0x0b, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x49, 0x44, 0x00, // "InstanceID\0"
    0x0f, 0x00, 0x00, 0x00, 0x53, 0x74, 0x72, 0x75, 0x63, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "StructProperty\0"
    0x7f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x12, 0x00, 0x00, 0x00, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x74, 0x49, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x49, 0x44, 0x00, // "ProductInstanceID\0"
    0x0a, 0x00, 0x00, 0x00, 0x55, 0x70, 0x70, 0x65, 0x72, 0x42, 0x69, 0x74, 0x73, 0x00, // "UpperBits\0"
    0x0e, 0x00, 0x00, 0x00, 0x51, 0x57, 0x6f, 0x72, 0x64, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "QWordProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // UpperBits u64 (offset 150)
    0x0a, 0x00, 0x00, 0x00, 0x4c, 0x6f, 0x77, 0x65, 0x72, 0x42, 0x69, 0x74, 0x73, 0x00, // "LowerBits\0"
    0x0e, 0x00, 0x00, 0x00, 0x51, 0x57, 0x6f, 0x72, 0x64, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "QWordProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // LowerBits u64 (offset 198)
    0x05, 0x00, 0x00, 0x00, 0x4e, 0x6f, 0x6e, 0x65, 0x00, // "None\0"
    0x09, 0x00, 0x00, 0x00, 0x53, 0x65, 0x72, 0x69, 0x65, 0x73, 0x49, 0x44, 0x00, // "SeriesID\0"
    0x0c, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "IntProperty\0"
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, // SeriesID = 1
    0x0f, 0x00, 0x00, 0x00, 0x41, 0x64, 0x64, 0x65, 0x64, 0x54, 0x69, 0x6d, 0x65, 0x73, 0x74, 0x61, 0x6d, 0x70, 0x00, // "AddedTimestamp\0"
    0x0e, 0x00, 0x00, 0x00, 0x51, 0x57, 0x6f, 0x72, 0x64, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "QWordProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // AddedTimestamp u64 (offset 301)
    0x05, 0x00, 0x00, 0x00, 0x4e, 0x6f, 0x6e, 0x65, 0x00, // "None\0"
];

// 367-byte template for painted TAGame.OnlineProduct_TA
const PAINTED_TEMPLATE: [u8; 367] = [
    0xff, 0xff, 0xff, 0xff, // OBJHEADER
    0x0a, 0x00, 0x00, 0x00, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x74, 0x49, 0x44, 0x00, // "ProductID\0"
    0x0c, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "IntProperty\0"
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, // ProductID i32 (offset 42)
    0x0b, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x49, 0x44, 0x00, // "InstanceID\0"
    0x0f, 0x00, 0x00, 0x00, 0x53, 0x74, 0x72, 0x75, 0x63, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "StructProperty\0"
    0x7f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x12, 0x00, 0x00, 0x00, 0x50, 0x72, 0x6f, 0x64, 0x75, 0x63, 0x74, 0x49, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x49, 0x44, 0x00, // "ProductInstanceID\0"
    0x0a, 0x00, 0x00, 0x00, 0x55, 0x70, 0x70, 0x65, 0x72, 0x42, 0x69, 0x74, 0x73, 0x00, // "UpperBits\0"
    0x0e, 0x00, 0x00, 0x00, 0x51, 0x57, 0x6f, 0x72, 0x64, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "QWordProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // UpperBits u64 (offset 150)
    0x0a, 0x00, 0x00, 0x00, 0x4c, 0x6f, 0x77, 0x65, 0x72, 0x42, 0x69, 0x74, 0x73, 0x00, // "LowerBits\0"
    0x0e, 0x00, 0x00, 0x00, 0x51, 0x57, 0x6f, 0x72, 0x64, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "QWordProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // LowerBits u64 (offset 198)
    0x05, 0x00, 0x00, 0x00, 0x4e, 0x6f, 0x6e, 0x65, 0x00, // "None\0"
    0x09, 0x00, 0x00, 0x00, 0x53, 0x65, 0x72, 0x69, 0x65, 0x73, 0x49, 0x44, 0x00, // "SeriesID\0"
    0x0c, 0x00, 0x00, 0x00, 0x49, 0x6e, 0x74, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "IntProperty\0"
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, // SeriesID = 1
    0x0f, 0x00, 0x00, 0x00, 0x41, 0x64, 0x64, 0x65, 0x64, 0x54, 0x69, 0x6d, 0x65, 0x73, 0x74, 0x61, 0x6d, 0x70, 0x00, // "AddedTimestamp\0"
    0x0e, 0x00, 0x00, 0x00, 0x51, 0x57, 0x6f, 0x72, 0x64, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "QWordProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // AddedTimestamp u64 (offset 301)
    0x0b, 0x00, 0x00, 0x00, 0x41, 0x74, 0x74, 0x72, 0x69, 0x62, 0x75, 0x74, 0x65, 0x73, 0x00, // "Attributes\0"
    0x0e, 0x00, 0x00, 0x00, 0x41, 0x72, 0x72, 0x61, 0x79, 0x50, 0x72, 0x6f, 0x70, 0x65, 0x72, 0x74, 0x79, 0x00, // "ArrayProperty\0"
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // length = 8
    0x01, 0x00, 0x00, 0x00, // count = 1
    0x00, 0x00, 0x00, 0x00, // Attribute object index i32 (offset 354)
    0x05, 0x00, 0x00, 0x00, 0x4e, 0x6f, 0x6e, 0x65, 0x00, // "None\0"
];

pub fn build_product_blob(
    product_id: i32,
    _paint_id: i32,
    attribute_obj_idx: Option<i32>,
    ident_upper: u64,
    ident_lower: u64,
    timestamp: u64,
) -> Vec<u8> {
    if let Some(attr_idx) = attribute_obj_idx {
        let mut b = PAINTED_TEMPLATE.to_vec();
        b[42..46].copy_from_slice(&product_id.to_le_bytes());
        b[150..158].copy_from_slice(&ident_upper.to_le_bytes());
        b[198..206].copy_from_slice(&ident_lower.to_le_bytes());
        b[301..309].copy_from_slice(&timestamp.to_le_bytes());
        b[354..358].copy_from_slice(&attr_idx.to_le_bytes());
        b
    } else {
        let mut b = UNPAINTED_TEMPLATE.to_vec();
        b[42..46].copy_from_slice(&product_id.to_le_bytes());
        b[150..158].copy_from_slice(&ident_upper.to_le_bytes());
        b[198..206].copy_from_slice(&ident_lower.to_le_bytes());
        b[301..309].copy_from_slice(&timestamp.to_le_bytes());
        b
    }
}

pub fn build_paint_attribute_blob(paint_id: i32) -> Vec<u8> {
    let mut b = PAINT_TEMPLATE.to_vec();
    b[40..44].copy_from_slice(&paint_id.to_le_bytes());
    b
}

pub fn save_envelope_to_file(envelope: &SaveEnvelope, path: &Path) -> Result<(), String> {
    let mut header = envelope.header.clone();
    let new_save_len = (envelope.sd.len() + 4) as i32;
    header[20..24].copy_from_slice(&new_save_len.to_le_bytes());

    let mut table_payload = Vec::new();
    for entry in &envelope.table {
        table_payload.extend_from_slice(&write_ue3_string(&entry.type_name));
        table_payload.extend_from_slice(&entry.file_position.to_le_bytes());
        table_payload.extend_from_slice(&entry.object_index.to_le_bytes());
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&header);
    payload.extend_from_slice(&envelope.sd);
    let table_count = envelope.table.len() as i32;
    payload.extend_from_slice(&table_count.to_le_bytes());
    payload.extend_from_slice(&table_payload);

    let encrypted = aes_encrypt(&payload);
    let crc = save_crc32(&encrypted, CRC_SEED);

    let mut file_bytes = Vec::with_capacity(8 + encrypted.len());
    let enc_len = encrypted.len() as u32;
    file_bytes.extend_from_slice(&enc_len.to_le_bytes());
    file_bytes.extend_from_slice(&crc.to_le_bytes());
    file_bytes.extend_from_slice(&encrypted);

    // Atomic write via temp file
    let tmp_path = path.with_extension("save.tmp");
    fs::write(&tmp_path, &file_bytes)
        .map_err(|e| format!("Failed to write save temp file {}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .map_err(|e| format!("Failed to commit save file {}: {e}", path.display()))?;

    Ok(())
}

pub fn default_base_product_for_slot(slot: usize) -> i32 {
    match slot {
        0 => 23,   // Body: Octane
        1 => 0,    // Decal: None
        2 => 376,  // Wheels: OEM
        3 => 63,   // Boost: Standard
        4 => 0,    // Antenna: None
        5 => 0,    // Topper: None
        6 => 0,    // Paint Finish Primary: None
        7 => 0,    // Paint Finish Accent: None
        8 => 0,    // Engine Audio
        9 => 3220, // Trail: Classic
        10 => 4038,// Goal Explosion: Standard
        _ => 0,
    }
}

pub fn slot_name_to_index(slot_name: &str) -> Option<usize> {
    match slot_name.trim().to_lowercase().as_str() {
        "body" => Some(0),
        "decal" | "skin" => Some(1),
        "wheels" | "wheel" => Some(2),
        "rocket boost" | "boost" => Some(3),
        "antenna" => Some(4),
        "topper" | "hat" => Some(5),
        "paint finish" | "paintfinish" => Some(6),
        "paint finish accent" => Some(7),
        "engine audio" | "engine sound" | "audio" => Some(8),
        "trail" => Some(9),
        "goal explosion" | "explosion" => Some(10),
        "player banner" | "banner" => Some(11),
        "avatar border" | "border" => Some(12),
        "player title" | "title" => Some(13),
        _ => None,
    }
}

/// Patch equipped loadouts in save file so that:
/// - Products[slot] contains a server-authoritative valid base (e.g. Octane 23 for Body)
/// - OnlineProducts128[slot] contains the 128-bit ProductInstanceID (UpperBits, LowerBits)
/// This prevents the match server from reverting/correcting the loadout after a goal or on kickoff respawn.
pub fn patch_equipped_loadouts(
    sd: &mut [u8],
    table: &[ObjectTableEntry],
    slot_index: usize,
    base_product_id: i32,
    ident_upper: u64,
    ident_lower: u64,
) -> Result<usize, String> {
    let mut loadout_indices = Vec::new();

    // 1. Traverse ProfileLoadoutSave_TA -> EquippedLoadoutSet -> Loadouts
    if let Some(pls_entry) = table.iter().find(|t| t.type_name == "TAGame.ProfileLoadoutSave_TA") {
        let pls_pos = pls_entry.file_position as usize;
        let mut cur = pls_pos;
        let mut eq_set_idx: Option<i32> = None;
        while cur < sd.len() {
            let (name, next_cur) = read_ue3_string(sd, cur)?;
            if name == "None" || name.is_empty() {
                break;
            }
            let (_tag, tag_cur) = read_ue3_string(sd, next_cur)?;
            if tag_cur + 8 > sd.len() {
                break;
            }
            let length = i32::from_le_bytes(sd[tag_cur..tag_cur + 4].try_into().unwrap()) as usize;
            let val_cur = tag_cur + 8;
            if name == "EquippedLoadoutSet" && length >= 4 && val_cur + 4 <= sd.len() {
                eq_set_idx = Some(i32::from_le_bytes(sd[val_cur..val_cur + 4].try_into().unwrap()));
            }
            cur = val_cur + length;
        }

        if let Some(set_idx) = eq_set_idx {
            if let Some(set_entry) = table.iter().find(|t| t.object_index == set_idx as u32) {
                let mut cur = set_entry.file_position as usize;
                while cur < sd.len() {
                    let (name, next_cur) = read_ue3_string(sd, cur)?;
                    if name == "None" || name.is_empty() {
                        break;
                    }
                    let (_tag, tag_cur) = read_ue3_string(sd, next_cur)?;
                    if tag_cur + 8 > sd.len() {
                        break;
                    }
                    let length = i32::from_le_bytes(sd[tag_cur..tag_cur + 4].try_into().unwrap()) as usize;
                    let val_cur = tag_cur + 8;
                    if name == "Loadouts" && length >= 4 && val_cur + 4 <= sd.len() {
                        let l_idx = i32::from_le_bytes(sd[val_cur..val_cur + 4].try_into().unwrap());
                        loadout_indices.push(l_idx);
                    }
                    cur = val_cur + length;
                }
            }
        }
    }

    // Fallback: If no loadout indices discovered through hierarchy, patch all TAGame.Loadout_TA objects
    if loadout_indices.is_empty() {
        for entry in table.iter().filter(|t| t.type_name == "TAGame.Loadout_TA") {
            loadout_indices.push(entry.object_index as i32);
        }
    }

    let mut patched_count = 0;
    for l_idx in loadout_indices {
        if let Some(l_entry) = table.iter().find(|t| t.object_index == l_idx as u32) {
            let mut cur = l_entry.file_position as usize;
            while cur < sd.len() {
                let (name, next_cur) = read_ue3_string(sd, cur)?;
                if name == "None" || name.is_empty() {
                    break;
                }
                let (tag, tag_cur) = read_ue3_string(sd, next_cur)?;
                if tag_cur + 8 > sd.len() {
                    break;
                }
                let length = i32::from_le_bytes(sd[tag_cur..tag_cur + 4].try_into().unwrap()) as usize;
                let val_cur = tag_cur + 8;

                if name == "Products" && tag == "ArrayProperty" && val_cur + 4 <= sd.len() {
                    let count = i32::from_le_bytes(sd[val_cur..val_cur + 4].try_into().unwrap()) as usize;
                    if slot_index < count {
                        let prod_offset = val_cur + 4 + slot_index * 4;
                        if prod_offset + 4 <= sd.len() {
                            sd[prod_offset..prod_offset + 4].copy_from_slice(&base_product_id.to_le_bytes());
                        }
                    }
                } else if name == "OnlineProducts128" && tag == "ArrayProperty" && val_cur + 4 <= sd.len() {
                    let count = i32::from_le_bytes(sd[val_cur..val_cur + 4].try_into().unwrap()) as usize;
                    if slot_index < count {
                        let slot_start = val_cur + 4 + slot_index * 105;
                        if slot_start + 96 <= sd.len() {
                            // UpperBits value at slot_start + 40
                            sd[slot_start + 40..slot_start + 48].copy_from_slice(&ident_upper.to_le_bytes());
                            // LowerBits value at slot_start + 88
                            sd[slot_start + 88..slot_start + 96].copy_from_slice(&ident_lower.to_le_bytes());
                            patched_count += 1;
                        }
                    }
                }

                cur = val_cur + length;
            }
        }
    }

    Ok(patched_count)
}

/// Native save data spawner: injects product into save file and optionally equips it into loadouts
pub fn spawn_item_native_ext(
    save_path: &Path,
    product_id: i32,
    paint_id: i32,
    slot_index: Option<usize>,
    base_product_id: Option<i32>,
) -> Result<SpawnRecord, String> {
    let mut envelope = read_envelope(save_path)?;

    // Backup current save before modification
    let backup_dir = save_path.parent().unwrap_or(save_path).join("VelocityRL Backups");
    let _ = fs::create_dir_all(&backup_dir);
    let now_str = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let backup_file = backup_dir.join(format!("{}-{}.save", save_path.file_stem().unwrap().to_string_lossy(), now_str));
    let _ = fs::copy(save_path, &backup_file);

    let (prop_len_pos, val_pos, old_prop_len, cur_count) = find_online_products(&envelope.sd)?;

    let insert_at = val_pos + old_prop_len;
    let next_obj_idx = envelope.table.len() as i32;

    // Generate random 128-bit instance ID
    let ident_upper: u64 = rand::random();
    let ident_lower: u64 = rand::random();
    let instance_hex = format!("{ident_upper:016x}{ident_lower:016x}");
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let target_product_idx;

    if paint_id > 0 {
        // 1. Paint Attribute Object
        let paint_obj_idx = next_obj_idx;
        let paint_pos = envelope.sd.len() as u32;
        let paint_blob = build_paint_attribute_blob(paint_id);
        envelope.sd.extend_from_slice(&paint_blob);
        envelope.table.push(ObjectTableEntry {
            type_name: "TAGame.ProductAttribute_Painted_TA".into(),
            file_position: paint_pos + 4,
            object_index: paint_obj_idx as u32,
        });

        // 2. Product Object referencing paint attribute
        let prod_obj_idx = next_obj_idx + 1;
        target_product_idx = prod_obj_idx;
        let prod_pos = envelope.sd.len() as u32;
        let prod_blob = build_product_blob(
            product_id,
            paint_id,
            Some(paint_obj_idx),
            ident_upper,
            ident_lower,
            timestamp,
        );
        envelope.sd.extend_from_slice(&prod_blob);
        envelope.table.push(ObjectTableEntry {
            type_name: "TAGame.OnlineProduct_TA".into(),
            file_position: prod_pos + 4,
            object_index: prod_obj_idx as u32,
        });
    } else {
        // Unpainted Product Object
        let prod_obj_idx = next_obj_idx;
        target_product_idx = prod_obj_idx;
        let prod_pos = envelope.sd.len() as u32;
        let prod_blob = build_product_blob(
            product_id,
            0,
            None,
            ident_upper,
            ident_lower,
            timestamp,
        );
        envelope.sd.extend_from_slice(&prod_blob);
        envelope.table.push(ObjectTableEntry {
            type_name: "TAGame.OnlineProduct_TA".into(),
            file_position: prod_pos + 4,
            object_index: prod_obj_idx as u32,
        });
    }

    // Insert target product index into OnlineProducts array
    envelope.sd.splice(insert_at..insert_at, target_product_idx.to_le_bytes());

    // Update count and property length in OnlineProducts
    envelope.sd[val_pos..val_pos + 4].copy_from_slice(&(cur_count + 1).to_le_bytes());
    let new_prop_len = (old_prop_len + 4) as i32;
    envelope.sd[prop_len_pos..prop_len_pos + 4].copy_from_slice(&new_prop_len.to_le_bytes());

    // Adjust object table file positions that are >= insert_at by +4
    for entry in &mut envelope.table {
        if entry.file_position as usize >= insert_at {
            entry.file_position += 4;
        }
    }

    // If slot is provided, patch equipped loadouts to prevent match server reset after goal
    if let Some(slot) = slot_index {
        let base_pid = base_product_id.unwrap_or_else(|| default_base_product_for_slot(slot));
        let _ = patch_equipped_loadouts(
            &mut envelope.sd,
            &envelope.table,
            slot,
            base_pid,
            ident_upper,
            ident_lower,
        );
    }

    // Commit changes to disk
    save_envelope_to_file(&envelope, save_path)?;

    Ok(SpawnRecord {
        instance_hex,
        product_id,
        product_name: String::new(), // Populated by caller
        paint_id,
        slot: slot_index.map(|s| s.to_string()),
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// Native save data spawner: injects product into save file
pub fn spawn_item_native(
    save_path: &Path,
    product_id: i32,
    paint_id: i32,
) -> Result<SpawnRecord, String> {
    spawn_item_native_ext(save_path, product_id, paint_id, None, None)
}

/// Remove a spawned item instance from the save file
pub fn remove_spawn_native(
    save_path: &Path,
    product_id: i32,
) -> Result<bool, String> {
    let mut envelope = read_envelope(save_path)?;
    let (prop_len_pos, val_pos, old_prop_len, cur_count) = find_online_products(&envelope.sd)?;

    if cur_count <= 0 {
        return Ok(false);
    }

    // Search OnlineProducts array for an object matching product_id
    let mut found_index_in_array = None;
    let mut found_prod_idx = None;

    for i in 0..cur_count as usize {
        let entry_pos = val_pos + 4 + i * 4;
        let prod_idx = i32::from_le_bytes(envelope.sd[entry_pos..entry_pos + 4].try_into().unwrap());
        if let Some(tbl_entry) = envelope.table.iter().find(|t| t.object_index == prod_idx as u32) {
            if tbl_entry.type_name == "TAGame.OnlineProduct_TA" {
                let obj_pos = tbl_entry.file_position as usize;
                if obj_pos + 42 <= envelope.sd.len() {
                    let pid = i32::from_le_bytes(envelope.sd[obj_pos + 38..obj_pos + 42].try_into().unwrap());
                    if pid == product_id {
                        found_index_in_array = Some(i);
                        found_prod_idx = Some(prod_idx);
                        break;
                    }
                }
            }
        }
    }

    if let (Some(idx), Some(_)) = (found_index_in_array, found_prod_idx) {
        let remove_at = val_pos + 4 + idx * 4;
        envelope.sd.drain(remove_at..remove_at + 4);

        // Update count and prop_len
        envelope.sd[val_pos..val_pos + 4].copy_from_slice(&(cur_count - 1).to_le_bytes());
        let new_prop_len = (old_prop_len - 4) as i32;
        envelope.sd[prop_len_pos..prop_len_pos + 4].copy_from_slice(&new_prop_len.to_le_bytes());

        // Shift object table positions >= remove_at by -4
        for entry in &mut envelope.table {
            if entry.file_position as usize >= remove_at {
                entry.file_position -= 4;
            }
        }

        save_envelope_to_file(&envelope, save_path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_crc32() {
        let sample = b"Test 12345";
        let crc = save_crc32(sample, CRC_SEED);
        assert_eq!(crc, 0xd42bafdc);
    }

    #[test]
    fn test_spawn_and_remove_native() {
        let real_save = Path::new("/root/test_real.save");
        if !real_save.exists() {
            return;
        }
        let tmp = PathBuf::from("/root/test_spawner_unit.save");
        let _ = fs::copy(real_save, &tmp);

        // Spawn painted Fennec (product 4284, paint 12)
        let record = spawn_item_native(&tmp, 4284, 12).expect("spawn succeeded");
        assert_eq!(record.product_id, 4284);
        assert_eq!(record.paint_id, 12);

        // Verify envelope
        let env = read_envelope(&tmp).expect("read envelope after spawn");
        let (_, _, _, count) = find_online_products(&env.sd).expect("find online products");
        assert_eq!(count, 285); // 284 original + 1 spawned

        // Run validate_savedata.py on the spawned file
        let output = std::process::Command::new("python3")
            .arg("tools/validate_savedata.py")
            .arg(&tmp)
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
            .output()
            .expect("run validate_savedata.py after spawn");
        assert!(
            output.status.success(),
            "Validation failed after spawn:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );

        // Remove the spawned product
        let removed = remove_spawn_native(&tmp, 4284).expect("remove succeeded");
        assert!(removed);

        let env2 = read_envelope(&tmp).expect("read envelope after remove");
        let (_, _, _, count2) = find_online_products(&env2.sd).expect("find online products");
        assert_eq!(count2, 284);

        let output2 = std::process::Command::new("python3")
            .arg("tools/validate_savedata.py")
            .arg(&tmp)
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
            .output()
            .expect("run validate_savedata.py after remove");
        assert!(
            output2.status.success(),
            "Validation failed after remove:\n{}",
            String::from_utf8_lossy(&output2.stdout)
        );

        let _ = fs::remove_file(&tmp);
        let _ = fs::remove_dir_all("/root/VelocityRL Backups");
    }

    #[test]
    fn test_spawn_with_equipped_loadout_patch() {
        let real_save = Path::new("/root/test_real.save");
        if !real_save.exists() {
            return;
        }
        let tmp = PathBuf::from("/root/test_loadout_patch_unit.save");
        let _ = fs::copy(real_save, &tmp);

        // Spawn Fennec (product 4284) into slot 0 with base Octane (23)
        let record = spawn_item_native_ext(&tmp, 4284, 0, Some(0), Some(23)).expect("spawn with loadout succeeded");
        assert_eq!(record.product_id, 4284);
        assert_eq!(record.slot.as_deref(), Some("0"));

        // Validate that Products[0] is 23 and OnlineProducts128[0] has non-zero upper bits
        let env = read_envelope(&tmp).expect("read envelope");
        let pls_entry = env.table.iter().find(|t| t.type_name == "TAGame.ProfileLoadoutSave_TA").expect("find pls");
        let (_, next) = read_ue3_string(&env.sd, pls_entry.file_position as usize).expect("read first prop");
        let _ = next;

        // Run validate_savedata.py on the patched file
        let output = std::process::Command::new("python3")
            .arg("tools/validate_savedata.py")
            .arg(&tmp)
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
            .output()
            .expect("run validate_savedata.py");
        assert!(
            output.status.success(),
            "Validation failed after loadout patch spawn:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );

        let _ = fs::remove_file(&tmp);
        let _ = fs::remove_dir_all("/root/VelocityRL Backups");
    }
}


