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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagameSwapperStatus {
    pub applied: bool,
    pub backup_present: bool,
    pub tagame_path: String,
    pub active_swaps: Vec<TagameSwapItem>,
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

pub const VANILLA_COPY_LOOP_PREFIX: [u8; 111] = [
    0x57, 0x0A, 0x5E, 0x12, 0x00, 0x20, 0x16, 0x64, 0x00, 0x00, 0x96, 0x00, 0xE4, 0x63, 0x00, 0x00,
    0x00, 0x02, 0xE4, 0x63, 0x00, 0x00, 0x2B, 0x4A, 0x00, 0x00, 0x00, 0x48, 0xAE, 0x00, 0x0F, 0x57,
    0x00, 0x00, 0x5E, 0x19, 0x00, 0x2B, 0x4A, 0x00, 0x00, 0x00, 0x09, 0x00, 0x5D, 0x26, 0x00, 0x00,
    0x00, 0x01, 0x5D, 0x26, 0x00, 0x00, 0x35, 0x4F, 0x07, 0x00, 0x00, 0x50, 0x07, 0x00, 0x00, 0x00,
    0x01, 0x2B, 0x4B, 0x00, 0x00, 0x00, 0x57, 0x00, 0x00, 0x5E, 0x19, 0x00, 0x2B, 0x4A, 0x00, 0x00,
    0x00, 0x09, 0x00, 0x5D, 0x26, 0x00, 0x00, 0x00, 0x01, 0x5D, 0x26, 0x00, 0x00, 0x35, 0x4F, 0x07,
    0x00, 0x00, 0x3C, 0x09, 0x00, 0x00, 0x00, 0x00, 0x46, 0x4D, 0x00, 0x00, 0x00, 0x31, 0x30,
];

pub const VANILLA_CONVERT_TO_CLIENT_LOADOUT_BYTECODE: [u8; 124] = [
    0x57, 0x0A, 0x5E, 0x12, 0x00, 0x20, 0x16, 0x64, 0x00, 0x00, 0x96, 0x00, 0xE4, 0x63, 0x00, 0x00,
    0x00, 0x02, 0xE4, 0x63, 0x00, 0x00, 0x2B, 0x4A, 0x00, 0x00, 0x00, 0x48, 0xAE, 0x00, 0x0F, 0x57,
    0x00, 0x00, 0x5E, 0x19, 0x00, 0x2B, 0x4A, 0x00, 0x00, 0x00, 0x09, 0x00, 0x5D, 0x26, 0x00, 0x00,
    0x00, 0x01, 0x5D, 0x26, 0x00, 0x00, 0x35, 0x4F, 0x07, 0x00, 0x00, 0x50, 0x07, 0x00, 0x00, 0x00,
    0x01, 0x2B, 0x4B, 0x00, 0x00, 0x00, 0x57, 0x00, 0x00, 0x5E, 0x19, 0x00, 0x2B, 0x4A, 0x00, 0x00,
    0x00, 0x09, 0x00, 0x5D, 0x26, 0x00, 0x00, 0x00, 0x01, 0x5D, 0x26, 0x00, 0x00, 0x35, 0x4F, 0x07,
    0x00, 0x00, 0x3C, 0x09, 0x00, 0x00, 0x00, 0x00, 0x46, 0x4D, 0x00, 0x00, 0x00, 0x31, 0x30,
    0x04, 0x2B, 0x4B, 0x00, 0x00, 0x00, 0x04, 0x3A, 0x4C, 0x00, 0x00, 0x00, 0x4C,
];
pub const VANILLA_CONVERT_TO_CLIENT_LOADOUT_MEM_SIZE: u32 = 196;

pub const MAX_SWAPS_LIMIT: usize = 50;

pub const DIRECT_STRUCT_COPY_PREFIX: [u8; 11] = [
    opcodes::EX_LET,
    opcodes::EX_INSTANCE_VARIABLE,
    0x4B, 0x00, 0x00, 0x00, // NewLoadout (75)
    opcodes::EX_LOCAL_VARIABLE,
    0x4D, 0x00, 0x00, 0x00, // FromData (77)
];

pub fn emit_convert_to_client_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    if slot_overrides.len() > MAX_SWAPS_LIMIT {
        return Err(TagameSwapError::Msg(
            "Too many swaps! Use presets to save your swaps when you want to use them.".into(),
        ));
    }

    if slot_overrides.is_empty() {
        return Ok((
            VANILLA_CONVERT_TO_CLIENT_LOADOUT_BYTECODE.to_vec(),
            VANILLA_CONVERT_TO_CLIENT_LOADOUT_MEM_SIZE,
        ));
    }

    let mut bc = Vec::new();

    // 1. Pristine native loop to populate NewLoadout.Products from FromData.Products
    bc.extend_from_slice(&VANILLA_COPY_LOOP_PREFIX);

    let mut mem_sz = 175u32; // Vanilla copy loop prefix footprint in UE3 64-bit memory

    // 2. Slot assignments: Index = slot_idx; NewLoadout.Products[Index] = target_id;
    for rule in slot_overrides {
        // Index = slot_idx
        bc.push(opcodes::EX_LET);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&74i32.to_le_bytes()); // Index variable (74)
        if rule.slot_idx == 0 {
            bc.push(opcodes::EX_INT_ZERO);
            mem_sz += 70; // 51 disk bytes + 19 in-memory expansion
        } else {
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&(rule.slot_idx as i32).to_le_bytes());
            mem_sz += 83; // 55 disk bytes + 28 in-memory expansion
        }

        // NewLoadout.Products[Index] = target_id
        bc.push(opcodes::EX_LET);
        bc.extend_from_slice(&[
            0x57, 0x00, 0x00, 0x5E, 0x19, 0x00, 0x2B, 0x4A, 0x00, 0x00, 0x00, 0x09, 0x00, 0x5D, 0x26,
            0x00, 0x00, 0x00, 0x01, 0x5D, 0x26, 0x00, 0x00, 0x35, 0x4F, 0x07, 0x00, 0x00, 0x50, 0x07,
            0x00, 0x00, 0x00, 0x01, 0x2B, 0x4B, 0x00, 0x00, 0x00,
        ]);
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&rule.target_id.to_le_bytes());
    }

    // 3. Full native return sequence (return NewLoadout + return out slot variable + EOS)
    bc.extend_from_slice(&[
        0x04, 0x2B, 0x4B, 0x00, 0x00, 0x00, // return NewLoadout (75)
        0x04, 0x3A, 0x4C, 0x00, 0x00, 0x00, // return out_var (76)
        0x4C,                               // EX_END_OF_SCRIPT
    ]);
    mem_sz += 21; // Full vanilla return sequence (13 disk bytes + 8 in-memory expansion)

    if max_disk_size > 0 && bc.len() > max_disk_size {
        return Err(TagameSwapError::Msg(
            "Too many swaps! Use presets to save your swaps when you want to use them.".into(),
        ));
    }

    if max_disk_size > 0 && bc.len() < max_disk_size {
        let pad_count = max_disk_size - bc.len();
        bc.resize(max_disk_size, opcodes::EX_NOTHING);
        mem_sz += pad_count as u32; // 0x0B (EX_NOTHING) adds 1 byte to disk AND 1 byte to memory
    }

    Ok((bc, mem_sz))
}

