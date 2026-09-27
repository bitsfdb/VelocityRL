use crate::upk::crypto;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const TAGAME_BACKUP_NAME: &str = "TAGame.upk.bak";
pub const TAGAME_KEY: [u8; 32] = [
    0xc7, 0xdf, 0x6b, 0x13, 0x25, 0x2a, 0xcc, 0x71,
    0x47, 0xbb, 0x51, 0xc9, 0x8a, 0xd7, 0xe3, 0x4b,
    0x7f, 0xe5, 0x00, 0xb7, 0x7f, 0xa5, 0xfa, 0xb2,
    0x93, 0xe2, 0xf2, 0x4e, 0x6b, 0x17, 0xe7, 0x79,
];

// Opcodes for UE3 64-bit bytecode
pub mod opcodes {
    pub const EX_LOCAL_VARIABLE: u8 = 0x46;
    pub const EX_INSTANCE_VARIABLE: u8 = 0x2B;
    pub const EX_STRUCT_MEMBER: u8 = 0x35;
    pub const EX_DYN_ARRAY_OP: u8 = 0x57;
    pub const EX_INT_ZERO: u8 = 0x25;
    pub const EX_INT_CONST_BYTE: u8 = 0x2C;
    pub const EX_INT_CONST: u8 = 0x1D;
    pub const EX_LET: u8 = 0x0F;
    pub const EX_RETURN: u8 = 0x04;
    pub const EX_END_OF_SCRIPT: u8 = 0x4C;
    pub const EX_NOTHING: u8 = 0x0B;
    pub const EX_JUMP_IF_NOT: u8 = 0x07;
    pub const EX_EQUAL_EQUAL_INT_INT: u8 = 0x98;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagameSwapItem {
    pub slot: String,
    #[serde(default)]
    pub slot_index: Option<i32>,
    #[serde(default)]
    pub owned_id: Option<i32>,
    #[serde(default)]
    pub product_id: i32,
    #[serde(default)]
    pub paint_id: Option<i32>,
    #[serde(default)]
    pub package_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TagameSwapperConfig {
    pub enabled: bool,
    #[serde(default)]
    pub swaps: Vec<TagameSwapItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomAvatarConfig {
    pub enabled: bool,
    pub avatar_asset_path: String,
    #[serde(default)]
    pub raw_image_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagameSwapperStatus {
    pub applied: bool,
    pub avatar_applied: bool,
    pub backup_present: bool,
    pub tagame_path: String,
    pub active_swaps: Vec<TagameSwapItem>,
    pub custom_avatar: Option<CustomAvatarConfig>,
    pub message: String,
}

#[derive(Debug)]
pub enum TagameSwapError {
    Msg(String),
}

impl std::fmt::Display for TagameSwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagameSwapError::Msg(s) => write!(f, "{s}"),
        }
    }
}

impl From<std::io::Error> for TagameSwapError {
    fn from(e: std::io::Error) -> Self {
        TagameSwapError::Msg(e.to_string())
    }
}

pub fn resolve_cooked_dir(game_dir: &Path) -> Result<PathBuf, TagameSwapError> {
    let check = |p: &Path| -> Option<PathBuf> {
        if p.join("TAGame.upk").is_file() {
            Some(p.to_path_buf())
        } else if p.join("CookedPCConsole").join("TAGame.upk").is_file() {
            Some(p.join("CookedPCConsole"))
        } else if p.join("TAGame").join("CookedPCConsole").join("TAGame.upk").is_file() {
            Some(p.join("TAGame").join("CookedPCConsole"))
        } else {
            None
        }
    };

    let base = if game_dir.is_file() {
        game_dir.parent().unwrap_or(game_dir)
    } else {
        game_dir
    };

    if let Some(hit) = check(base) {
        return Ok(hit);
    }
    let mut curr = base;
    for _ in 0..4 {
        if let Some(parent) = curr.parent() {
            if let Some(hit) = check(parent) {
                return Ok(hit);
            }
            curr = parent;
        } else {
            break;
        }
    }

    Err(TagameSwapError::Msg(format!(
        "TAGame.upk not found under {}. Point settings at CookedPCConsole.",
        game_dir.display()
    )))
}

#[derive(Debug, Clone, Copy)]
pub struct SlotSwapRule {
    pub slot_idx: u8,
    pub owned_id: Option<i32>,
    pub target_id: i32,
}

/// Emits bytecode for ConvertToClientLoadout:
/// 1. NewLoadout.Products = FromData.Products;
/// 2. For each swap:
///    if (owned_id == Some(id)) {
///        if (FromData.Products[slot_idx] == id) { NewLoadout.Products[slot_idx] = target_id; }
///    } else {
///        NewLoadout.Products[slot_idx] = target_id;
///    }
/// 3. return NewLoadout;
/// 4. 0x0B (NOP) padding to exactly max_disk_size bytes.
pub fn emit_convert_to_client_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    // 1. NewLoadout.Products = FromData.Products;
    // 0F 35 4F070000 50070000 00 01 2B 4B000000 35 4F070000 3C090000 00 00 46 4D000000
    bc.push(opcodes::EX_LET);
    bc.push(opcodes::EX_STRUCT_MEMBER);
    bc.extend_from_slice(&1871i32.to_le_bytes());
    bc.extend_from_slice(&1872i32.to_le_bytes());
    bc.extend_from_slice(&[0x00, 0x01]);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&75i32.to_le_bytes());
    bc.push(opcodes::EX_STRUCT_MEMBER);
    bc.extend_from_slice(&1871i32.to_le_bytes());
    bc.extend_from_slice(&2364i32.to_le_bytes());
    bc.extend_from_slice(&[0x00, 0x00]);
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&77i32.to_le_bytes());

    mem_sz += 57; // 28 disk -> 57 mem

    // 2. Overrides
    for rule in slot_overrides {
        if let Some(owned_id) = rule.owned_id {
            // Conditional: if (FromData.Products[slot_idx] == owned_id)
            let jump_pos = bc.len();
            bc.push(opcodes::EX_JUMP_IF_NOT);
            bc.extend_from_slice(&[0x00, 0x00]); // placeholder

            // Condition: EX_EqualEqual_IntInt (0x98)
            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);

            // LHS: FromData.Products[slot_idx]
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
            }
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&2364i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&77i32.to_le_bytes());

            // RHS: owned_id (IntConst)
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());

            // Body: NewLoadout.Products[slot_idx] = target_id;
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
            }
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let jump_target = bc.len() as u16;
            bc[jump_pos + 1..jump_pos + 3].copy_from_slice(&jump_target.to_le_bytes());

            mem_sz += if rule.slot_idx == 0 { 79 } else { 81 };
        } else {
            // Unconditional: NewLoadout.Products[slot_idx] = target_id;
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
            }
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            mem_sz += if rule.slot_idx == 0 { 38 } else { 39 };
        }
    }

    // 3. return NewLoadout;
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&75i32.to_le_bytes());

    // 4. End of script
    bc.push(opcodes::EX_END_OF_SCRIPT);

    mem_sz += 10 + 1; // Return + EOS

    if bc.len() > max_disk_size {
        return Err(TagameSwapError::Msg(format!(
            "Bytecode payload size {} exceeds max disk size {}",
            bc.len(),
            max_disk_size
        )));
    }

    let nop_count = max_disk_size - bc.len();
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

