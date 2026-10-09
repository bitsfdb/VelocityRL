/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use crate::upk::crypto;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TAGAME_BACKUP_NAME: &str = "TAGame.upk.bak";
pub const TAGAME_KEY: [u8; 32] = [
    0xc7, 0xdf, 0x6b, 0x13, 0x25, 0x2a, 0xcc, 0x71,
    0x47, 0xbb, 0x51, 0xc9, 0x8a, 0xd7, 0xe3, 0x4b,
    0x7f, 0xe5, 0x00, 0xb7, 0x7f, 0xa5, 0xfa, 0xb2,
    0x93, 0xe2, 0xf2, 0x4e, 0x6b, 0x17, 0xe7, 0x79,
];

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
    pub const EX_RETURN_VALUE: u8 = 0x3A;
    pub const EX_END_OF_SCRIPT: u8 = 0x4C;
    pub const EX_NOTHING: u8 = 0x0B;
    pub const EX_JUMP_IF_NOT: u8 = 0x07;
    pub const EX_EQUAL_EQUAL_INT_INT: u8 = 0x9A;
    pub const EX_END_FUNCTION_PARMS: u8 = 0x16;
    pub const EX_TRUE_CONST: u8 = 0x27;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagameSwapItem {
    pub slot: String,
    #[serde(default)]
    pub slot_index: Option<i32>,
    #[serde(default)]
    pub owned_id: Option<i32>,
    pub product_id: i32,
    #[serde(default)]
    pub paint_id: Option<i32>,
    #[serde(default)]
    pub custom_paint_hex: Option<String>,
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

    if let Some(hit) = check(game_dir) {
        return Ok(hit);
    }

    let mut curr = game_dir;
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

pub fn emit_convert_to_client_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    // 1. NewLoadout.Products = FromData.Products;
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
    mem_sz += 57;

    for rule in slot_overrides {
        let cond_disk_len = if rule.slot_idx == 0 { 56 } else { 58 };
        let uncond_disk_len = if rule.slot_idx == 0 { 26 } else { 27 };

        if let Some(owned_id) = rule.owned_id {
            if bc.len() + cond_disk_len + 12 > max_disk_size {
                break;
            }
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            let index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };

            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
            }
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let cond_mem = 1 + 3 + index_mem + 28 + 5 + 1;
            let body_mem = 1 + 3 + index_mem + 28 + 5;
            let total_rule_mem = 3 + cond_mem + body_mem;
            let jump_target = bc.len() as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
            mem_sz += total_rule_mem;
        } else {
            if bc.len() + uncond_disk_len + 12 > max_disk_size {
                break;
            }
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                mem_sz += 38;
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                mem_sz += 39;
            }
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
        }
    }

    // 3. return NewLoadout;
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&75i32.to_le_bytes());
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 11;

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;
    Ok((bc, mem_sz))
}