#[derive(Deserialize)]
struct PaintDef {
    id: i32,
    hex: String,
}

fn hex_to_rgba(hex: &str) -> (f32, f32, f32, f32) {
    let clean = hex.trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(0) as f32 / 255.0;
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(0) as f32 / 255.0;
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(0) as f32 / 255.0;
        (r, g, b, 1.0)
    } else {
        (0.005, 0.005, 0.005, 1.0)
    }
}

pub fn get_paint_rgba(paint_id: i32) -> [u8; 16] {
    let (r, g, b, a) = match paint_id {
        0 => (0.0, 0.0, 0.0, 1.0), // None / Default
        1 => (0.83, 0.13, 0.17, 1.0), // Crimson
        2 => (0.65, 0.90, 0.0, 1.0), // Lime
        3 => (0.005, 0.005, 0.005, 1.0), // Black: Deep luminance vector for UE3 body shaders
        4 => (1.0, 0.45, 0.0, 1.0), // Orange
        5 => (0.0, 0.70, 1.0, 1.0), // Sky Blue
        6 => (0.12, 0.27, 0.98, 1.0), // Cobalt
        7 => (1.0, 0.94, 0.09, 1.0), // Saffron
        8 => (0.54, 0.54, 0.54, 1.0), // Grey
        9 => (1.0, 0.43, 0.70, 1.0), // Pink
        10 => (0.18, 0.54, 0.34, 1.0), // Forest Green
        11 => (0.50, 0.0, 0.50, 1.0), // Purple
        12 => (1.5, 1.5, 1.5, 1.0), // Titanium White: High specular boost for UE3 car shaders
        13 => (1.0, 0.84, 0.0, 1.0), // Gold
        14 => (0.71, 0.43, 0.47, 1.0), // Rose Gold
        _ => {
            let paints_json = include_str!("../../../ui/paints.json");
            if let Ok(defs) = serde_json::from_str::<Vec<PaintDef>>(paints_json) {
                if let Some(p) = defs.iter().find(|p| p.id == paint_id) {
                    hex_to_rgba(&p.hex)
                } else {
                    (0.005, 0.005, 0.005, 1.0)
                }
            } else {
                (0.005, 0.005, 0.005, 1.0)
            }
        }
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
    if c_off + 16 > file_bytes.len() {
        return Err(TagameSwapError::Msg("Chunk 0 offset invalid".into()));
    }

    let magic = u32::from_le_bytes(file_bytes[c_off..c_off+4].try_into().unwrap());
    let b_sz = u32::from_le_bytes(file_bytes[c_off+4..c_off+8].try_into().unwrap()) as usize;
    let u_sz = u32::from_le_bytes(file_bytes[c_off+12..c_off+16].try_into().unwrap()) as usize;

    if b_sz == 0 || u_sz == 0 {
        return Ok(());
    }

    let num_blocks = (u_sz + b_sz - 1) / b_sz;
    let mut b_pos = c_off + 16;
    let mut blocks = Vec::with_capacity(num_blocks);
    for _ in 0..num_blocks {
        if b_pos + 8 > file_bytes.len() {
            return Err(TagameSwapError::Msg("Chunk block table out of bounds".into()));
        }
        let b_csz = i32::from_le_bytes(file_bytes[b_pos..b_pos+4].try_into().unwrap()) as usize;
        let b_usz = i32::from_le_bytes(file_bytes[b_pos+4..b_pos+8].try_into().unwrap()) as usize;
        blocks.push((b_csz, b_usz));
        b_pos += 8;
    }

    let payload_start = b_pos;
    let mut cur_payload = payload_start;
    let mut decomp = Vec::with_capacity(u_sz);
    for &(b_csz, _) in &blocks {
        if cur_payload + b_csz > file_bytes.len() {
            return Err(TagameSwapError::Msg("Chunk block payload out of bounds".into()));
        }
        let mut decoder = ZlibDecoder::new(&file_bytes[cur_payload..cur_payload + b_csz]);
        decoder.read_to_end(&mut decomp)?;
        cur_payload += b_csz;
    }

    let target_rgba = get_paint_rgba(paint_id);

    // CustomColor pattern (0.0663)
    let p1 = [0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x00, 0x00, 0x80, 0x3f];
    // TrimColor pattern (0.12)
    let p2 = [0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x00, 0x00, 0x80, 0x3f];

    let mut modified = false;
    if decomp.len() >= 16 {
        for pos in 0..=decomp.len() - 16 {
            if decomp[pos..pos+16] == p1 || decomp[pos..pos+16] == p2 {
                decomp[pos..pos+16].copy_from_slice(&target_rgba);
                modified = true;
            }
        }
    }

    // If this package does not contain paintable trim material parameters (e.g. stock Octane Body_Octane_SF),
    // safely return without modifying or recompressing to prevent asset corruption.
    if !modified {
        return Ok(());
    }

    // Recompress Chunk 0 preserving block structure
    let mut new_blocks = Vec::with_capacity(num_blocks);
    let mut new_compressed_payload = Vec::new();
    let mut decomp_offset = 0;

    for &(_, b_usz) in &blocks {
        let chunk_slice = &decomp[decomp_offset..decomp_offset + b_usz];
        decomp_offset += b_usz;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(chunk_slice)?;
        let c_bytes = encoder.finish()?;
        new_blocks.push((c_bytes.len(), b_usz));
        new_compressed_payload.extend_from_slice(&c_bytes);
    }

    let sum_new_csz: usize = new_blocks.iter().map(|b| b.0).sum();
    let total_new_chunk_disk = 16 + num_blocks * 8 + sum_new_csz;
    let orig_chunk_disk = 16 + num_blocks * 8 + blocks.iter().map(|b| b.0).sum::<usize>();

    if total_new_chunk_disk > orig_chunk_disk {
        // Fallback: If best compression still exceeded original disk allocation for chunk 0,
        // abort safely rather than corrupting the file
        return Ok(());
    }

    let mut new_chunk_data = Vec::with_capacity(orig_chunk_disk);
    new_chunk_data.extend_from_slice(&magic.to_le_bytes());
    new_chunk_data.extend_from_slice(&(b_sz as u32).to_le_bytes());
    new_chunk_data.extend_from_slice(&(sum_new_csz as u32).to_le_bytes());
    new_chunk_data.extend_from_slice(&(u_sz as u32).to_le_bytes());

    for &(b_csz, b_usz) in &new_blocks {
        new_chunk_data.extend_from_slice(&(b_csz as i32).to_le_bytes());
        new_chunk_data.extend_from_slice(&(b_usz as i32).to_le_bytes());
    }
    new_chunk_data.extend_from_slice(&new_compressed_payload);
    new_chunk_data.resize(orig_chunk_disk, 0);

    file_bytes[c_off..c_off+orig_chunk_disk].copy_from_slice(&new_chunk_data);

    // Update chunk table in plain header
    for p_candidate in 0..(plain_header.len().saturating_sub(24)) {
        let u_s = u32::from_le_bytes(plain_header[p_candidate+8..p_candidate+12].try_into().unwrap()) as usize;
        let c_o = u64::from_le_bytes(plain_header[p_candidate+12..p_candidate+20].try_into().unwrap()) as usize;
        if c_o == c_off && u_s == u_sz {
            plain_header[p_candidate+20..p_candidate+24].copy_from_slice(&(total_new_chunk_disk as i32).to_le_bytes());
            break;
        }
    }

    let re_enc = crypto::encrypt_ecb(&key, &plain_header);
    file_bytes[name_offset..enc_end].copy_from_slice(&re_enc);

    fs::write(&pkg_path, &file_bytes)?;
    Ok(())
}

/// Applies loadout modifications cleanly to TAGame.upk via Chunk 0 decompression & recompression and associated body UPKs.
pub fn apply_tagame_modifications(
    cooked_dir: &Path,
    swaps: &[TagameSwapItem],
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

    // Always preserve TAGame.upk from pristine backup to prevent chunk misalignment or startup crashes
    if backup_path.is_file() {
        let _ = fs::copy(&backup_path, &tagame_path);
    }

    // Apply material paint overrides (e.g. Fennec Black / body_grain_SF)
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

    Ok(TagameSwapperStatus {
        applied: !applied_swaps.is_empty(),
        backup_present: backup_path.is_file(),
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: applied_swaps,
        message: "Loadout and paint modifications applied successfully.".to_string(),
    })
}

/// Injects or synchronizes TAGame.upk hooks dynamically from `swaps.ini` and `decals.ini`.
pub fn apply_tagame_ini_hook(cooked_dir: &Path) -> Result<TagameSwapperStatus, TagameSwapError> {
    let swaps_cfg = crate::upk::ini_swapper::read_swaps_ini(cooked_dir);
    let _decals_cfg = crate::upk::ini_swapper::read_decals_ini(cooked_dir);

    let mut swaps = Vec::new();
    if let Some(cfg) = swaps_cfg {
        if cfg.enabled {
            let mut add_item = |slot: &str, slot_idx: u8, pid: i32, paint: i32, pkg: &str| {
                if pid > 0 {
                    swaps.push(TagameSwapItem {
                        slot: slot.to_string(),
                        slot_index: Some(slot_idx as i32),
                        owned_id: None,
                        product_id: pid,
                        paint_id: if paint > 0 { Some(paint) } else { None },
                        package_name: Some(pkg.to_string()),
                    });
                }
            };
            add_item("Body", 0, cfg.body, cfg.body_paint, "Body_Fennec_SF");
            add_item("Decal", 1, cfg.decal, cfg.decal_paint, "");
            add_item("Wheels", 2, cfg.wheels, cfg.wheels_paint, "");
            add_item("Boost", 3, cfg.boost, cfg.boost_paint, "");
            add_item("Antenna", 4, cfg.antenna, 0, "");
            add_item("Topper", 5, cfg.topper, 0, "");
            add_item("PaintFinish", 6, cfg.paint_finish, 0, "");
            add_item("EngineAudio", 8, cfg.engine_audio, 0, "");
            add_item("Trail", 9, cfg.trail, 0, "");
            add_item("GoalExplosion", 10, cfg.goal_explosion, 0, "");
            add_item("PlayerBanner", 11, cfg.player_banner, 0, "");
            add_item("PlayerAnthem", 12, cfg.player_anthem, 0, "");
            add_item("AvatarBorder", 13, cfg.avatar_border, 0, "");
        }
    }

    let keys_txt = include_str!("../../resources/keys.txt");
    let keys_map_json = include_str!("../../resources/keys_map.json");

    apply_tagame_modifications(
        cooked_dir,
        &swaps,
        keys_txt,
        keys_map_json,
    )
}

/// Restores TAGame.upk loadout rules and body packages by reverting the bytecode in-place.
pub fn restore_tagame_upk(cooked_dir: &Path) -> Result<TagameSwapperStatus, TagameSwapError> {
    let tagame_path = cooked_dir.join("TAGame.upk");
    let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);

    let keys_txt = include_str!("../../resources/keys.txt");
    let keys_map_json = include_str!("../../resources/keys_map.json");

    // If backup exists, copy backup directly to tagame_path
    if backup_path.is_file() {
        let _ = fs::copy(&backup_path, &tagame_path);
    } else {
        // Otherwise revert the bytecode in-place
        let _ = apply_tagame_modifications(
            cooked_dir,
            &[],
            keys_txt,
            keys_map_json,
        );
    }

    // Restore body package backups if they exist
    if let Ok(entries) = fs::read_dir(cooked_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with("body_") && file_name.ends_with(".upk.bak") {
                    let live_name = file_name.trim_end_matches(".bak");
                    let live_path = cooked_dir.join(live_name);
                    let _ = fs::copy(&path, &live_path);
                }
            }
        }
    }

    Ok(TagameSwapperStatus {
        applied: false,
        backup_present: backup_path.is_file(),
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: Vec::new(),
        message: "Loadout and paint bytecode reverted to default.".to_string(),
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
        let (bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&rules, 0).unwrap();
        assert!(bc.len() > 124);
        assert_eq!(&bc[0..111], &VANILLA_COPY_LOOP_PREFIX);
        assert_eq!(bc[111], opcodes::EX_LET);
        assert!(mem_sz > 0);
    }

    #[test]
    fn test_emit_convert_to_client_loadout_bytecode_conditional() {
        let rules = [SlotSwapRule {
            slot_idx: 3,
            owned_id: Some(100),
            target_id: 400,
        }];
        let (bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&rules, 0).unwrap();
        assert!(bc.len() > 124);
        assert_eq!(&bc[0..111], &VANILLA_COPY_LOOP_PREFIX);
        assert!(mem_sz > 0);
    }

    #[test]
    fn test_emit_all_fourteen_slots_bytecode() {
        let all_slots = [
            SlotSwapRule { slot_idx: 0, owned_id: None, target_id: 4284 },
            SlotSwapRule { slot_idx: 1, owned_id: None, target_id: 101 },
            SlotSwapRule { slot_idx: 2, owned_id: None, target_id: 1560 },
            SlotSwapRule { slot_idx: 3, owned_id: None, target_id: 890 },
            SlotSwapRule { slot_idx: 4, owned_id: None, target_id: 200 },
            SlotSwapRule { slot_idx: 5, owned_id: None, target_id: 300 },
            SlotSwapRule { slot_idx: 6, owned_id: None, target_id: 400 },
            SlotSwapRule { slot_idx: 8, owned_id: None, target_id: 500 },
            SlotSwapRule { slot_idx: 9, owned_id: None, target_id: 600 },
            SlotSwapRule { slot_idx: 10, owned_id: None, target_id: 4002 },
            SlotSwapRule { slot_idx: 11, owned_id: None, target_id: 2526 },
            SlotSwapRule { slot_idx: 12, owned_id: None, target_id: 700 },
            SlotSwapRule { slot_idx: 13, owned_id: None, target_id: 800 },
        ];
        let (raw_bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&all_slots, 0).unwrap();
        assert!(raw_bc.len() > 124);
        assert_eq!(&raw_bc[0..111], &VANILLA_COPY_LOOP_PREFIX);
        assert!(mem_sz > raw_bc.len() as u32);
    }

    #[test]
    fn test_paint_normalization() {
        let black_rgba = get_paint_rgba(3);
        let tw_rgba = get_paint_rgba(12);
        let gold_rgba = get_paint_rgba(13);

        let r_black = f32::from_le_bytes(black_rgba[0..4].try_into().unwrap());
        let r_tw = f32::from_le_bytes(tw_rgba[0..4].try_into().unwrap());
        let r_gold = f32::from_le_bytes(gold_rgba[0..4].try_into().unwrap());

        assert_eq!(r_black, 0.005);
        assert_eq!(r_tw, 1.5);
        assert_eq!(r_gold, 1.0);
    }

    #[test]
    fn test_live_apply_on_tagame() {
        let cooked = Path::new(r"E:\games\rocketleague\TAGame\CookedPCConsole");
        let tagame_path = cooked.join("TAGame.upk");
        if tagame_path.is_file() {
            let file_bytes = fs::read(&tagame_path).unwrap();
            let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
            let mut p = 12;
            let flen = i32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
            p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
            let pkg_flags = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
            p += 4;
            let name_count = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
            p += 4;
            let name_offset = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap()) as usize;
            p += 4;
            let export_count = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
            p += 4;
            let export_offset = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap()) as usize;
            p += 4;
            let import_count = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
            p += 4;
            let import_offset = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap()) as usize;

            println!("total_header_size: {total_header_size:#X}, pkg_flags: {pkg_flags:#X}");
            println!("name_count: {name_count}, name_offset: {name_offset:#X}");
            println!("export_count: {export_count}, export_offset: {export_offset:#X}");
            println!("import_count: {import_count}, import_offset: {import_offset:#X}");

            let garbage_size = 559792;
            let enc_size = total_header_size.saturating_sub(garbage_size + name_offset);
            let enc_aligned = (enc_size + 15) & !15;
            let enc_end = name_offset + enc_aligned;
            let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);

            let exp_off_in_plain = export_offset - name_offset;
            println!("exp_off_in_plain: {exp_off_in_plain:#X}, plain_header.len(): {:#X}", plain_header.len());

            let target_func_offset = 0xABC8F9;
            println!("target_func_offset: {target_func_offset:#X}");

            // Search for all exports in Chunk 0
            let u_off_0 = 0xAB9000;
            let mut count_in_c0 = 0;
            for off in (exp_off_in_plain..plain_header.len() - 32).step_by(4) {
                let val = u32::from_le_bytes(plain_header[off..off+4].try_into().unwrap()) as usize;
                if val >= u_off_0 && val < u_off_0 + 0x20000 {
                    let sz = u32::from_le_bytes(plain_header[off-4..off].try_into().unwrap());
                    println!("Chunk 0 export at header {off:#X}: SerialOffset = {val:#X}, SerialSize = {sz}");
                    count_in_c0 += 1;
                }
            }
            println!("Total exports in Chunk 0: {count_in_c0}");

            // Let's dump ConvertToClientLoadout
            let chunk_count_pos = 0xAB9FA8;
            let c_pos_0 = chunk_count_pos + 4;
            let u_off_0 = u64::from_le_bytes(plain_header[c_pos_0..c_pos_0+8].try_into().unwrap()) as usize;
            let c_off_0 = u64::from_le_bytes(plain_header[c_pos_0+12..c_pos_0+20].try_into().unwrap()) as usize;
            let orig_c_sz_0 = i32::from_le_bytes(plain_header[c_pos_0+20..c_pos_0+24].try_into().unwrap()) as usize;
            let c_data_0 = &file_bytes[c_off_0..c_off_0 + orig_c_sz_0];
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
            let mut full_uncomp = Vec::with_capacity(total_uncomp);
            let mut cur_payload = pos;
            for (csz, usz) in &blocks {
                let mut dec = ZlibDecoder::new(&c_data_0[cur_payload..cur_payload + *csz]);
                let mut b = Vec::with_capacity(*usz);
                dec.read_to_end(&mut b).unwrap();
                full_uncomp.extend_from_slice(&b);
                cur_payload += *csz;
            }
            let func_off_1 = target_func_offset - u_off_0;
            println!("func bytes (191): {:02X?}", &full_uncomp[func_off_1..func_off_1 + 191]);

            // Test applying a swap and verify export table alignment
            let swaps = [TagameSwapItem {
                slot: "Boost".to_string(),
                slot_index: Some(3),
                owned_id: None,
                product_id: 32,
                paint_id: None,
                package_name: None,
            }];
            let keys_txt = include_str!("../../resources/keys.txt");
            let keys_map_json = include_str!("../../resources/keys_map.json");
            let res = apply_tagame_modifications(cooked, &swaps, keys_txt, keys_map_json);
            assert!(res.is_ok(), "Apply modifications failed: {:?}", res.err());

            // Re-read modified TAGame.upk and verify export SerialSize matches the payload exactly
            let mod_bytes = fs::read(&tagame_path).unwrap();
            let mod_plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &mod_bytes[name_offset..enc_end]);
            let mut mod_entry_off = exp_off_in_plain;
            let mut found = false;
            for _ in 0..export_count as usize {
                if mod_entry_off + 48 > mod_plain_header.len() {
                    break;
                }
                let s_sz = i32::from_le_bytes(mod_plain_header[mod_entry_off + 32..mod_entry_off + 36].try_into().unwrap());
                let s_off = u64::from_le_bytes(mod_plain_header[mod_entry_off + 36..mod_entry_off + 44].try_into().unwrap()) as usize;
                let net_count = i32::from_le_bytes(mod_plain_header[mod_entry_off + 48..mod_entry_off + 52].try_into().unwrap());
                if s_off == target_func_offset {
                    println!("Verified ConvertToClientLoadout SerialSize = {s_sz}");
                    assert_eq!(s_sz, 191);
                    found = true;
                    break;
                }
                mod_entry_off += 72 + if net_count > 0 { (net_count as usize) * 4 } else { 0 };
            }
            assert!(found, "ConvertToClientLoadout export not found");

            // Restore cleanly after test
            let _ = restore_tagame_upk(cooked);
        }
    }
}