fn get_paint_rgba(paint_id: i32) -> [u8; 16] {
    let (r, g, b, a): (f32, f32, f32, f32) = match paint_id {
        1 => (0.831, 0.129, 0.169, 1.0),   // Crimson
        2 => (0.655, 0.902, 0.000, 1.0),   // Lime
        3 => (0.005, 0.005, 0.005, 1.0),   // Black
        4 => (1.000, 0.455, 0.000, 1.0),   // Orange
        5 => (0.000, 0.706, 1.000, 1.0),   // Sky Blue
        6 => (0.122, 0.271, 0.988, 1.0),   // Cobalt
        7 => (1.000, 0.945, 0.090, 1.0),   // Saffron
        8 => (0.541, 0.541, 0.541, 1.0),   // Grey
        9 => (1.000, 0.431, 0.706, 1.0),   // Pink
        10 => (0.180, 0.545, 0.341, 1.0),  // Forest Green
        11 => (0.502, 0.000, 0.502, 1.0),  // Purple
        12 => (1.500, 1.500, 1.500, 1.0),  // Titanium White
        13 => (1.000, 0.843, 0.000, 1.0),  // Gold
        14 => (0.718, 0.431, 0.475, 1.0),  // Rose Gold
        _ => (0.005, 0.005, 0.005, 1.0),
    };

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&r.to_le_bytes());
    out[4..8].copy_from_slice(&g.to_le_bytes());
    out[8..12].copy_from_slice(&b.to_le_bytes());
    out[12..16].copy_from_slice(&a.to_le_bytes());
    out
}

