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
    pub const EX_EQUAL_EQUAL_INT_INT: u8 = 0x9A;
    pub const EX_END_FUNCTION_PARMS: u8 = 0x16;
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

    // 2. Overrides (Conditional slot assignments, only swapping the user's specific item)
    for rule in slot_overrides {
        let cond_owned_id = rule.owned_id.or_else(|| if rule.slot_idx == 0 { Some(23) } else { None });
        let cond_disk_len = if rule.slot_idx == 0 { 56 } else { 58 };
        let uncond_disk_len = if rule.slot_idx == 0 { 26 } else { 27 };

        let use_cond = cond_owned_id.is_some() && (bc.len() + cond_disk_len + 7 <= max_disk_size);
        let use_uncond = !use_cond && (bc.len() + uncond_disk_len + 7 <= max_disk_size);

        if !use_cond && !use_uncond {
            // Buffer capacity reached (124 disk bytes)
            break;
        }

        if use_cond {
            let owned_id = cond_owned_id.unwrap();
            // EX_JumpIfNot
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]); // placeholder for jump target

            // Native 154 (0x9A) EqualEqual_IntInt
            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            // Arg 1: NewLoadout.Products[slot]
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            let index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());

            // Arg 2: owned_id (IntConst)
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());

            // End parms
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            // Body: NewLoadout.Products[slot] = target_id;
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

            let cond_mem = 1 + 3 + index_mem + 28 + 5 + 1; // Native (1) + DynArray (3) + index + Struct (19) + Var (9) + IntConst (5) + EndParms (1)
            let body_mem = 1 + 3 + index_mem + 28 + 5;     // Let (1) + DynArray (3) + index + Struct (19) + Var (9) + IntConst (5)
            let total_rule_mem = 3 + cond_mem + body_mem;  // JumpIfNot (1) + wOffset (2) + cond + body
            let jump_target_mem = (mem_sz + total_rule_mem) as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target_mem.to_le_bytes());

            mem_sz += total_rule_mem;
        } else {
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            // Index expression
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                mem_sz += 38; // 26 disk -> 38 mem
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                mem_sz += 39; // 27 disk -> 39 mem
            }
            // Array expression
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            // Value expression
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
        }
    }

    // 3. return NewLoadout;
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&75i32.to_le_bytes());

    // 4. End of script
    bc.push(opcodes::EX_END_OF_SCRIPT);

    mem_sz += 10 + 1; // Return (10) + EOS (1) = 11 mem

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;
    Ok((bc, mem_sz))
}

/// Emits bytecode for CorrectOnlineData in LoadoutValidation_TA:
/// 1. For each swap rule:
///    if (owned_id == Some(id)) {
///        if (OutLoadout.Products[slot_idx] == id) { OutLoadout.Products[slot_idx] = target_id; }
///    } else {
///        OutLoadout.Products[slot_idx] = target_id;
///    }
/// 2. return true;
/// 3. Pad with EX_NOTHING to max_disk_size.
pub fn emit_correct_online_data_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    for rule in slot_overrides {
        if let Some(owned_id) = rule.owned_id {
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            // Condition: EqualEqual_IntInt(OutLoadout.Products[slot], owned_id)
            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            let index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes()); // Products
            bc.extend_from_slice(&1872i32.to_le_bytes()); // ClientLoadoutData
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE); // OutLoadout
            bc.extend_from_slice(&47858i32.to_le_bytes());

            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            // Body: OutLoadout.Products[slot] = target_id
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
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&47858i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let cond_mem = 1 + 3 + index_mem + 28 + 5 + 1;
            let body_mem = 1 + 3 + index_mem + 28 + 5;
            let total_rule_mem = 3 + cond_mem + body_mem;
            let jump_target_mem = (mem_sz + total_rule_mem) as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target_mem.to_le_bytes());
            mem_sz += total_rule_mem;
        } else {
            // Unconditional: OutLoadout.Products[slot] = target_id
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                mem_sz += 38;
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                mem_sz += 39;
            }
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&47858i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
        }
    }

    // Return true
    bc.push(opcodes::EX_RETURN);
    bc.push(0x27); // EX_TRUE_CONST
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 1 + 1 + 1;

    if bc.len() > max_disk_size {
        return Err(TagameSwapError::Msg(format!(
            "CorrectOnlineData bytecode size {} exceeds max disk size {}",
            bc.len(), max_disk_size
        )));
    }

    let nop_count = max_disk_size - bc.len();
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

