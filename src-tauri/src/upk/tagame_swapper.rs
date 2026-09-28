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
    pub const EX_RETURN_VALUE: u8 = 0x3A;
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

    // 2. Overrides (Conditional slot assignments: if Products[slot] == owned_id -> target_id)
    for rule in slot_overrides {
        let uncond_disk_len = if rule.slot_idx == 0 { 26 } else { 27 };
        let cond_disk_len = if rule.slot_idx == 0 { 56 } else { 58 };

        if let Some(owned_id) = rule.owned_id.filter(|_| bc.len() + cond_disk_len + 12 <= max_disk_size) {
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
        } else if bc.len() + uncond_disk_len + 12 <= max_disk_size {
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
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&75i32.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());
        }
    }

    // 3. return NewLoadout;
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&75i32.to_le_bytes());
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_RETURN_VALUE);
    bc.extend_from_slice(&76i32.to_le_bytes());

    // 4. End of script
    bc.push(opcodes::EX_END_OF_SCRIPT);

    mem_sz += 21; // Return NewLoadout (10) + Return ReturnValue (10) + EOS (1) = 21 mem

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
            let cond_disk_len = if rule.slot_idx == 0 { 56 } else { 58 };
            if bc.len() + cond_disk_len + 3 > max_disk_size {
                break;
            }

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
            let uncond_disk_len = if rule.slot_idx == 0 { 26 } else { 27 };
            if bc.len() + uncond_disk_len + 3 > max_disk_size {
                break;
            }
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
        let uncond_disk_len = if rule.slot_idx == 0 { 26 } else { 27 };
        let cond_disk_len = if rule.slot_idx == 0 { 56 } else { 58 };

        if let Some(owned_id) = rule.owned_id.filter(|_| bc.len() + cond_disk_len + 100 <= max_disk_size) {
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
        } else if bc.len() + uncond_disk_len + 100 <= max_disk_size {
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



#[derive(Debug, Clone)]
pub struct FunctionExport {
    pub name: String,
    pub outer_name: String,
    pub serial_size: usize,
    pub serial_offset: usize,
}

pub fn find_function_exports(
    plain: &[u8],
    summary: &crate::upk::parser::FileSummary,
) -> Result<Vec<FunctionExport>, TagameSwapError> {
    let names = crate::upk::palette::parse_names_in_block(plain, summary.name_count)
        .map_err(|e| TagameSwapError::Msg(e.to_string()))?;
    let export_rel = (summary.export_offset - summary.name_offset) as usize;
    let depends_rel = (summary.depends_offset - summary.name_offset) as usize;

    let mut raw_exports = Vec::new();
    let mut pos = export_rel;
    while pos + 72 <= depends_rel && pos + 72 <= plain.len() && raw_exports.len() < 200_000 {
        let i32_at = |a: usize| i32::from_le_bytes(plain[pos + a..pos + a + 4].try_into().unwrap());
        let noc = i32_at(48);
        raw_exports.push((
            i32_at(8),  // outer_index
            i32_at(12), // name_idx
            i32_at(32) as usize, // serial_size
            i64::from_le_bytes(plain[pos + 36..pos + 44].try_into().unwrap()) as usize, // serial_offset
        ));
        pos += 72 + (noc.max(0) as usize) * 4;
    }

    let mut out = Vec::with_capacity(raw_exports.len());
    for (outer_idx, name_idx, serial_size, serial_offset) in &raw_exports {
        let name = names.get(*name_idx as usize).cloned().unwrap_or_default();
        let outer_name = if *outer_idx > 0 {
            let outer_name_idx = raw_exports.get((*outer_idx - 1) as usize).map(|r| r.1).unwrap_or(0);
            names.get(outer_name_idx as usize).cloned().unwrap_or_default()
        } else {
            String::new()
        };

        out.push(FunctionExport {
            name,
            outer_name,
            serial_size: *serial_size,
            serial_offset: *serial_offset,
        });
    }

    Ok(out)
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
    if !backup_path.is_file() {
        fs::copy(&tagame_path, &backup_path).map_err(|e| {
            TagameSwapError::Msg(format!("Failed to create backup {}: {e}", backup_path.display()))
        })?;
    }

    // 2. Read from backup (or live if palette applied)
    let mut file_bytes = fs::read(&backup_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read {}: {e}", backup_path.display()))
    })?;

    // Check if custom color palette was enabled/applied.
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
        p += 4; // package_flags
        let name_count = i32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
        p += 4;
        let name_offset = u32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap()) as usize;

        let enc_size = (total_header_size - name_offset + 15) & !15;
        let enc_end = name_offset + enc_size;
        if enc_end > file_bytes.len() {
            return Err(TagameSwapError::Msg("Encrypted header bounds invalid".into()));
        }

        let mut plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);

        // Parse Names
        let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count)
            .map_err(|e| TagameSwapError::Msg(e.to_string()))?;

        // Read summary offsets
        let p_sum = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize }) + 4 + 8;
        let export_count = i32::from_le_bytes(file_bytes[p_sum..p_sum + 4].try_into().unwrap()) as usize;
        let export_offset = i32::from_le_bytes(file_bytes[p_sum + 4..p_sum + 8].try_into().unwrap()) as usize;
        let _import_count = i32::from_le_bytes(file_bytes[p_sum + 8..p_sum + 12].try_into().unwrap()) as usize;
        let _import_offset = i32::from_le_bytes(file_bytes[p_sum + 12..p_sum + 16].try_into().unwrap()) as usize;
        let depends_offset = i32::from_le_bytes(file_bytes[p_sum + 16..p_sum + 20].try_into().unwrap()) as usize;

        let export_rel = export_offset - name_offset;
        let depends_rel = depends_offset - name_offset;

        // Walk Exports
        struct ExportItem {
            pos: usize,
            name: String,
            serial_size: i32,
            serial_offset: i64,
        }

        let mut exports = Vec::new();
        let mut pos = export_rel;
        let mut target_exp_idx: Option<usize> = None;
        while pos + 72 <= depends_rel && pos + 72 <= plain_header.len() && exports.len() < export_count {
            let i32_at = |a: usize| i32::from_le_bytes(plain_header[pos + a..pos + a + 4].try_into().unwrap());
            let name_idx = i32_at(12);
            let serial_size = i32_at(32);
            let serial_offset = i64::from_le_bytes(plain_header[pos + 36..pos + 44].try_into().unwrap());
            let noc = i32_at(48);

            let name_str = if name_idx >= 0 && (name_idx as usize) < names.len() {
                names[name_idx as usize].clone()
            } else {
                String::new()
            };

            if name_str == "ConvertToClientLoadout" {
                target_exp_idx = Some(exports.len());
            }

            exports.push(ExportItem {
                pos,
                name: name_str,
                serial_size,
                serial_offset,
            });

            pos += 72 + (noc.max(0) as usize) * 4;
        }

        let target_idx = target_exp_idx.ok_or_else(|| {
            TagameSwapError::Msg("Function ConvertToClientLoadout not found in export table".into())
        })?;
        let target_exp = &exports[target_idx];

        // Parse Chunk Table (located at depends_rel with 36-byte stride)
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

        if chunks.is_empty() {
            return Err(TagameSwapError::Msg("No chunks found in chunk table".into()));
        }

        let c0 = &chunks[0];
        let c0_start = c0.comp_offset as usize;
        let c0_end = c0_start + c0.comp_size as usize;
        if c0_end > file_bytes.len() {
            return Err(TagameSwapError::Msg("Chunk 0 offset out of bounds".into()));
        }

        let mut decomp0 = crate::upk::compression::decompress_chunk(&file_bytes[c0_start..c0_end])
            .map_err(|e| TagameSwapError::Msg(format!("Decompress chunk 0 failed: {e}")))?;

        let func_off0 = (target_exp.serial_offset - c0.uncomp_offset) as usize;
        if func_off0 + 48 > decomp0.len() {
            return Err(TagameSwapError::Msg("ConvertToClientLoadout offset out of bounds in Chunk 0".into()));
        }

        let orig_disk_sz = u32::from_le_bytes(decomp0[func_off0 + 44..func_off0 + 48].try_into().unwrap()) as usize;

        // Collect slot overrides
        let mut slot_overrides = Vec::new();
        for s in swaps {
            let slot_idx = if let Some(idx) = s.slot_index {
                idx as u8
            } else {
                crate::presets::slot_index_from_str(&s.slot) as u8
            };
            let pid = if s.product_id > 0 { s.product_id } else { 4284 };
            slot_overrides.push(SlotSwapRule {
                slot_idx,
                owned_id: s.owned_id,
                target_id: pid,
            });

            // If owned_id is a default item (e.g. 1903 for Classic Goal Explosion, 1902 for Classic Trail, etc.)
            // also add rule for owned_id = Some(0) so in-game empty/0 slot values get overridden!
            if let Some(oid) = s.owned_id {
                let is_default = oid == 0 || oid == 1903 || oid == 1902 || oid == 3753 || oid == 3247;
                if is_default && oid != 0 {
                    slot_overrides.push(SlotSwapRule {
                        slot_idx,
                        owned_id: Some(0),
                        target_id: pid,
                    });
                }
            }
        }

        const EXPANDED_SIZE: usize = 3000;

        if orig_disk_sz < EXPANDED_SIZE {
            let delta = EXPANDED_SIZE - orig_disk_sz;
            let insert_pos = func_off0 + 48 + orig_disk_sz;
            if insert_pos > decomp0.len() {
                return Err(TagameSwapError::Msg("Insertion position out of bounds in Chunk 0".into()));
            }

            decomp0.splice(insert_pos..insert_pos, std::iter::repeat(opcodes::EX_NOTHING).take(delta));

            let (payload0, mem_sz0) = emit_convert_to_client_loadout_bytecode(&slot_overrides, EXPANDED_SIZE)?;
            decomp0[func_off0 + 40..func_off0 + 44].copy_from_slice(&mem_sz0.to_le_bytes());
            decomp0[func_off0 + 44..func_off0 + 48].copy_from_slice(&(EXPANDED_SIZE as u32).to_le_bytes());
            decomp0[func_off0 + 48..func_off0 + 48 + EXPANDED_SIZE].copy_from_slice(&payload0);

            // 1. Update Export Table in plain_header
            let new_serial_sz = target_exp.serial_size + delta as i32;
            plain_header[target_exp.pos + 32..target_exp.pos + 36].copy_from_slice(&new_serial_sz.to_le_bytes());
            for exp in &exports {
                if exp.serial_offset > target_exp.serial_offset {
                    let new_s_off = exp.serial_offset + delta as i64;
                    plain_header[exp.pos + 36..exp.pos + 44].copy_from_slice(&new_s_off.to_le_bytes());
                }
            }

            // 2. Update Chunk Table in plain_header
            let new_c0_uncomp_sz = c0.uncomp_size + delta as i32;
            plain_header[c0.pos + 8..c0.pos + 12].copy_from_slice(&new_c0_uncomp_sz.to_le_bytes());
            for ch in &chunks[1..] {
                let new_u_off = ch.uncomp_offset + delta as i64;
                plain_header[ch.pos..ch.pos + 8].copy_from_slice(&new_u_off.to_le_bytes());
            }
        } else {
            let target_sz = orig_disk_sz;
            let (payload0, mem_sz0) = emit_convert_to_client_loadout_bytecode(&slot_overrides, target_sz)?;
            decomp0[func_off0 + 40..func_off0 + 44].copy_from_slice(&mem_sz0.to_le_bytes());
            decomp0[func_off0 + 48..func_off0 + 48 + target_sz].copy_from_slice(&payload0);
        }

        // 3. Recompress Chunk 0
        let mut recomp0 = crate::upk::compression::compress_chunk(&decomp0)
            .map_err(|e| TagameSwapError::Msg(format!("Compress Chunk 0 failed: {e}")))?;
        let orig_c0_sz = c0.comp_size as usize;
        if recomp0.len() > orig_c0_sz {
            return Err(TagameSwapError::Msg(format!(
                "Recompressed Chunk 0 size ({}) exceeds original allocation ({})",
                recomp0.len(),
                orig_c0_sz
            )));
        }
        recomp0.resize(orig_c0_sz, 0);
        file_bytes[c0_start..c0_start + orig_c0_sz].copy_from_slice(&recomp0);

        // 4. Re-encrypt Header cleanly
        let re_enc = crypto::encrypt_ecb(&TAGAME_KEY, &plain_header);
        file_bytes[name_offset..enc_end].copy_from_slice(&re_enc);

        // 5. Write to TAGame.upk
        fs::write(&tagame_path, &file_bytes)?;
    } else {
        if !pal_st.applied && backup_path.is_file() {
            let _ = fs::copy(&backup_path, &tagame_path);
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
        assert_eq!(mem_sz, 168);
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
        assert_eq!(mem_sz, 180);
    }

    #[test]
    fn test_emit_car_and_decal_fit_in_expanded_buffer() {
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
        let (bc, mem_sz) = emit_convert_to_client_loadout_bytecode(&rules, 500).unwrap();
        assert_eq!(bc.len(), 500);
        assert_eq!(bc[0], opcodes::EX_LET);
        assert_eq!(bc[33], opcodes::EX_JUMP_IF_NOT);
        assert!(mem_sz > 500);
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