pub fn apply_body_paint_modification(
    cooked_dir: &Path,
    package_name: &str,
    paint_id: i32,
    keys_map_json: &str,
) -> Result<(), TagameSwapError> {
    let pkg_file_name = if package_name.ends_with(".upk") {
        package_name.to_string()
    } else {
        format!("{package_name}.upk")
    };
    let pkg_path = cooked_dir.join(&pkg_file_name);
    let backup_path = cooked_dir.join(format!("{pkg_file_name}.bak"));

    if !pkg_path.is_file() {
        return Ok(());
    }

    if !backup_path.is_file() {
        let _ = fs::copy(&pkg_path, &backup_path);
    }

    let src_path = if backup_path.is_file() { &backup_path } else { &pkg_path };
    let mut file_bytes = fs::read(src_path)?;

    if file_bytes.len() < 32 {
        return Err(TagameSwapError::Msg("UPK file too small".into()));
    }

    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
    p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
    p += 4; // skip package flags
    p += 4; // skip name count
    let name_offset = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap()) as usize;

    let enc_size = (total_header_size - name_offset + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end > file_bytes.len() {
        return Err(TagameSwapError::Msg("Encrypted header out of bounds".into()));
    }

    let keys_map = crypto::load_keys_map(keys_map_json);
    let key_name = package_name.trim_end_matches(".upk").to_lowercase();
    let key = keys_map
        .get(&key_name)
        .copied()
        .or_else(|| {
            let alt_name = key_name.replace("_sf", "");
            keys_map.get(&alt_name).copied()
        })
        .unwrap_or(TAGAME_KEY);

    let mut plain_header = crypto::decrypt_ecb(&key, &file_bytes[name_offset..enc_end]);

    // Decompress Chunk 0
    let c_off = total_header_size;
    if c_off + 24 > file_bytes.len() {
        return Err(TagameSwapError::Msg("Chunk 0 offset invalid".into()));
    }

    let magic = u32::from_le_bytes(file_bytes[c_off..c_off+4].try_into().unwrap());
    let b_sz = u32::from_le_bytes(file_bytes[c_off+4..c_off+8].try_into().unwrap());
    let u_sz = u32::from_le_bytes(file_bytes[c_off+12..c_off+16].try_into().unwrap());
    let b_csz = i32::from_le_bytes(file_bytes[c_off+16..c_off+20].try_into().unwrap()) as usize;

    if c_off + 24 + b_csz > file_bytes.len() {
        return Err(TagameSwapError::Msg("Chunk 0 compressed block truncated".into()));
    }

    let mut decoder = ZlibDecoder::new(&file_bytes[c_off+24..c_off+24+b_csz]);
    let mut decomp = Vec::new();
    decoder.read_to_end(&mut decomp)?;

    let target_rgba = get_paint_rgba(paint_id);

    // CustomColor pattern (0.0663)
    let p1 = [0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x00, 0x00, 0x80, 0x3f];
    if let Some(pos) = decomp.windows(16).position(|w| w == p1) {
        decomp[pos..pos+16].copy_from_slice(&target_rgba);
    }

    // TrimColor pattern (0.12)
    let p2 = [0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x00, 0x00, 0x80, 0x3f];
    if let Some(pos) = decomp.windows(16).position(|w| w == p2) {
        decomp[pos..pos+16].copy_from_slice(&target_rgba);
    }

    // Recompress Chunk 0
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&decomp)?;
    let new_comp = encoder.finish()?;
    let new_csz = new_comp.len() as u32;

    let mut new_chunk = Vec::new();
    new_chunk.extend_from_slice(&magic.to_le_bytes());
    new_chunk.extend_from_slice(&b_sz.to_le_bytes());
    new_chunk.extend_from_slice(&new_csz.to_le_bytes());
    new_chunk.extend_from_slice(&u_sz.to_le_bytes());
    new_chunk.extend_from_slice(&(new_csz as i32).to_le_bytes());
    new_chunk.extend_from_slice(&(decomp.len() as i32).to_le_bytes());
    new_chunk.extend_from_slice(&new_comp);

    let total_chunk_disk = new_chunk.len();
    let orig_chunk_disk = 16 + 8 + b_csz;

    if total_chunk_disk <= orig_chunk_disk {
        new_chunk.resize(orig_chunk_disk, 0);
        file_bytes[c_off..c_off+orig_chunk_disk].copy_from_slice(&new_chunk);
    } else {
        file_bytes[c_off..c_off+orig_chunk_disk].copy_from_slice(&new_chunk[..orig_chunk_disk]);
    }

    // Update chunk table in plain header
    for p_candidate in 0..(plain_header.len().saturating_sub(24)) {
        let u_s = u32::from_le_bytes(plain_header[p_candidate+8..p_candidate+12].try_into().unwrap());
        let c_o = u64::from_le_bytes(plain_header[p_candidate+12..p_candidate+20].try_into().unwrap()) as usize;
        if c_o == c_off && u_s == u_sz {
            plain_header[p_candidate+20..p_candidate+24].copy_from_slice(&(total_chunk_disk as i32).to_le_bytes());
            break;
        }
    }

    let re_enc = crypto::encrypt_ecb(&key, &plain_header);
    file_bytes[name_offset..enc_end].copy_from_slice(&re_enc);

    fs::write(&pkg_path, &file_bytes)?;
    Ok(())
}