/// Emits bytecode for Car_TA::SetLoadout (#16587):
/// 1. For each swap rule:
///    if (owned_id == Some(id)) {
///        if (Data.Products[slot_idx] == id) { Data.Products[slot_idx] = target_id; }
///    } else {
///        Data.Products[slot_idx] = target_id;
///    }
/// 2. bLoadoutSet = true;
/// 3. ProductLoader.PreLoad();
/// 4. ProductLoader.ClearLoaded();
/// 5. ProductLoader.LoadClientLoadout(Data);
/// 6. return;
/// 7. 0x0B (NOP) padding to max_disk_size.
pub fn emit_car_set_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    // 1. Swap Overrides on Data.Products (Data is local parameter #16586)
    for rule in slot_overrides {
        let cond_owned_id = rule.owned_id.or_else(|| if rule.slot_idx == 0 { Some(23) } else { None });
        if let Some(owned_id) = cond_owned_id {
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            // Condition: EqualEqual_IntInt(Data.Products[slot], owned_id)
            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            let index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes()); // Products
            bc.extend_from_slice(&1872i32.to_le_bytes()); // ClientLoadoutData
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE); // Data (#16586)
            bc.extend_from_slice(&16586i32.to_le_bytes());

            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            // Body: Data.Products[slot] = target_id
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
            bc.push(opcodes::EX_LOCAL_VARIABLE); // Data (#16586)
            bc.extend_from_slice(&16586i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let cond_mem = 1 + 3 + index_mem + 28 + 5 + 1;
            let body_mem = 1 + 3 + index_mem + 28 + 5;
            let total_rule_mem = 3 + cond_mem + body_mem;
            let jump_target_mem = (mem_sz + total_rule_mem) as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target_mem.to_le_bytes());
            mem_sz += total_rule_mem;
        } else {
            // Unconditional: Data.Products[slot] = target_id
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                mem_sz += 38;
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                mem_sz += 39;
            }
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&1871i32.to_le_bytes());
            bc.extend_from_slice(&1872i32.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&16586i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
        }
    }

    // 2. Original Car_TA::SetLoadout remaining operations:
    // bLoadoutSet = true;
    bc.extend_from_slice(&[0x14, 0x2D, 0x01, 0x8D, 0x40, 0x00, 0x00, 0x27]);
    mem_sz += 12;

    // ProductLoader.PreLoad()
    bc.extend_from_slice(&[
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF8, 0x3E, 0x00, 0x00, 0x00,
        0x01, 0xF8, 0x3E, 0x00, 0x00, 0x49, 0x8B, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    mem_sz += 43;

    // ProductLoader.ClearLoaded()
    bc.extend_from_slice(&[
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF7, 0x3E, 0x00, 0x00, 0x00,
        0x01, 0xF7, 0x3E, 0x00, 0x00, 0x49, 0x7D, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    mem_sz += 42;

    // ProductLoader.LoadClientLoadout(Data)
    bc.extend_from_slice(&[
        0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x1C, 0x10, 0x3F, 0x00, 0x00, 0x46, 0xCA, 0x40, 0x00, 0x00, 0x16,
    ]);
    mem_sz += 41;

    // Return;
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_NOTHING);
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 3;

    if bc.len() > max_disk_size {
        return Err(TagameSwapError::Msg(format!(
            "SetLoadout bytecode size {} exceeds max disk size {}",
            bc.len(), max_disk_size
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
    let source_path = if backup_path.is_file() {
        &backup_path
    } else {
        fs::copy(&tagame_path, &backup_path).map_err(|e| {
            TagameSwapError::Msg(format!("Failed to create backup {}: {e}", backup_path.display()))
        })?;
        &backup_path
    };

    // 2. Always read from the pristine backup so changes are idempotent and never corrupt
    let mut file_bytes = fs::read(source_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read {}: {e}", source_path.display()))
    })?;

    // Decrypt and parse chunks dynamically
    let (_summary, meta, mut plain_header, _, _) = crate::upk::palette::debug_decrypt(&file_bytes, _keys_txt, keys_map_json)
        .map_err(|e| TagameSwapError::Msg(format!("Failed to decrypt TAGame.upk: {e}")))?;

    let (_stride, chunks) = crate::upk::parser::parse_chunks_with_stride(&plain_header, meta.compressed_chunks_offset)
        .map_err(|e| TagameSwapError::Msg(format!("Failed to parse chunks: {e}")))?;

    if chunks.is_empty() {
        return Err(TagameSwapError::Msg("TAGame has no chunks".into()));
    }

    let c0 = &chunks[0];

    let c0_payload = &file_bytes[c0.compressed_offset as usize..(c0.compressed_offset + c0.compressed_size as i64) as usize];
    let mut decomp0 = crate::upk::compression::decompress_chunk(c0_payload)
        .map_err(|e| TagameSwapError::Msg(format!("Failed to decompress Chunk 0: {e}")))?;

    // Collect slot overrides
    let mut slot_overrides = Vec::new();
    for s in swaps {
        let slot_idx = if let Some(idx) = s.slot_index {
            idx as u8
        } else {
            match s.slot.to_lowercase().as_str() {
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
                _ => 0,
            }
        };
        let pid = if s.product_id > 0 { s.product_id } else { 4284 };
        slot_overrides.push(SlotSwapRule {
            slot_idx,
            owned_id: s.owned_id,
            target_id: pid,
        });
    }

    let export_count = _summary.export_count as usize;
    let export_offset = _summary.export_offset as usize;
    let name_offset = _summary.name_offset as usize;
    let depends_offset = _summary.depends_offset as usize;

    let export_rel = export_offset.saturating_sub(name_offset);
    let depends_rel = depends_offset.saturating_sub(name_offset);
    let mut exports = Vec::new();
    let mut pos = export_rel;
    while pos + 72 <= depends_rel && pos + 72 <= plain_header.len() && exports.len() < export_count {
        let i32_at = |a: usize| i32::from_le_bytes(plain_header[pos + a..pos + a + 4].try_into().unwrap());
        let noc = i32_at(48);
        exports.push((pos, i32_at(0), i32_at(8), i32_at(12), i32_at(32), i64::from_le_bytes(plain_header[pos + 36..pos + 44].try_into().unwrap())));
        pos += 72 + (noc.max(0) as usize) * 4;
    }

    if exports.len() < 16587 {
        return Err(TagameSwapError::Msg(format!("TAGame exports table truncated (found {} exports, expected at least 16587)", exports.len())));
    }

    let delta1 = 1500usize;
    let delta2 = 1500usize;
    let total_delta = delta1 + delta2;
    let max_disk_sz0 = 124 + delta1;
    let max_disk_sz_set = 186 + delta2;

    let exp_78_pos = exports[77].0;
    let exp_78_off = exports[77].5 as usize;
    let exp_78_sz = exports[77].4 as usize;

    let exp_set_pos = exports[16587 - 1].0;
    let exp_set_off = exports[16587 - 1].5 as usize;
    let exp_set_sz = exports[16587 - 1].4 as usize;

    // 1. Update export table in plain_header
    let new_sz0 = (exp_78_sz + delta1) as i32;
    plain_header[exp_78_pos + 32..exp_78_pos + 36].copy_from_slice(&new_sz0.to_le_bytes());

    let new_sz_set = (exp_set_sz + delta2) as i32;
    plain_header[exp_set_pos + 32..exp_set_pos + 36].copy_from_slice(&new_sz_set.to_le_bytes());

    for e in &exports {
        let e_off = e.5 as usize;
        let mut shift = 0i64;
        if e_off > exp_78_off {
            shift += delta1 as i64;
        }
        if e_off > exp_set_off {
            shift += delta2 as i64;
        }
        if shift > 0 {
            let new_off = e_off as i64 + shift;
            plain_header[e.0 + 36..e.0 + 44].copy_from_slice(&new_off.to_le_bytes());
        }
    }

    // 2. Update chunk table in plain_header
    let chunk_table_offset = meta.compressed_chunks_offset as usize;
    let c0_entry = chunk_table_offset + 4;

    // Chunk 0 uncompressed size
    let c0_usz = u32::from_le_bytes(plain_header[c0_entry + 8..c0_entry + 12].try_into().unwrap()) as usize + total_delta;
    plain_header[c0_entry + 8..c0_entry + 12].copy_from_slice(&(c0_usz as u32).to_le_bytes());

    // Subsequent chunks uncompressed offset (64-bit integer)
    for i in 1..chunks.len() {
        let p = c0_entry + i * _stride;
        let u_off = u64::from_le_bytes(plain_header[p..p + 8].try_into().unwrap()) + total_delta as u64;
        plain_header[p..p + 8].copy_from_slice(&u_off.to_le_bytes());
    }

    // 3. Decompress Chunk 0, splice deltas right after bytecode (preserving 19-byte trailers), inject bytecodes
    let func_off0_1 = exp_78_off - c0.uncompressed_offset as usize;
    let orig_disk_sz1 = u32::from_le_bytes(decomp0[func_off0_1 + 44..func_off0_1 + 48].try_into().unwrap()) as usize;
    let insert_pos1 = func_off0_1 + 48 + orig_disk_sz1;
    decomp0.splice(insert_pos1..insert_pos1, vec![opcodes::EX_NOTHING; delta1]);

    let (payload0, mem_sz0) = emit_convert_to_client_loadout_bytecode(&slot_overrides, max_disk_sz0)?;
    decomp0[func_off0_1 + 40..func_off0_1 + 44].copy_from_slice(&mem_sz0.to_le_bytes());
    decomp0[func_off0_1 + 44..func_off0_1 + 48].copy_from_slice(&(max_disk_sz0 as u32).to_le_bytes());
    decomp0[func_off0_1 + 48..func_off0_1 + 48 + max_disk_sz0].copy_from_slice(&payload0);

    let func_off0_2 = (exp_set_off + delta1) - c0.uncompressed_offset as usize;
    let orig_disk_sz2 = u32::from_le_bytes(decomp0[func_off0_2 + 44..func_off0_2 + 48].try_into().unwrap()) as usize;
    let insert_pos2 = func_off0_2 + 48 + orig_disk_sz2;
    decomp0.splice(insert_pos2..insert_pos2, vec![opcodes::EX_NOTHING; delta2]);

    let (payload_set, mem_sz_set) = emit_car_set_loadout_bytecode(&slot_overrides, max_disk_sz_set)?;
    decomp0[func_off0_2 + 40..func_off0_2 + 44].copy_from_slice(&mem_sz_set.to_le_bytes());
    decomp0[func_off0_2 + 44..func_off0_2 + 48].copy_from_slice(&(max_disk_sz_set as u32).to_le_bytes());
    decomp0[func_off0_2 + 48..func_off0_2 + 48 + max_disk_sz_set].copy_from_slice(&payload_set);

    let mut recomp0 = crate::upk::compression::compress_chunk(&decomp0)
        .map_err(|e| TagameSwapError::Msg(format!("Failed to compress Chunk 0: {e}")))?;
    let orig_c_sz0 = c0.compressed_size as usize;
    if recomp0.len() > orig_c_sz0 {
        return Err(TagameSwapError::Msg(format!("Chunk 0 compressed size {} exceeded allocation {}", recomp0.len(), orig_c_sz0)));
    }
    recomp0.resize(orig_c_sz0, 0);
    file_bytes[c0.compressed_offset as usize..c0.compressed_offset as usize + orig_c_sz0].copy_from_slice(&recomp0);

    // 4. Decompress Chunk 2 and patch LoadoutValidation_TA::CorrectOnlineData and PRI_TA::ValidateReplicatedLoadout in-place
    if chunks.len() > 2 {
        let c2 = &chunks[2];
        let c2_payload = &file_bytes[c2.compressed_offset as usize..(c2.compressed_offset + c2.compressed_size as i64) as usize];
        let mut decomp2 = crate::upk::compression::decompress_chunk(c2_payload)
            .map_err(|e| TagameSwapError::Msg(format!("Failed to decompress Chunk 2: {e}")))?;

        // CorrectOnlineData (#47859)
        let func_off2_cod = 16098022 - c2.uncompressed_offset as usize;
        let orig_disk_sz_cod = u32::from_le_bytes(decomp2[func_off2_cod + 44..func_off2_cod + 48].try_into().unwrap()) as usize;
        if orig_disk_sz_cod == 3000 {
            let max_disk_sz2 = 3000usize;
            let (payload2, mem_sz2) = emit_correct_online_data_bytecode(&slot_overrides, max_disk_sz2)?;
            decomp2[func_off2_cod + 40..func_off2_cod + 44].copy_from_slice(&mem_sz2.to_le_bytes());
            decomp2[func_off2_cod + 44..func_off2_cod + 48].copy_from_slice(&(max_disk_sz2 as u32).to_le_bytes());
            decomp2[func_off2_cod + 48..func_off2_cod + 48 + max_disk_sz2].copy_from_slice(&payload2);
        }

        // ValidateReplicatedLoadout (#57832) - bypass to prevent online item stripping
        if exports.len() >= 57832 {
            let exp_val_off = exports[57832 - 1].5 as usize;
            let func_off2_val = exp_val_off - c2.uncompressed_offset as usize;
            let orig_disk_sz_val = u32::from_le_bytes(decomp2[func_off2_val + 44..func_off2_val + 48].try_into().unwrap()) as usize;
            if orig_disk_sz_val == 708 {
                let mut val_payload = vec![opcodes::EX_NOTHING; 708];
                val_payload[0] = opcodes::EX_RETURN;
                val_payload[1] = opcodes::EX_NOTHING;
                val_payload[2] = opcodes::EX_END_OF_SCRIPT;
                let val_mem_sz: u32 = 708;
                decomp2[func_off2_val + 40..func_off2_val + 44].copy_from_slice(&val_mem_sz.to_le_bytes());
                decomp2[func_off2_val + 48..func_off2_val + 48 + 708].copy_from_slice(&val_payload);
            }
        }

        let mut recomp2 = crate::upk::compression::compress_chunk(&decomp2)
            .map_err(|e| TagameSwapError::Msg(format!("Failed to compress Chunk 2: {e}")))?;
        let orig_c_sz2 = c2.compressed_size as usize;
        if recomp2.len() > orig_c_sz2 {
            return Err(TagameSwapError::Msg(format!(
                "Chunk 2 compressed size {} exceeded allocation {}",
                recomp2.len(),
                orig_c_sz2
            )));
        }
        recomp2.resize(orig_c_sz2, 0);
        file_bytes[c2.compressed_offset as usize..c2.compressed_offset as usize + orig_c_sz2].copy_from_slice(&recomp2);
    }

    // Re-encrypt header with TAGAME_KEY
    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap());
    p += 4 + if flen > 0 { flen as usize } else { (-flen * 2) as usize };
    p += 4;
    p += 4;
    let name_offset = u32::from_le_bytes(file_bytes[p..p+4].try_into().unwrap()) as usize;
    let garbage_size = 559792;
    let enc_size = total_header_size - garbage_size - name_offset;
    let enc_aligned = (enc_size + 15) & !15;
    let enc_end = name_offset + enc_aligned;

    let re_enc = crypto::encrypt_ecb(&TAGAME_KEY, &plain_header);
    let target_len = enc_end - name_offset;
    file_bytes[name_offset..enc_end].copy_from_slice(&re_enc[..target_len]);

    // Write TAGame.upk safely
    fs::write(&tagame_path, &file_bytes).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to write {}: {e}", tagame_path.display()))
    })?;

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

    Ok(TagameSwapperStatus {
        applied: !applied_swaps.is_empty(),
        backup_present: backup_path.is_file(),
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: applied_swaps,
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
            backup_present: false,
            tagame_path: tagame_path.to_string_lossy().into_owned(),
            active_swaps: Vec::new(),
            message: "No backup found to restore.".to_string(),
        });
    }

    fs::copy(&backup_path, &tagame_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to restore TAGame.upk from backup: {e}"))
    })?;

    // Restore any modified body packages from backup
    if let Ok(entries) = fs::read_dir(cooked_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let lower = file_name.to_lowercase();
                if lower.starts_with("body_") && lower.ends_with(".upk.bak") {
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
        assert_eq!(mem_sz, 176);
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
        assert_eq!(mem_sz, 176);
    }

    #[test]
    fn test_emit_car_and_decal_fit_in_124_bytes() {
        let rules = [
            SlotSwapRule {
                slot_idx: 0,
                owned_id: Some(23),
                target_id: 4284,
            },
            SlotSwapRule {
                slot_idx: 1,
                owned_id: Some(100),
                target_id: 500,
            },
        ];
        let (bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&rules, 124).unwrap();
        assert_eq!(bc.len(), 124);
        assert_eq!(mem_sz, 188);
    }

    #[test]
    fn test_emit_50_swaps_fits_in_expanded_buffers() {
        let mut rules = Vec::new();
        for i in 0..50 {
            rules.push(SlotSwapRule {
                slot_idx: (i % 14) as u8,
                owned_id: Some(100 + i as i32),
                target_id: 2000 + i as i32,
            });
        }

        // Test ConvertToClientLoadout expanded buffer (2933 bytes)
        let (bc0, mem_sz0) = emit_convert_to_client_loadout_bytecode(&rules, 2933).unwrap();
        assert_eq!(bc0.len(), 2933);
        assert!(mem_sz0 > 2933);

        // Test CorrectOnlineData buffer (3000 bytes)
        let (bc2, mem_sz2) = emit_correct_online_data_bytecode(&rules, 3000).unwrap();
        assert_eq!(bc2.len(), 3000);
        assert!(mem_sz2 > 3000);
    }

    #[test]
    fn test_emit_correct_online_data_bytecode() {
        let rules = [SlotSwapRule {
            slot_idx: 0,
            owned_id: Some(23),
            target_id: 4284,
        }];
        let (bc, mem_sz) = emit_correct_online_data_bytecode(&rules, 3000).unwrap();
        assert_eq!(bc.len(), 3000);
        assert_eq!(bc[0], opcodes::EX_JUMP_IF_NOT);
        assert_eq!(bc[3], opcodes::EX_EQUAL_EQUAL_INT_INT);
        assert_eq!(mem_sz, 3024);
    }

    #[test]
    fn test_emit_car_set_loadout_bytecode() {
        let rules = [
            SlotSwapRule {
                slot_idx: 0,
                owned_id: Some(23),
                target_id: 4284,
            },
            SlotSwapRule {
                slot_idx: 3,
                owned_id: Some(33),
                target_id: 32,
            },
        ];
        let max_sz = 186 + 1500;
        let (bc, mem_sz) = emit_car_set_loadout_bytecode(&rules, max_sz).unwrap();
        assert_eq!(bc.len(), max_sz);
        assert!(mem_sz > max_sz as u32);
        assert_eq!(bc[0], opcodes::EX_JUMP_IF_NOT);
        assert_eq!(bc[3], opcodes::EX_EQUAL_EQUAL_INT_INT);
    }
}