pub fn emit_car_set_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    // 1. Slot assignments on Data (local #16586)
    // De-duplicate by (slot_idx, owned_id) so latest rule for each combination applies.
    // This allows multiple different swaps on the SAME slot (e.g. 5 different wheels, multiple boosts)
    // across different presets while preserving preset conditionals!
    let mut seen = std::collections::HashSet::new();
    let mut unique_rules = Vec::new();
    for r in slot_overrides.iter().rev() {
        if seen.insert((r.slot_idx, r.owned_id)) {
            unique_rules.push(r.clone());
        }
    }
    unique_rules.reverse();

    for rule in &unique_rules {
        if let Some(owned_id) = rule.owned_id {
            // Conditional rule:
            // if (Data.Products[slot_idx] == owned_id) Data.Products[slot_idx] = target_id;
            let rule_disk_len = if rule.slot_idx == 0 { 56 } else { 58 };
            if bc.len() + rule_disk_len + 97 > max_disk_size {
                break;
            }

            // EX_JUMP_IF_NOT
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            // EX_EQUAL_EQUAL_INT_INT (==)
            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);

            // Left side: Data.Products[slot_idx]
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes()); // Products (#1871)
            bc.extend_from_slice(&1872i32.to_le_bytes()); // ClientLoadoutData (#1872)
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&16586i32.to_le_bytes()); // Data (#16586)
            let index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };

            // Right side: owned_id
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());

            // End parms for == operator
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            // Body: Data.Products[slot_idx] = target_id
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes()); // Products (#1871)
            bc.extend_from_slice(&1872i32.to_le_bytes()); // ClientLoadoutData (#1872)
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&16586i32.to_le_bytes()); // Data (#16586)
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
            }
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let jump_target = bc.len() as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());

            // Memory size for conditional:
            // JUMP_IF_NOT (3) + cond_mem (38 + index_mem) + body_mem (37 + index_mem) = 78 + 2 * index_mem
            let total_rule_mem = 3 + (38 + index_mem) + (37 + index_mem);
            mem_sz += total_rule_mem;
        } else {
            // Unconditional rule:
            // Data.Products[slot_idx] = target_id;
            let rule_disk_len = if rule.slot_idx == 0 { 26 } else { 27 };
            if bc.len() + rule_disk_len + 97 > max_disk_size {
                break;
            }
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes()); // Products (#1871)
            bc.extend_from_slice(&1872i32.to_le_bytes()); // ClientLoadoutData (#1872)
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&16586i32.to_le_bytes()); // Data (#16586)
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                mem_sz += 38;
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                mem_sz += 39;
            }
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
        }
    }

    // 2. Exact vanilla execution body:
    // bLoadoutSet = true; Loadout.EventAssetLoaded = ...; Loadout.EventAllAssetsLoaded = ...; ProductLoader.LoadClientLoadout(Data); return;
    let vanilla_body: [u8; 97] = [
        0x14, 0x2D, 0x01, 0x8D, 0x40, 0x00, 0x00, 0x27,
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF8, 0x3E, 0x00, 0x00, 0x00, 0x01, 0xF8, 0x3E, 0x00, 0x00, 0x49, 0x8B, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF7, 0x3E, 0x00, 0x00, 0x00, 0x01, 0xF7, 0x3E, 0x00, 0x00, 0x49, 0x7D, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x1C, 0x10, 0x3F, 0x00, 0x00, 0x46, 0xCA, 0x40, 0x00, 0x00, 0x16,
        0x04, 0x0B, 0x4C
    ];
    bc.extend_from_slice(&vanilla_body);
    // Exact vanilla execution body UScript memory size:
    // Vanilla function has MemSize=242, DiskSize=186 (97 bytecode bytes + 89 NOPs).
    // Each EX_NOTHING NOP is 1 byte in memory and 1 byte on disk (89 bytes).
    // Therefore, the 97-byte execution body consumes exactly 242 - 89 = 153 UScript memory bytes.
    mem_sz += 153;

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

#[allow(dead_code)]
pub fn emit_get_asset_by_id_bytecode(
    rules: &[SlotSwapRule],
    orig_script: &[u8],
    orig_mem_sz: u32,
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    for rule in rules {
        if let Some(owned_id) = rule.owned_id {
            if bc.len() + 26 + orig_script.len() > max_disk_size {
                break;
            }
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&16151i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&16151i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let cond_mem = 1 + 9 + 5 + 1;
            let body_mem = 1 + 9 + 5;
            let total_rule_mem = 3 + cond_mem + body_mem;
            let jump_target = bc.len() as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
            mem_sz += total_rule_mem;
        } else {
            if bc.len() + 11 + orig_script.len() > max_disk_size {
                break;
            }
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&16151i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
            mem_sz += 1 + 9 + 5;
        }
    }

    let remaining = max_disk_size.saturating_sub(bc.len());
    let copy_len = orig_script.len().min(remaining);
    bc.extend_from_slice(&orig_script[..copy_len]);
    mem_sz += orig_mem_sz;

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

pub fn hex_to_linear_rgba(hex: &str) -> [u8; 16] {
    let clean = hex.trim().trim_start_matches('#');
    let (r_srgb, g_srgb, b_srgb) = if clean.len() >= 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(255) as f32 / 255.0;
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(255) as f32 / 255.0;
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(255) as f32 / 255.0;
        (r, g, b)
    } else if clean.len() == 3 {
        let r = u8::from_str_radix(&clean[0..1].repeat(2), 16).unwrap_or(255) as f32 / 255.0;
        let g = u8::from_str_radix(&clean[1..2].repeat(2), 16).unwrap_or(255) as f32 / 255.0;
        let b = u8::from_str_radix(&clean[2..3].repeat(2), 16).unwrap_or(255) as f32 / 255.0;
        (r, g, b)
    } else {
        (1.0, 1.0, 1.0)
    };

    let r_linear = r_srgb.powf(2.2);
    let g_linear = g_srgb.powf(2.2);
    let b_linear = b_srgb.powf(2.2);
    let a_linear = 1.0f32;

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&r_linear.to_le_bytes());
    out[4..8].copy_from_slice(&g_linear.to_le_bytes());
    out[8..12].copy_from_slice(&b_linear.to_le_bytes());
    out[12..16].copy_from_slice(&a_linear.to_le_bytes());
    out
}