/// Applies loadout modifications cleanly to TAGame.upk and associated body UPKs.
pub fn apply_tagame_modifications(
    cooked_dir: &Path,
    swaps: &[TagameSwapItem],
    avatar_config: Option<&CustomAvatarConfig>,
    _keys_txt: &str,
    keys_map_json: &str,
) -> Result<TagameSwapperStatus, TagameSwapError> {
    let tagame_path = cooked_dir.join("TAGame.upk");
    let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);

    if !tagame_path.is_file() {
        return Err(TagameSwapError::Msg(format!(
            "TAGame.upk not found at {}",
            tagame_path.display()
        )));
    }

    // 1. Ensure pristine backup exists
    if !backup_path.is_file() {
        fs::copy(&tagame_path, &backup_path).map_err(|e| {
            TagameSwapError::Msg(format!("Failed to create backup {}: {e}", backup_path.display()))
        })?;
    }

    // 2. Read live file so other modifications (e.g. custom color palette) are preserved
    let mut file_bytes = fs::read(&tagame_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read {}: {e}", tagame_path.display()))
    })?;

    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
    p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
    p += 4; // skip flags
    p += 4; // skip name count
    let name_offset = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap()) as usize;

    let garbage_size = 559792;
    let enc_size = total_header_size - garbage_size - name_offset;
    let enc_aligned = (enc_size + 15) & !15;
    let enc_end = name_offset + enc_aligned;

    if enc_end > file_bytes.len() {
        return Err(TagameSwapError::Msg("TAGame encrypted header OOB".into()));
    }

    let mut plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);

    let chunk_count_pos = 0xAB9FA8;
    let c_pos_0 = chunk_count_pos + 4;
    let u_off_0 = u64::from_le_bytes(plain_header[c_pos_0..c_pos_0+8].try_into().unwrap()) as usize;
    let c_off_0 = u64::from_le_bytes(plain_header[c_pos_0+12..c_pos_0+20].try_into().unwrap()) as usize;
    let orig_c_sz_0 = i32::from_le_bytes(plain_header[c_pos_0+20..c_pos_0+24].try_into().unwrap()) as usize;

    let target_func_offset = 0xABC8F9;
    let c_data_0 = &file_bytes[c_off_0..c_off_0 + orig_c_sz_0];

    let magic = u32::from_le_bytes(c_data_0[0..4].try_into().unwrap());
    let block_size = u32::from_le_bytes(c_data_0[4..8].try_into().unwrap()) as usize;
    let total_uncomp = u32::from_le_bytes(c_data_0[12..16].try_into().unwrap()) as usize;
    let num_blocks = (total_uncomp + block_size - 1) / block_size;

    let mut pos = 16;
    let mut blocks = Vec::new();
    for _ in 0..num_blocks {
        let b_csz = i32::from_le_bytes(c_data_0[pos..pos+4].try_into().unwrap()) as usize;
        let b_usz = i32::from_le_bytes(c_data_0[pos+4..pos+8].try_into().unwrap()) as usize;
        blocks.push((b_csz, b_usz));
        pos += 8;
    }

    let payload_pos = pos;
    let mut decoder = ZlibDecoder::new(&c_data_0[payload_pos..payload_pos + blocks[0].0]);
    let mut block_0_decomp = Vec::new();
    decoder.read_to_end(&mut block_0_decomp)?;

    let func_off_1 = target_func_offset - u_off_0;

    // Collect slot overrides
    let mut slot_overrides = Vec::new();
    for s in swaps {
        let slot_idx = match s.slot.to_lowercase().as_str() {
            "body" | "0" => 0,
            "skin" | "decal" | "1" => 1,
            "wheel" | "wheels" | "2" => 2,
            "boost" | "rocket boost" | "rocketboost" | "3" => 3,
            "antenna" | "4" => 4,
            "topper" | "5" => 5,
            "paint finish" | "paintfinish" | "paint" | "6" => 6,
            "engine audio" | "engineaudio" | "8" => 8,
            "trail" | "9" => 9,
            "goal explosion" | "goalexplosion" | "10" => 10,
            "player banner" | "playerbanner" | "banner" | "11" => 11,
            "player anthem" | "playeranthem" | "anthem" | "music" | "12" => 12,
            "avatar border" | "avatarborder" | "border" | "13" => 13,
            _ => s.slot_index.unwrap_or(0) as u8,
        };
        let pid = if s.product_id > 0 { s.product_id } else { 4284 };
        slot_overrides.push(SlotSwapRule {
            slot_idx,
            owned_id: s.owned_id,
            target_id: pid,
        });
    }

    if slot_overrides.is_empty() {
        slot_overrides.push(SlotSwapRule {
            slot_idx: 0,
            owned_id: None,
            target_id: 4284,
        });
    }

    let (payload, mem_sz) = emit_convert_to_client_loadout_bytecode(&slot_overrides, 124)?;

    // Update function header at func_off_1 + 40
    block_0_decomp[func_off_1 + 40..func_off_1 + 44].copy_from_slice(&mem_sz.to_le_bytes());
    block_0_decomp[func_off_1 + 44..func_off_1 + 48].copy_from_slice(&124u32.to_le_bytes());
    block_0_decomp[func_off_1 + 48..func_off_1 + 48 + 124].copy_from_slice(&payload);

    // Recompress Block 0
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&block_0_decomp)?;
    let new_comp_b0 = encoder.finish()?;

    let mut new_c_data_0 = Vec::new();
    new_c_data_0.extend_from_slice(&magic.to_le_bytes());
    new_c_data_0.extend_from_slice(&(block_size as u32).to_le_bytes());
    new_c_data_0.extend_from_slice(&0u32.to_le_bytes()); // placeholder
    new_c_data_0.extend_from_slice(&(total_uncomp as u32).to_le_bytes());

    let table_pos = new_c_data_0.len();
    new_c_data_0.resize(table_pos + num_blocks * 8, 0);

    let mut new_blocks = Vec::new();
    let mut cur_payload = payload_pos;
    for idx in 0..num_blocks {
        let (c_bytes, u_sz) = if idx == 0 {
            (new_comp_b0.as_slice(), block_0_decomp.len())
        } else {
            let b_csz = blocks[idx].0;
            (&c_data_0[cur_payload..cur_payload + b_csz], blocks[idx].1)
        };
        new_blocks.push((c_bytes.len(), u_sz));
        cur_payload += blocks[idx].0;
        new_c_data_0.extend_from_slice(c_bytes);
    }

    for (idx, &(b_csz, b_usz)) in new_blocks.iter().enumerate() {
        let off = table_pos + idx * 8;
        new_c_data_0[off..off+4].copy_from_slice(&(b_csz as i32).to_le_bytes());
        new_c_data_0[off+4..off+8].copy_from_slice(&(b_usz as i32).to_le_bytes());
    }

    let sum_comp_payload = new_blocks.iter().map(|b| b.0).sum::<usize>();
    let total_chunk_disk = 16 + num_blocks * 8 + sum_comp_payload;

    new_c_data_0[8..12].copy_from_slice(&(sum_comp_payload as u32).to_le_bytes());

    if total_chunk_disk > orig_c_sz_0 {
        return Err(TagameSwapError::Msg("Chunk 0 compressed size exceeded allocation".into()));
    }
    new_c_data_0.resize(orig_c_sz_0, 0);

    file_bytes[c_off_0..c_off_0 + orig_c_sz_0].copy_from_slice(&new_c_data_0);
    plain_header[c_pos_0 + 20..c_pos_0 + 24].copy_from_slice(&(total_chunk_disk as i32).to_le_bytes());

    let re_enc = crypto::encrypt_ecb(&TAGAME_KEY, &plain_header);
    file_bytes[name_offset..enc_end].copy_from_slice(&re_enc);

    // Write TAGame.upk
    fs::write(&tagame_path, &file_bytes)?;

    // 3. Apply material paint overrides (e.g. Fennec Black / body_grain_SF)
    for s in swaps {
        let pkg = s.package_name.as_deref().unwrap_or("body_grain_SF");
        if let Some(paint_id) = s.paint_id {
            if paint_id > 0 {
                let _ = apply_body_paint_modification(cooked_dir, pkg, paint_id, keys_map_json);
            } else {
                let pkg_file_name = if pkg.ends_with(".upk") { pkg.to_string() } else { format!("{pkg}.upk") };
                let pkg_path = cooked_dir.join(&pkg_file_name);
                let bak_path = cooked_dir.join(format!("{pkg_file_name}.bak"));
                if bak_path.is_file() {
                    let _ = fs::copy(&bak_path, &pkg_path);
                }
            }
        }
    }

    let applied_swaps = swaps.to_vec();
    let avatar_applied = avatar_config.map(|a| a.enabled).unwrap_or(false);

    Ok(TagameSwapperStatus {
        applied: !applied_swaps.is_empty(),
        avatar_applied,
        backup_present: backup_path.is_file(),
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: applied_swaps,
        custom_avatar: avatar_config.cloned(),
        message: "Loadout and paint modifications applied successfully.".to_string(),
    })
}