pub fn get_paint_rgba(paint_id: i32) -> [u8; 16] {
    let (r, g, b, a): (f32, f32, f32, f32) = match paint_id {
        1 => (0.831, 0.129, 0.169, 1.0),   // Crimson
        2 => (0.655, 0.902, 0.000, 1.0),   // Lime
        3 => (0.005, 0.005, 0.005, 1.0),   // Black
        4 => (0.000, 0.706, 1.000, 1.0),   // Sky Blue
        5 => (0.122, 0.271, 0.988, 1.0),   // Cobalt
        6 => (0.420, 0.196, 0.051, 1.0),   // Burnt Sienna
        7 => (0.180, 0.545, 0.341, 1.0),   // Forest Green
        8 => (0.502, 0.000, 0.502, 1.0),   // Purple
        9 => (1.000, 0.431, 0.706, 1.0),   // Pink
        10 => (1.000, 0.455, 0.000, 1.0),  // Orange
        11 => (0.541, 0.541, 0.541, 1.0),  // Grey
        12 => (1.500, 1.500, 1.500, 1.0),  // Titanium White
        13 => (1.000, 0.945, 0.090, 1.0),  // Saffron
        14 => (1.000, 0.843, 0.000, 1.0),  // Gold
        15 => (0.718, 0.431, 0.475, 1.0),  // Rose Gold
        16 => (1.200, 1.150, 0.900, 1.0),  // White Gold
        17 => (0.002, 0.002, 0.002, 1.0),  // Onyx
        18 => (0.850, 0.900, 0.950, 1.0),  // Platinum
        19 => (0.220, 0.741, 0.973, 2.5),  // Sky Blue Glow
        20 => (0.231, 0.510, 0.965, 2.5),  // Cobalt Glow
        21 => (0.635, 0.424, 0.271, 2.5),  // Burnt Sienna Glow
        22 => (0.290, 0.871, 0.502, 2.5),  // Forest Green Glow
        23 => (0.745, 0.949, 0.392, 2.5),  // Lime Glow
        24 => (0.984, 0.573, 0.235, 2.5),  // Orange Glow
        25 => (0.957, 0.447, 0.714, 2.5),  // Pink Glow
        26 => (0.753, 0.518, 0.988, 2.5),  // Purple Glow
        27 => (0.973, 0.443, 0.443, 2.5),  // Crimson Glow
        28 => (2.500, 2.500, 2.500, 2.5),  // Titanium White Glow
        29 => (0.992, 0.878, 0.278, 2.5),  // Saffron Glow
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
    custom_hex: Option<&str>,
    _keys_map_json: &str,
) -> Result<(), TagameSwapError> {
    let (pkg_path, actual_file_name) = match crate::upk::swapper::resolve_package_path(cooked_dir, package_name) {
        Some(res) => res,
        None => return Ok(()),
    };
    let backup_path = cooked_dir.join(format!("{actual_file_name}.bak"));

    if !backup_path.is_file() {
        let _ = fs::copy(&pkg_path, &backup_path);
    }

    let src_path = if backup_path.is_file() { &backup_path } else { &pkg_path };
    let mut file_bytes = fs::read(src_path)?;

    if file_bytes.len() < 32 {
        return Err(TagameSwapError::Msg("UPK file too small".into()));
    }

    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;

    let c_off = total_header_size;
    if c_off + 16 > file_bytes.len() {
        return Err(TagameSwapError::Msg("Chunk 0 offset invalid".into()));
    }

    let c0_magic = u32::from_le_bytes(file_bytes[c_off..c_off + 4].try_into().unwrap());
    if c0_magic != 0x9E2A83C1 {
        return Err(TagameSwapError::Msg("Invalid Chunk 0 magic".into()));
    }
    let comp_sz = u32::from_le_bytes(file_bytes[c_off + 8..c_off + 12].try_into().unwrap()) as usize;
    let uncomp_sz = u32::from_le_bytes(file_bytes[c_off + 12..c_off + 16].try_into().unwrap()) as usize;

    let mut header_pos = c_off + 16;
    let mut sum_uncomp = 0usize;
    while sum_uncomp < uncomp_sz && header_pos + 8 <= file_bytes.len() {
        let bu = i32::from_le_bytes(file_bytes[header_pos + 4..header_pos + 8].try_into().unwrap()) as usize;
        sum_uncomp += bu;
        header_pos += 8;
    }
    let c0_total_disk = (header_pos - c_off) + comp_sz;
    if c_off + c0_total_disk > file_bytes.len() {
        return Err(TagameSwapError::Msg("Chunk 0 disk size out of bounds".into()));
    }

    let mut decomp = match crate::upk::compression::decompress_chunk(&file_bytes[c_off..c_off + c0_total_disk]) {
        Ok(d) => d,
        Err(e) => return Err(TagameSwapError::Msg(format!("Decompress chunk failed: {e}"))),
    };

    let target_rgba = if let Some(hex) = custom_hex.filter(|h| !h.trim().is_empty()) {
        hex_to_linear_rgba(hex)
    } else {
        get_paint_rgba(paint_id)
    };

    // Replace all material instance vector parameter patterns across decompressed chunk
    let patterns = [
        // 0.0663 f32 (CustomColor stock on Fennec/Dominus/Octane)
        [0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x00, 0x00, 0x80, 0x3f],
        // 0.12 f32 (TrimColor stock)
        [0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x00, 0x00, 0x80, 0x3f],
        // 0.05 f32
        [0xcd, 0xcc, 0x4c, 0x3d, 0xcd, 0xcc, 0x4c, 0x3d, 0xcd, 0xcc, 0x4c, 0x3d, 0x00, 0x00, 0x80, 0x3f],
    ];

    for pat in &patterns {
        let mut search_from = 0;
        while let Some(rel) = decomp[search_from..].windows(16).position(|w| w == pat) {
            let pos = search_from + rel;
            decomp[pos..pos + 16].copy_from_slice(&target_rgba);
            search_from = pos + 16;
        }
    }


    // Recompress Chunk 0
    let mut recomp = crate::upk::compression::compress_chunk(&decomp)
        .map_err(|e| TagameSwapError::Msg(format!("Compress chunk failed: {e}")))?;

    if recomp.len() <= c0_total_disk {
        recomp.resize(c0_total_disk, 0);
        file_bytes[c_off..c_off + c0_total_disk].copy_from_slice(&recomp);
    } else {
        return Err(TagameSwapError::Msg(format!(
            "Recompressed Chunk 0 ({} bytes) exceeds allocation ({} bytes)",
            recomp.len(),
            c0_total_disk
        )));
    }

    fs::write(&pkg_path, &file_bytes)?;
    Ok(())
}

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

    if !backup_path.is_file() {
        fs::copy(&tagame_path, &backup_path).map_err(|e| {
            TagameSwapError::Msg(format!("Failed to create backup {}: {e}", backup_path.display()))
        })?;
    }

    let mut file_bytes = fs::read(&backup_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read {}: {e}", backup_path.display()))
    })?;

    let pal_st = crate::upk::palette::read_palette_status(cooked_dir, None);
    if pal_st.applied {
        if let Ok(_) = crate::upk::palette::apply_rich_palette_to_file(cooked_dir, _keys_txt, keys_map_json) {
            if let Ok(pal_bytes) = fs::read(&tagame_path) {
                file_bytes = pal_bytes;
            }
        }
    }

    if !swaps.is_empty() {
        if file_bytes.len() < 32 {
            return Err(TagameSwapError::Msg("TAGame.upk too small".into()));
        }

        let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
        let mut p = 12;
        let flen = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
        p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
        p += 4;
        let name_count = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
        p += 4;
        let name_offset = u32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap()) as usize;

        let enc_size = (total_header_size - name_offset + 15) & !15;
        let enc_end = name_offset + enc_size;
        if enc_end > file_bytes.len() {
            return Err(TagameSwapError::Msg("Encrypted header bounds invalid".into()));
        }

        let mut plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
        let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count)
            .map_err(|e| TagameSwapError::Msg(e.to_string()))?;

        let p_sum = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize }) + 4 + 8;
        let export_count = i32::from_le_bytes(file_bytes[p_sum..p_sum + 4].try_into().unwrap()) as usize;
        let export_offset = i32::from_le_bytes(file_bytes[p_sum + 4..p_sum + 8].try_into().unwrap()) as usize;
        let depends_offset = i32::from_le_bytes(file_bytes[p_sum + 16..p_sum + 20].try_into().unwrap()) as usize;

        let export_rel = export_offset - name_offset;
        let depends_rel = depends_offset - name_offset;

        #[allow(dead_code)]
        struct ExportItem {
            idx: usize,
            pos: usize,
            name: String,
            outer_idx: i32,
            outer_name: String,
            serial_size: i32,
            serial_offset: i64,
        }

        let mut exports: Vec<ExportItem> = Vec::new();
        let mut pos = export_rel;

        while pos + 72 <= depends_rel && pos + 72 <= plain_header.len() && exports.len() < export_count {
            let i32_at = |a: usize| i32::from_le_bytes(plain_header[pos + a..pos + a + 4].try_into().unwrap());
            let outer_idx = i32_at(8);
            let name_idx = i32_at(12);
            let serial_size = i32_at(32);
            let serial_offset = i64::from_le_bytes(plain_header[pos + 36..pos + 44].try_into().unwrap());
            let noc = i32_at(48);

            let nm = if name_idx >= 0 && (name_idx as usize) < names.len() {
                names[name_idx as usize].clone()
            } else {
                String::new()
            };

            let cur_idx = exports.len() + 1;
            exports.push(ExportItem {
                idx: cur_idx,
                pos,
                name: nm,
                outer_idx,
                outer_name: String::new(),
                serial_size,
                serial_offset,
            });

            pos += 72 + (noc.max(0) as usize) * 4;
        }

        for i in 0..exports.len() {
            let out_idx = exports[i].outer_idx;
            if out_idx > 0 && (out_idx as usize) <= exports.len() {
                exports[i].outer_name = exports[out_idx as usize - 1].name.clone();
            }
        }

        #[allow(dead_code)]
        struct ChunkItem {
            pos: usize,
            uncomp_offset: i64,
            uncomp_size: i32,
            comp_offset: i64,
            comp_size: i32,
        }

        let mut c_pos = depends_rel;
        if c_pos + 4 > plain_header.len() {
            return Err(TagameSwapError::Msg("Chunk table offset out of bounds".into()));
        }
        let chunk_count = i32::from_le_bytes(plain_header[c_pos..c_pos + 4].try_into().unwrap()) as usize;
        c_pos += 4;

        let mut chunks = Vec::with_capacity(chunk_count);
        for _ in 0..chunk_count {
            if c_pos + 36 > plain_header.len() {
                break;
            }
            let uncomp_offset = i64::from_le_bytes(plain_header[c_pos..c_pos + 8].try_into().unwrap());
            let uncomp_size = i32::from_le_bytes(plain_header[c_pos + 8..c_pos + 12].try_into().unwrap());
            let comp_offset = i64::from_le_bytes(plain_header[c_pos + 12..c_pos + 20].try_into().unwrap());
            let comp_size = i32::from_le_bytes(plain_header[c_pos + 20..c_pos + 24].try_into().unwrap());
            chunks.push(ChunkItem {
                pos: c_pos,
                uncomp_offset,
                uncomp_size,
                comp_offset,
                comp_size,
            });
            c_pos += 36;
        }

        let mut slot_overrides = Vec::new();

        for s in swaps {
            let pid = if s.product_id > 0 { s.product_id } else { 4284 };
            let norm = s.slot.to_lowercase().replace([' ', '_', '-'], "");
            let slot_idx: u8 = if norm.contains("body") || s.slot_index == Some(0) {
                0
            } else if norm.contains("decal") || norm.contains("skin") || s.slot_index == Some(1) {
                1
            } else if norm.contains("wheel") || s.slot_index == Some(2) {
                2
            } else if norm.contains("boost") || s.slot_index == Some(3) {
                3
            } else if norm.contains("antenna") || s.slot_index == Some(4) {
                4
            } else if norm.contains("topper") || norm.contains("hat") || s.slot_index == Some(5) {
                5
            } else if norm.contains("paintfinish") || norm.contains("finish") || s.slot_index == Some(6) {
                6
            } else if norm.contains("accent") || s.slot_index == Some(7) {
                7
            } else if norm.contains("audio") || norm.contains("engine") || s.slot_index == Some(8) {
                8
            } else if norm.contains("trail") || s.slot_index == Some(9) || s.slot_index == Some(13) {
                9
            } else if norm.contains("goal") || norm.contains("explosion") || s.slot_index == Some(10) || s.slot_index == Some(14) {
                10
            } else if norm.contains("banner") || s.slot_index == Some(11) || s.slot_index == Some(15) {
                11
            } else if norm.contains("anthem") || norm.contains("music") || s.slot_index == Some(12) || s.slot_index == Some(18) {
                12
            } else if norm.contains("border") || s.slot_index == Some(13) || s.slot_index == Some(20) {
                13
            } else if let Some(idx) = s.slot_index {
                idx as u8
            } else {
                crate::presets::slot_index_from_str(&s.slot) as u8
            };

            slot_overrides.push(SlotSwapRule {
                slot_idx,
                owned_id: s.owned_id,
                target_id: pid,
            });

            let is_ge = slot_idx == 10 || norm.contains("goal") || norm.contains("explosion") || s.slot_index == Some(14);
            if is_ge {
                let target_ge = if s.product_id > 0 { s.product_id } else { 2044 };

                // Goal Explosions map to Slot 10 (garage/legacy) and Slot 15 (in-match/replication)
                slot_overrides.push(SlotSwapRule {
                    slot_idx: 15,
                    owned_id: s.owned_id,
                    target_id: target_ge,
                });

                // If owned is 0, 1903, or None (default), cover both 0 and 1903 on both slot 10 and slot 15
                if s.owned_id == Some(1903) || s.owned_id == Some(0) || s.owned_id.is_none() {
                    slot_overrides.push(SlotSwapRule {
                        slot_idx: 10,
                        owned_id: Some(0),
                        target_id: target_ge,
                    });
                    slot_overrides.push(SlotSwapRule {
                        slot_idx: 10,
                        owned_id: Some(1903),
                        target_id: target_ge,
                    });
                    slot_overrides.push(SlotSwapRule {
                        slot_idx: 15,
                        owned_id: Some(0),
                        target_id: target_ge,
                    });
                    slot_overrides.push(SlotSwapRule {
                        slot_idx: 15,
                        owned_id: Some(1903),
                        target_id: target_ge,
                    });
                }
            }
        }

        // 1. Patch Chunk 0: Car_TA::SetLoadout (#16587) with dynamic allocation expansion for up to 50+ swaps
        if !chunks.is_empty() {
            let c0_uncomp_offset = chunks[0].uncomp_offset;
            let c0_comp_offset = chunks[0].comp_offset;
            let c0_comp_size = chunks[0].comp_size;
            let c0_start = c0_comp_offset as usize;
            let c0_end = c0_start + c0_comp_size as usize;
            if c0_end <= file_bytes.len() {
                if let Ok(mut decomp0) = crate::upk::compression::decompress_chunk(&file_bytes[c0_start..c0_end]) {
                    // Car_TA::SetLoadout handles swapping directly on ClientLoadoutData when the vehicle spawns.
                    // Leaving ConvertToClientLoadout vanilla preserves all non-product fields (paint, colors, etc.) without corruption.

                    if let Some(exp) = exports.iter().find(|e| e.name == "SetLoadout" && e.outer_name == "Car_TA") {
                        let func_off = (exp.serial_offset - c0_uncomp_offset) as usize;
                        if func_off + 48 <= decomp0.len() {
                            let orig_disk_sz = u32::from_le_bytes(decomp0[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                            if let Ok((payload, mem_sz)) = emit_car_set_loadout_bytecode(&slot_overrides, orig_disk_sz) {
                                decomp0[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                decomp0[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                            }
                        }
                    }

                    if let Ok(mut recomp0) = crate::upk::compression::compress_chunk(&decomp0) {
                        let orig_c0_sz = c0_comp_size as usize;
                        if recomp0.len() <= orig_c0_sz {
                            recomp0.resize(orig_c0_sz, 0);
                            file_bytes[c0_start..c0_start + orig_c0_sz].copy_from_slice(&recomp0);
                        } else {
                            return Err(TagameSwapError::Msg(format!(
                                "Recompressed Chunk 0 ({} bytes) exceeds allocation ({} bytes)",
                                recomp0.len(),
                                orig_c0_sz
                            )));
                        }
                    }
                }
            }
        }

        // Helper to safely patch any simple function bytecode with padding and updated mem_sz
        let patch_func = |decomp: &mut [u8], exp_name: &str, outer_name: Option<&str>, chunk_u_off: i64, bc: &[u8]| -> bool {
            if let Some(exp) = exports.iter().find(|e| e.name == exp_name && outer_name.map_or(true, |o| e.outer_name == o)) {
                let func_off = (exp.serial_offset - chunk_u_off) as usize;
                if func_off + 48 <= decomp.len() {
                    let orig_disk_sz = u32::from_le_bytes(decomp[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                    if bc.len() <= orig_disk_sz && func_off + 48 + orig_disk_sz <= decomp.len() {
                        let pad = orig_disk_sz - bc.len();
                        let mem_sz = (bc.len() + pad) as u32;
                        decomp[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                        decomp[func_off + 48..func_off + 48 + bc.len()].copy_from_slice(bc);
                        for b in &mut decomp[func_off + 48 + bc.len()..func_off + 48 + orig_disk_sz] {
                            *b = opcodes::EX_NOTHING;
                        }
                        return true;
                    }
                }
            }
            false
        };





        // Ensure TAGame.upk references the exact Engine package GUID from Engine.upk (prevents version mismatch error)
        let engine_path = cooked_dir.join("Engine.upk");
        if let Some(engine_guid) = crate::upk::swapper::read_package_guid(&engine_path) {
            for exp in &exports {
                if exp.name.eq_ignore_ascii_case("Engine") {
                    let guid_pos = exp.pos + 56;
                    if guid_pos + 16 <= plain_header.len() {
                        plain_header[guid_pos..guid_pos + 16].copy_from_slice(&engine_guid);
                    }
                    break;
                }
            }
        }

        let re_enc = crypto::encrypt_ecb(&TAGAME_KEY, &plain_header);
        file_bytes[name_offset..enc_end].copy_from_slice(&re_enc);

        fs::write(&tagame_path, &file_bytes)?;
    } else {
        if !pal_st.applied && backup_path.is_file() {
            let _ = fs::copy(&backup_path, &tagame_path);
        }
    }

    // Apply body trim paint overrides (e.g. Fennec Black / custom trim hex)
    for s in swaps {
        let is_body = s.slot.to_lowercase().contains("body")
            || s.package_name.as_deref().map_or(false, |p| p.to_lowercase().starts_with("body_"));
        if !is_body {
            continue;
        }

        let pkg = s.package_name.as_deref().unwrap_or("body_grain_SF");
        let custom_hex_str = s.custom_paint_hex.as_deref();
        let has_custom = custom_hex_str.map(|h| !h.trim().is_empty()).unwrap_or(false);
        let pid = s.paint_id.unwrap_or(0);

        if has_custom || pid > 0 {
            let _ = apply_body_paint_modification(cooked_dir, "body_grain_SF", pid, custom_hex_str, keys_map_json);
            if pkg != "body_grain_SF" {
                let _ = apply_body_paint_modification(cooked_dir, pkg, pid, custom_hex_str, keys_map_json);
            }
        } else {
            for restore_name in [pkg, "body_grain_SF"] {
                if let Some((pkg_path, actual_file_name)) = crate::upk::swapper::resolve_package_path(cooked_dir, restore_name) {
                    let bak_path = cooked_dir.join(format!("{actual_file_name}.bak"));
                    if bak_path.is_file() {
                        let _ = fs::copy(&bak_path, &pkg_path);
                    }
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
        message: "Loadout modifications applied successfully.".to_string(),
    })
}

pub fn restore_tagame_upk(cooked_dir: &Path) -> Result<TagameSwapperStatus, TagameSwapError> {
    let tagame_path = cooked_dir.join("TAGame.upk");
    let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);

    if !backup_path.is_file() {
        return Ok(TagameSwapperStatus {
            applied: false,
            backup_present: false,
            tagame_path: tagame_path.to_string_lossy().into_owned(),
            active_swaps: Vec::new(),
            message: "No backup found to restore.".to_string(),
        });
    }

    fs::copy(&backup_path, &tagame_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to restore TAGame.upk from backup: {e}"))
    })?;

    if let Ok(entries) = fs::read_dir(cooked_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let lower = file_name.to_lowercase();
                if lower.ends_with(".upk.bak") {
                    let live_name = file_name.trim_end_matches(".bak");
                    let live_path = cooked_dir.join(live_name);
                    let _ = fs::copy(&path, &live_path);
                }
            }
        }
    }

    Ok(TagameSwapperStatus {
        applied: false,
        backup_present: true,
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: Vec::new(),
        message: "TAGame.upk and backups restored successfully.".to_string(),
    })
}

/// Download clean, official TAGame.upk from VelocityRL asset endpoint with progress reporting.
pub async fn download_official_tagame_upk<F>(target_path: &Path, mut progress_cb: F) -> Result<(), String>
where
    F: FnMut(usize, usize) + Send + 'static,
{
    use futures_util::StreamExt;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let build_secret = option_env!("VRL_BUILD_SECRET")
        .unwrap_or("18667c8a510a5a0eb3ea0124d23f372b7387b6383a328ca4136e30f1f633997a");

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut mac = HmacSha256::new_from_slice(build_secret.as_bytes())
        .map_err(|e| format!("HMAC init failed: {e}"))?;
    mac.update(format!("{now_ts}:TAGame.upk").as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());

    let url = format!(
        "https://api.velocityrl.tech/v2/rl/assets/tagame.upk?secret={sig}&t={now_ts}"
    );

    crate::applog::event(&format!("tagame: downloading official TAGame.upk from {url}"));

    let client = reqwest::Client::builder()
        .user_agent(crate::app_user_agent())
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Asset endpoint returned HTTP {}", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(78_252_961) as usize;
    let mut downloaded = 0usize;

    let mut stream = resp.bytes_stream();
    let mut bytes = Vec::with_capacity(total_size);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Network error while downloading: {e}"))?;
        downloaded += chunk.len();
        bytes.extend_from_slice(&chunk);
        progress_cb(downloaded, total_size);
    }

    if bytes.len() < 10_000_000 {
        return Err(format!("Downloaded TAGame.upk file too small ({} bytes)", bytes.len()));
    }

    if let Some(parent) = target_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(target_path, &bytes).map_err(|e| format!("Failed to write TAGame.upk: {e}"))?;

    crate::applog::event(&format!(
        "tagame: official TAGame.upk downloaded successfully ({} bytes)",
        bytes.len()
    ));

    Ok(())
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
        assert_eq!(bc[0], opcodes::EX_LET);
        assert_eq!(bc[33], opcodes::EX_JUMP_IF_NOT);
        assert_eq!(bc[36], opcodes::EX_EQUAL_EQUAL_INT_INT);
        assert_eq!(mem_sz, 176);
    }

    #[test]
    fn test_emit_car_set_loadout_bytecode() {
        let rules = [
            SlotSwapRule { slot_idx: 0, owned_id: None, target_id: 4284 },
            SlotSwapRule { slot_idx: 2, owned_id: None, target_id: 30 },
            SlotSwapRule { slot_idx: 3, owned_id: None, target_id: 32 },
        ];
        let (bc, mem_sz) = emit_car_set_loadout_bytecode(&rules, 186).unwrap();
        assert_eq!(bc.len(), 186);
        assert_eq!(bc[0], opcodes::EX_LET);
        assert!(mem_sz >= 186);
    }

    #[test]
    fn test_emit_car_set_loadout_bytecode_conditional() {
        let rules = [
            SlotSwapRule { slot_idx: 2, owned_id: Some(376), target_id: 30 },
            SlotSwapRule { slot_idx: 2, owned_id: Some(400), target_id: 50 },
            SlotSwapRule { slot_idx: 3, owned_id: Some(29), target_id: 12968 },
        ];
        let (bc, mem_sz) = emit_car_set_loadout_bytecode(&rules, 300).unwrap();
        assert_eq!(bc.len(), 300);
        assert_eq!(bc[0], opcodes::EX_JUMP_IF_NOT);
        assert_eq!(bc[3], opcodes::EX_EQUAL_EQUAL_INT_INT);
        assert_eq!(bc[4], opcodes::EX_DYN_ARRAY_OP);
        assert!(mem_sz >= 300);
    }

    #[test]
    fn test_apply_tagame_modifications_real_upk() {
        let cooked_bak = Path::new(r"E:\games\rocketleague\TAGame\CookedPCConsole");
        if !cooked_bak.join("TAGame.upk.bak").is_file() {
            return;
        }

        let temp_dir = std::env::temp_dir().join("velocityrl_test_tagame");
        let _ = fs::create_dir_all(&temp_dir);
        let tagame_test_bak = temp_dir.join("TAGame.upk.bak");
        let tagame_test = temp_dir.join("TAGame.upk");

        let _ = fs::copy(cooked_bak.join("TAGame.upk.bak"), &tagame_test_bak);
        let _ = fs::copy(cooked_bak.join("TAGame.upk.bak"), &tagame_test);

        let swaps = vec![
            TagameSwapItem {
                slot: "Body".to_string(),
                slot_index: Some(0),
                owned_id: Some(23),
                product_id: 4284, // Fennec
                paint_id: None,
                custom_paint_hex: None,
                package_name: Some("body_grain_SF".to_string()),
            },
            TagameSwapItem {
                slot: "Rocket Boost".to_string(),
                slot_index: Some(3),
                owned_id: Some(29),
                product_id: 12968, // Alpha Boost
                paint_id: None,
                custom_paint_hex: None,
                package_name: None,
            },
            TagameSwapItem {
                slot: "Wheels".to_string(),
                slot_index: Some(2),
                owned_id: Some(376),
                product_id: 30, // Goldstone
                paint_id: None,
                custom_paint_hex: None,
                package_name: None,
            },
            TagameSwapItem {
                slot: "Goal Explosion".to_string(),
                slot_index: Some(10),
                owned_id: Some(1903),
                product_id: 2044, // Dueling Dragons
                paint_id: None,
                custom_paint_hex: None,
                package_name: Some("explosion_Dragon".to_string()),
            },
        ];

        let keys_txt = include_str!("../../resources/keys.txt");
        let keys_map_json = include_str!("../../resources/keys_map.json");

        let status = apply_tagame_modifications(&temp_dir, &swaps, keys_txt, keys_map_json).expect("apply_tagame_modifications must succeed");
        assert!(status.applied);
        assert_eq!(status.active_swaps.len(), 4);

        let modified_bytes = fs::read(&tagame_test).expect("read modified TAGame.upk");
        assert!(modified_bytes.len() > 1024 * 1024);
    }
}