/// Restores TAGame.upk and body package UPKs from their backups.
pub fn restore_tagame_upk(cooked_dir: &Path) -> Result<TagameSwapperStatus, TagameSwapError> {
    let tagame_path = cooked_dir.join("TAGame.upk");
    let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);

    if !backup_path.is_file() {
        return Ok(TagameSwapperStatus {
            applied: false,
            avatar_applied: false,
            backup_present: false,
            tagame_path: tagame_path.to_string_lossy().into_owned(),
            active_swaps: Vec::new(),
            custom_avatar: None,
            message: "No backup found to restore.".to_string(),
        });
    }

    fs::copy(&backup_path, &tagame_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to restore TAGame.upk from backup: {e}"))
    })?;

    // Restore body_grain_SF.upk if backup exists
    let grain_path = cooked_dir.join("body_grain_SF.upk");
    let grain_bak = cooked_dir.join("body_grain_SF.upk.bak");
    if grain_bak.is_file() {
        let _ = fs::copy(&grain_bak, &grain_path);
    }

    Ok(TagameSwapperStatus {
        applied: false,
        avatar_applied: false,
        backup_present: true,
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: Vec::new(),
        custom_avatar: None,
        message: "TAGame.upk and body packages restored successfully from backup.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_convert_to_client_loadout_bytecode_unconditional() {
        let rules = [SlotSwapRule {
            slot_idx: 0,
            owned_id: None,
            target_id: 4284,
        }];
        let (bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&rules, 124).unwrap();
        assert_eq!(bc.len(), 124);
        assert_eq!(mem_sz, 164);
        assert_eq!(bc[0], opcodes::EX_LET);
    }

    #[test]
    fn test_emit_convert_to_client_loadout_bytecode_conditional() {
        let rules = [SlotSwapRule {
            slot_idx: 3,
            owned_id: Some(100),
            target_id: 400,
        }];
        let (bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&rules, 124).unwrap();
        assert_eq!(bc.len(), 124);
        assert_eq!(bc[33], opcodes::EX_JUMP_IF_NOT);
        assert!(mem_sz > 0);
    }
}
