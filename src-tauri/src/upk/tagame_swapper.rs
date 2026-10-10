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
    pub const EX_DYN_ARRAY_ELEMENT: u8 = 0x5E;
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
    pub const EX_FALSE_CONST: u8 = 0x28;
    pub const EX_FINAL_FUNCTION: u8 = 0x1C;
    pub const EX_VIRTUAL_FUNCTION: u8 = 0x1B;
    pub const EX_FLOAT_CONST: u8 = 0x1E;
    pub const EX_BOOL_VARIABLE: u8 = 0x2D;
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

#[inline]
pub fn is_car_loadout_slot(slot_idx: u8) -> bool {
    matches!(slot_idx, 0 | 1 | 2 | 3 | 4 | 5 | 7 | 12 | 13 | 14)
}

pub fn emit_car_set_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
    products_id_opt: Option<i32>,
    cld_id_opt: Option<i32>,
    data_var_id_opt: Option<i32>,
    vanilla_tail_opt: Option<&[u8]>,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    let products_id = products_id_opt.unwrap_or(1871);
    let cld_id = cld_id_opt.unwrap_or(1872);
    let data_var_id = data_var_id_opt.unwrap_or(16586);

    // 1. Slot assignments on Data (local #data_var_id)
    // De-duplicate by slot_idx so highest priority rule applies.
    // Car_TA manages vehicle-attached cosmetics: slots 0 (Body), 1 (Decal), 2 (Wheels),
    // 3 (Boost), 4 (Antenna), 5 (Topper), 7 (PaintFinish), 12 (Accent), 13 (Audio), 14 (Trail).
    let mut seen = std::collections::HashSet::new();
    let mut unique_rules = Vec::new();
    for r in slot_overrides.iter().rev() {
        if is_car_loadout_slot(r.slot_idx) && seen.insert(r.slot_idx) {
            unique_rules.push(r.clone());
        }
    }
    unique_rules.reverse();

    let trigger_opt = unique_rules.iter().find(|r| r.slot_idx == 0 && r.owned_id.is_some())
        .or_else(|| unique_rules.iter().find(|r| r.owned_id.is_some()))
        .cloned();

    let vanilla_tail_len = 97;
    let available_space = max_disk_size.saturating_sub(vanilla_tail_len);

    if let Some(trigger) = trigger_opt {
        let owned_id = trigger.owned_id.unwrap();

        // 1. Unified condition header: if (Data.Products[trigger.slot_idx] == owned_id)
        bc.push(opcodes::EX_JUMP_IF_NOT);
        let jump_pos = bc.len();
        bc.extend_from_slice(&[0x00, 0x00]);

        bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
        bc.push(opcodes::EX_DYN_ARRAY_OP);
        bc.extend_from_slice(&[0x00, 0x00]);
        bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
        let cond_index_mem = if trigger.slot_idx == 0 {
            bc.push(opcodes::EX_INT_ZERO);
            1u32
        } else {
            bc.push(opcodes::EX_INT_CONST_BYTE);
            bc.push(trigger.slot_idx);
            2u32
        };
        bc.push(opcodes::EX_STRUCT_MEMBER);
        bc.extend_from_slice(&products_id.to_le_bytes());
        bc.extend_from_slice(&cld_id.to_le_bytes());
        bc.extend_from_slice(&[0x00, 0x01]);
        bc.push(opcodes::EX_LOCAL_VARIABLE);
        bc.extend_from_slice(&data_var_id.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&owned_id.to_le_bytes());
        bc.push(opcodes::EX_END_FUNCTION_PARMS);

        let cond_mem = 3 + 1 + 3 + 1 + cond_index_mem + 28 + 5 + 1;
        mem_sz += cond_mem;

        // 2. Body assignments: trigger rule first, then remaining unique rules
        let mut ordered_rules = Vec::new();
        ordered_rules.push(trigger);
        for r in &unique_rules {
            if r.slot_idx != trigger.slot_idx {
                ordered_rules.push(r.clone());
            }
        }

        for rule in ordered_rules {
            let index_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
            let assign_disk_len = 26 + index_disk_len;
            if bc.len() + assign_disk_len > available_space {
                break;
            }

            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
            let assign_index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&products_id.to_le_bytes());
            bc.extend_from_slice(&cld_id.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&data_var_id.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let assign_mem = 1 + 3 + 1 + assign_index_mem + 28 + 5;
            mem_sz += assign_mem;
        }

        // Jump target is local bytecode offset immediately after body
        let jump_target = bc.len() as u16;
        bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
    } else {
        // Pure unconditional rules (only if caller explicitly provided no owned_ids)
        for rule in &unique_rules {
            let index_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
            let assign_disk_len = 26 + index_disk_len;
            if bc.len() + assign_disk_len > available_space {
                break;
            }
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
            let assign_index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&products_id.to_le_bytes());
            bc.extend_from_slice(&cld_id.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&data_var_id.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let assign_mem = 1 + 3 + 1 + assign_index_mem + 28 + 5;
            mem_sz += assign_mem;
        }
    }

    // 2. Exact vanilla execution body:
    // bLoadoutSet = true; Loadout.EventAssetLoaded = ...; Loadout.EventAllAssetsLoaded = ...; ProductLoader.LoadClientLoadout(Data); return;
    let default_vanilla_body: [u8; 97] = [
        0x14, 0x2D, 0x01, 0x8D, 0x40, 0x00, 0x00, 0x27,
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF8, 0x3E, 0x00, 0x00, 0x00, 0x01, 0xF8, 0x3E, 0x00, 0x00, 0x49, 0x8B, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x52, 0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x09, 0x00, 0xF7, 0x3E, 0x00, 0x00, 0x00, 0x01, 0xF7, 0x3E, 0x00, 0x00, 0x49, 0x7D, 0x5C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x5E, 0x19, 0x00, 0x01, 0x8E, 0x40, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x1C, 0x10, 0x3F, 0x00, 0x00, 0x46, 0xCA, 0x40, 0x00, 0x00, 0x16,
        0x04, 0x0B, 0x4C
    ];
    let vanilla_body = match vanilla_tail_opt {
        Some(tail) if tail.len() == 97 => tail,
        _ => &default_vanilla_body[..],
    };
    bc.extend_from_slice(vanilla_body);
    // Exact vanilla execution body UScript memory size in 64-bit RocketLeague.exe:
    // Stmt 1 (EX_LET_BOOL): 11
    // Stmt 2 (EX_DELEGATE_PROPERTY): 40
    // Stmt 3 (EX_DELEGATE_PROPERTY): 40
    // Stmt 4 (EX_DYN_ARRAY_ELEMENT): 20 (64-bit object pointer expansion)
    // Stmt 5 (EX_VIRTUAL_FUNCTION LoadClientLoadout): 19
    // Stmt 6 (EX_RETURN; EX_NOTHING; EX_END_OF_SCRIPT): 3
    // Total for the 97-byte vanilla execution body is exactly 133 UScript memory bytes.
    mem_sz += 133;

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

#[allow(dead_code)]
pub fn emit_car_preview_set_loadout_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
    products_id_opt: Option<i32>,
    loadout_data_id_opt: Option<i32>,
    new_loadout_id_opt: Option<i32>,
    in_loadout_id_opt: Option<i32>,
    force_set_loadout_name_idx_opt: Option<i32>,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    let products_id = products_id_opt.unwrap_or(1871);
    let loadout_data_id = loadout_data_id_opt.unwrap_or(2364);
    let new_loadout_id = new_loadout_id_opt.unwrap_or(18093);
    let in_loadout_id = in_loadout_id_opt.unwrap_or(18094);
    let force_set_loadout_name_idx = force_set_loadout_name_idx_opt.unwrap_or(20264);

    // 1. Assignment: NewLoadout = InLoadout; (11 bytes disk, 19 mem)
    bc.push(opcodes::EX_LET);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&new_loadout_id.to_le_bytes());
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&in_loadout_id.to_le_bytes());
    mem_sz += 19;

    // Filter to vehicle cosmetics: slots 0, 1, 2, 3, 4, 5, 7, 12, 13, 14
    let mut seen = std::collections::HashSet::new();
    let mut unique_rules = Vec::new();
    for r in slot_overrides.iter().rev() {
        if is_car_loadout_slot(r.slot_idx) && seen.insert(r.slot_idx) {
            unique_rules.push(r.clone());
        }
    }
    unique_rules.reverse();

    let trigger_opt = unique_rules.iter().find(|r| r.slot_idx == 0 && r.owned_id.is_some())
        .or_else(|| unique_rules.iter().find(|r| r.owned_id.is_some()))
        .cloned();

    // 2. Slot override rules on NewLoadout
    // ForceSetLoadout tail call is 19 bytes disk
    let tail_disk_len = 19;
    let available_space = max_disk_size.saturating_sub(tail_disk_len);

    if let Some(trigger) = trigger_opt {
        let owned_id = trigger.owned_id.unwrap();

        // Unified condition: if (NewLoadout.Products[trigger.slot_idx] == owned_id)
        bc.push(opcodes::EX_JUMP_IF_NOT);
        let jump_pos = bc.len();
        bc.extend_from_slice(&[0x00, 0x00]);

        bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
        bc.push(opcodes::EX_DYN_ARRAY_OP);
        bc.extend_from_slice(&[0x00, 0x00]);
        bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
        let cond_index_mem = if trigger.slot_idx == 0 {
            bc.push(opcodes::EX_INT_ZERO);
            1u32
        } else {
            bc.push(opcodes::EX_INT_CONST_BYTE);
            bc.push(trigger.slot_idx);
            2u32
        };
        bc.push(opcodes::EX_STRUCT_MEMBER);
        bc.extend_from_slice(&products_id.to_le_bytes());
        bc.extend_from_slice(&loadout_data_id.to_le_bytes());
        bc.extend_from_slice(&[0x00, 0x01]);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&new_loadout_id.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&owned_id.to_le_bytes());
        bc.push(opcodes::EX_END_FUNCTION_PARMS);

        let cond_mem = 3 + 1 + 3 + 1 + cond_index_mem + 28 + 5 + 1;
        mem_sz += cond_mem;

        let mut ordered_rules = Vec::new();
        ordered_rules.push(trigger);
        for r in &unique_rules {
            if r.slot_idx != trigger.slot_idx {
                ordered_rules.push(r.clone());
            }
        }

        for rule in ordered_rules {
            let index_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
            let assign_disk_len = 26 + index_disk_len;
            if bc.len() + assign_disk_len > available_space {
                break;
            }

            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
            let assign_index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&products_id.to_le_bytes());
            bc.extend_from_slice(&loadout_data_id.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&new_loadout_id.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let assign_mem = 1 + 3 + 1 + assign_index_mem + 28 + 5;
            mem_sz += assign_mem;
        }

        let jump_target = bc.len() as u16;
        bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
    } else {
        for rule in &unique_rules {
            let index_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
            let assign_disk_len = 26 + index_disk_len;
            if bc.len() + assign_disk_len > available_space {
                break;
            }
            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_DYN_ARRAY_OP);
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
            let assign_index_mem = if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
                1u32
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
                2u32
            };
            bc.push(opcodes::EX_STRUCT_MEMBER);
            bc.extend_from_slice(&products_id.to_le_bytes());
            bc.extend_from_slice(&loadout_data_id.to_le_bytes());
            bc.extend_from_slice(&[0x00, 0x01]);
            bc.push(opcodes::EX_INSTANCE_VARIABLE);
            bc.extend_from_slice(&new_loadout_id.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&rule.target_id.to_le_bytes());

            let assign_mem = 1 + 3 + 1 + assign_index_mem + 28 + 5;
            mem_sz += assign_mem;
        }
    }

    // 3. Stmt: ForceSetLoadout(NewLoadout, false); return;
    bc.push(opcodes::EX_VIRTUAL_FUNCTION);
    bc.extend_from_slice(&force_set_loadout_name_idx.to_le_bytes());
    bc.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&new_loadout_id.to_le_bytes());
    bc.push(opcodes::EX_FALSE_CONST);
    bc.push(opcodes::EX_END_FUNCTION_PARMS);
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_NOTHING);
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 31;

    // 4. Pad with EX_NOTHING up to max_disk_size
    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

pub fn emit_explosion_previewer_set_loadout_bytecode(
    target_ge: i32,
    max_disk_size: usize,
    vanilla_code_opt: Option<&[u8]>,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::with_capacity(max_disk_size);

    // 1. Call SetProduct header (5 bytes: EX_FINAL_FUNCTION SetProduct)
    if let Some(code) = vanilla_code_opt {
        if code.len() >= 5 {
            bc.extend_from_slice(&code[0..5]);
        } else {
            bc.extend_from_slice(&[0x1C, 0xBF, 0x5A, 0x00, 0x00]);
        }
    } else {
        bc.extend_from_slice(&[0x1C, 0xBF, 0x5A, 0x00, 0x00]);
    }

    // 2. Arg 1: ProductID = EX_INT_CONST target_ge (5 bytes)
    bc.push(opcodes::EX_INT_CONST);
    bc.extend_from_slice(&target_ge.to_le_bytes());

    // 3. Arg 2: InLoadout.OnlineProducts[slot] (53 bytes)
    let default_arg2: [u8; 53] = [
        0x57, 0x00, 0x00, 0x5e, 0x19, 0x00, 0x12, 0x00, 0x20, 0x16, 0x64, 0x00, 0x00, 0x09, 0x00, 0xd3,
        0x63, 0x00, 0x00, 0x00, 0x02, 0xd3, 0x63, 0x00, 0x00, 0x09, 0x00, 0x5d, 0x26, 0x00, 0x00, 0x00,
        0x01, 0x5d, 0x26, 0x00, 0x00, 0x35, 0x38, 0x09, 0x00, 0x00, 0x3c, 0x09, 0x00, 0x00, 0x00, 0x00,
        0x46, 0xb9, 0x5a, 0x00, 0x00,
    ];
    let arg2 = match vanilla_code_opt {
        Some(code) if code.len() >= 111 => &code[58..111],
        _ => &default_arg2[..],
    };
    bc.extend_from_slice(arg2);

    // 4. End parms, Return, Nothing, End of Script (4 bytes)
    bc.extend_from_slice(&[
        opcodes::EX_END_FUNCTION_PARMS,
        opcodes::EX_RETURN,
        opcodes::EX_NOTHING,
        opcodes::EX_END_OF_SCRIPT,
    ]);

    // Memory size for the 67-byte active code:
    // EX_FINAL_FUNCTION: 9
    // EX_INT_CONST: 5
    // Arg 2 (OnlineProducts array element expression): 65
    // EX_END_FUNCTION_PARMS: 1
    // EX_RETURN: 1
    // EX_NOTHING: 1
    // EX_END_OF_SCRIPT: 1
    // Total active memory size: 83
    let mut mem_sz = 83u32;

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

pub fn emit_car_mesh_apply_paint_settings_bytecode(
    rgba: [u8; 16],
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::with_capacity(max_disk_size);
    let mut mem_sz: u32 = 0;

    // 1. EX_JUMP_IF_NOT: if (bLocalPlayer)
    bc.push(opcodes::EX_JUMP_IF_NOT);
    let jump_pos = bc.len();
    bc.extend_from_slice(&[0x00, 0x00]);

    // Condition: bLocalPlayer (Export #15648)
    bc.push(opcodes::EX_BOOL_VARIABLE);
    bc.extend_from_slice(&15648i32.to_le_bytes());
    bc.push(0x00);
    mem_sz += 3 + 8;

    // 2. Struct member assignments on CustomColorOverride (Export #15655)
    // LinearColor struct (-4233)
    // R: -1580, G: -1579, B: -1578, A: -1577
    let members: [(i32, &[u8]); 4] = [
        (-1580i32, &rgba[0..4]),   // R
        (-1579i32, &rgba[4..8]),   // G
        (-1578i32, &rgba[8..12]),  // B
        (-1577i32, &rgba[12..16]), // A
    ];

    for (prop_id, val_bytes) in members {
        bc.push(opcodes::EX_LET);
        bc.push(opcodes::EX_STRUCT_MEMBER);
        bc.extend_from_slice(&prop_id.to_le_bytes());
        bc.extend_from_slice(&(-4233i32).to_le_bytes());
        bc.extend_from_slice(&[0x00, 0x01]);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&15655i32.to_le_bytes());
        bc.push(opcodes::EX_FLOAT_CONST);
        bc.extend_from_slice(val_bytes);

        mem_sz += 1 + 18 + 8 + 4;
    }

    let jump_target = bc.len() as u16;
    bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());

    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_NOTHING);
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 3;

    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

pub fn emit_explosion_previewer_set_product_bytecode(
    owned_id_opt: Option<i32>,
    target_id: i32,
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::with_capacity(max_disk_size);
    let mut mem_sz: u32 = 0;
    let product_id_var = 23230i32; // ProductID local parameter

    if let Some(owned) = owned_id_opt {
        // if (ProductID == owned) ProductID = target;
        bc.push(opcodes::EX_JUMP_IF_NOT);
        let jump_pos = bc.len();
        bc.extend_from_slice(&[0x00, 0x00]);

        bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&product_id_var.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&owned.to_le_bytes());
        bc.push(opcodes::EX_END_FUNCTION_PARMS);

        bc.push(opcodes::EX_LET);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&product_id_var.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&target_id.to_le_bytes());

        let jump_target = bc.len() as u16;
        bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());

        mem_sz += 3 + (1 + 8 + 4 + 1) + (1 + 8 + 4);
    } else {
        // ProductID = target;
        bc.push(opcodes::EX_LET);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&product_id_var.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&target_id.to_le_bytes());

        mem_sz += 1 + 8 + 4;
    }

    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_NOTHING);
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 3;

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

pub fn apply_item_paint_modification(
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

    // 1. If Rocket League has a dedicated painted package file, copy it!
    if paint_id > 0 {
        if let Some(painted_pkg) = crate::upk::swapper::find_painted_package(cooked_dir, package_name, paint_id) {
            if let Some((src_painted, _)) = crate::upk::swapper::resolve_package_path(cooked_dir, &painted_pkg) {
                if src_painted.is_file() && src_painted != pkg_path {
                    let _ = fs::copy(&src_painted, &pkg_path);
                    return Ok(());
                }
            }
        }
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

    // Replace material instance vector parameter patterns across decompressed chunk
    let patterns = [
        // 0.0663 f32 (CustomColor stock on Fennec/Dominus/Octane)
        [0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x25, 0xc6, 0x87, 0x3d, 0x00, 0x00, 0x80, 0x3f],
        // 0.12 f32 (TrimColor stock)
        [0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x8f, 0xc2, 0xf5, 0x3d, 0x00, 0x00, 0x80, 0x3f],
        // 0.05 f32
        [0xcd, 0xcc, 0x4c, 0x3d, 0xcd, 0xcc, 0x4c, 0x3d, 0xcd, 0xcc, 0x4c, 0x3d, 0x00, 0x00, 0x80, 0x3f],
        // 0.02 f32 (dark trim / wheels)
        [0xa4, 0x70, 0x9d, 0x3c, 0xa4, 0x70, 0x9d, 0x3c, 0xa4, 0x70, 0x9d, 0x3c, 0x00, 0x00, 0x80, 0x3f],
        // 0.01 f32 (black trim / wheels)
        [0x0a, 0xd7, 0x23, 0x3c, 0x0a, 0xd7, 0x23, 0x3c, 0x0a, 0xd7, 0x23, 0x3c, 0x00, 0x00, 0x80, 0x3f],
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

#[inline]
pub fn apply_body_paint_modification(
    cooked_dir: &Path,
    package_name: &str,
    paint_id: i32,
    custom_hex: Option<&str>,
    keys_map_json: &str,
) -> Result<(), TagameSwapError> {
    apply_item_paint_modification(cooked_dir, package_name, paint_id, custom_hex, keys_map_json)
}

pub fn apply_tagame_modifications(
    cooked_dir: &Path,
    swaps: &[TagameSwapItem],
    keys_txt: &str,
    keys_map_json: &str,
    palette_enabled: Option<bool>,
) -> Result<TagameSwapperStatus, TagameSwapError> {
    let tagame_path = cooked_dir.join("TAGame.upk");
    let backup_path = cooked_dir.join(TAGAME_BACKUP_NAME);

    if !tagame_path.is_file() {
        return Err(TagameSwapError::Msg(format!(
            "TAGame.upk not found at {}",
            tagame_path.display()
        )));
    }

    let engine_path = cooked_dir.join("Engine.upk");
    let engine_guid = crate::upk::swapper::read_package_guid(&engine_path);

    // Auto-detect Rocket League game updates:
    // If the game was updated, Engine.upk will have a new package GUID that does not match the backup.
    // Note: TAGame.upk has an AES-encrypted header, so we decrypt and check the export/import table references
    // via `backup_references_stale_engine` rather than naive raw-byte window searches.
    let backup_is_stale = if backup_path.is_file() {
        if let Some(eg) = engine_guid {
            crate::upk::palette::backup_references_stale_engine(
                &backup_path,
                &eg,
                keys_txt,
                keys_map_json,
            )
        } else {
            false
        }
    } else {
        false
    };

    let live_is_vanilla = crate::upk::palette::is_vanilla_stock_file(&tagame_path, keys_txt, keys_map_json);
    if !backup_path.is_file() || backup_is_stale {
        if live_is_vanilla {
            crate::applog::event("apply_tagame_modifications: game update detected or backup missing, refreshing TAGame.upk.bak from vanilla stock file");
            fs::copy(&tagame_path, &backup_path).map_err(|e| {
                TagameSwapError::Msg(format!("Failed to create backup {}: {e}", backup_path.display()))
            })?;
        } else if !backup_path.is_file() {
            crate::applog::event("apply_tagame_modifications: backup missing and live file is modded; creating backup with warning");
            let _ = fs::copy(&tagame_path, &backup_path);
        }
    }

    let mut file_bytes = fs::read(&backup_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read {}: {e}", backup_path.display()))
    })?;

    let should_apply_palette = palette_enabled.unwrap_or_else(|| {
        let pal_st = crate::upk::palette::read_palette_status(cooked_dir, None);
        pal_st.applied
    });

    if should_apply_palette {
        if let Ok(_) = crate::upk::palette::apply_rich_palette_to_file(cooked_dir, keys_txt, keys_map_json) {
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
            let slot_idx = if let Some(idx) = s.slot_index {
                idx as u8
            } else {
                crate::presets::upk_slot_index_from_str(&s.slot)
            };

            slot_overrides.push(SlotSwapRule {
                slot_idx,
                owned_id: s.owned_id,
                target_id: pid,
            });
        }

        // 1. Patch Chunk 0: Car_TA::SetLoadout strictly within vanilla allocation (186 bytes)
        if !chunks.is_empty() {
            let c0_uncomp_offset = chunks[0].uncomp_offset;
            let c0_comp_offset = chunks[0].comp_offset;
            let c0_comp_size = chunks[0].comp_size;
            let c0_start = c0_comp_offset as usize;
            let c0_end = c0_start + c0_comp_size as usize;
            if c0_end <= file_bytes.len() {
                if let Ok(mut decomp0) = crate::upk::compression::decompress_chunk(&file_bytes[c0_start..c0_end]) {
                    // Patch Car_TA::SetLoadout in Chunk 0 for vehicle spawn (strictly within vanilla 186 bytes)
                    if let Some(exp) = exports.iter().find(|e| e.name == "SetLoadout" && e.outer_name == "Car_TA") {
                        let func_off = (exp.serial_offset - c0_uncomp_offset) as usize;
                        if func_off + 48 <= decomp0.len() {
                            let orig_disk_sz = u32::from_le_bytes(decomp0[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                            if orig_disk_sz >= 97 && func_off + 48 + orig_disk_sz <= decomp0.len() {
                                // Dynamically resolve Data, ClientLoadoutData, and Products export indices
                                let set_loadout_idx = exp.idx as i32;
                                let data_var_idx = exports.iter()
                                    .find(|e| e.name == "Data" && e.outer_idx == set_loadout_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(16586);
                                let cld_idx = exports.iter()
                                    .find(|e| e.name == "ClientLoadoutData")
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(1872);
                                let products_idx = exports.iter()
                                    .find(|e| e.name == "Products" && e.outer_idx == cld_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(1871);

                                // Dynamically extract vanilla body if present in orig_script:
                                // Looks for the vanilla execution body starting at [0x14, 0x2D] up to and including [0x04, 0x0B, 0x4C].
                                let orig_script = &decomp0[func_off + 48..func_off + 48 + orig_disk_sz];
                                let vanilla_tail = if let Some(start_pos) = orig_script.windows(2).position(|w| w == [0x14, 0x2D]) {
                                    if let Some(end_rel) = orig_script[start_pos..].windows(3).position(|w| w == [0x04, 0x0B, 0x4C]) {
                                        let tail_len = end_rel + 3;
                                        if tail_len >= 90 && tail_len <= 110 {
                                            Some(&orig_script[start_pos..start_pos + tail_len])
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };

                                if let Ok((payload, mem_sz)) = emit_car_set_loadout_bytecode(
                                    &slot_overrides,
                                    orig_disk_sz,
                                    Some(products_idx),
                                    Some(cld_idx),
                                    Some(data_var_idx),
                                    vanilla_tail,
                                ) {
                                    decomp0[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                    decomp0[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                                }
                            }
                        }
                    }

                    // Patch CarPreviewActor_TA::SetLoadout (230 bytes) in Chunk 0 for garage pedestal preview
                    if let Some(exp) = exports.iter().find(|e| e.name == "SetLoadout" && e.outer_name == "CarPreviewActor_TA") {
                        let func_off = (exp.serial_offset - c0_uncomp_offset) as usize;
                        if func_off + 48 <= decomp0.len() {
                            let orig_disk_sz = u32::from_le_bytes(decomp0[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                            if orig_disk_sz >= 30 && func_off + 48 + orig_disk_sz <= decomp0.len() {
                                let set_loadout_idx = exp.idx as i32;
                                let new_loadout_idx = exports.iter()
                                    .find(|e| e.name == "NewLoadout" && e.outer_idx == set_loadout_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(18093);
                                let in_loadout_idx = exports.iter()
                                    .find(|e| e.name == "InLoadout" && e.outer_idx == set_loadout_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(18094);
                                let loadout_data_idx = exports.iter()
                                    .find(|e| e.name == "LoadoutData")
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(2364);
                                let products_idx = exports.iter()
                                    .find(|e| e.name == "Products" && e.outer_idx == loadout_data_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(1871);
                                let force_set_idx = names.iter()
                                    .position(|n| n == "ForceSetLoadout")
                                    .map(|i| i as i32)
                                    .unwrap_or(20264);

                                if let Ok((payload, mem_sz)) = emit_car_preview_set_loadout_bytecode(
                                    &slot_overrides,
                                    orig_disk_sz,
                                    Some(products_idx),
                                    Some(loadout_data_idx),
                                    Some(new_loadout_idx),
                                    Some(in_loadout_idx),
                                    Some(force_set_idx),
                                ) {
                                    decomp0[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                    decomp0[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                                }
                            }
                        }
                    }

                    // Patch CarMeshComponentBase_TA::ApplyPaintSettings (636 bytes) if custom paint is present
                    let custom_paint = swaps.iter().find_map(|s| {
                        if let Some(ref hex) = s.custom_paint_hex.as_ref().filter(|h| !h.trim().is_empty()) {
                            Some(hex_to_linear_rgba(hex))
                        } else if let Some(pid) = s.paint_id.filter(|p| *p > 0) {
                            Some(get_paint_rgba(pid))
                        } else {
                            None
                        }
                    });

                    if let Some(paint_rgba) = custom_paint {
                        if let Some(exp) = exports.iter().find(|e| e.name == "ApplyPaintSettings" && e.outer_name == "CarMeshComponentBase_TA") {
                            let func_off = (exp.serial_offset - c0_uncomp_offset) as usize;
                            if func_off + 48 <= decomp0.len() {
                                let orig_disk_sz = u32::from_le_bytes(decomp0[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                                if orig_disk_sz >= 84 && func_off + 48 + orig_disk_sz <= decomp0.len() {
                                    if let Ok((payload, mem_sz)) = emit_car_mesh_apply_paint_settings_bytecode(
                                        paint_rgba,
                                        orig_disk_sz,
                                    ) {
                                        decomp0[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                        decomp0[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                                    }
                                }
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

        // 2. Patch Chunk 1: ExplosionPreviewer_TA::SetLoadout (115 bytes) & SetProduct (273 bytes) if a goal explosion swap is present
        let goal_explosion_target = swaps.iter().find_map(|s| {
            let slot_idx = s.slot_index.map(|i| i as u8).unwrap_or_else(|| crate::presets::upk_slot_index_from_str(&s.slot));
            if slot_idx == 15 || s.slot.to_lowercase().contains("goal") || s.slot.to_lowercase().contains("explosion") {
                Some(if s.product_id > 0 { s.product_id } else { 2044 })
            } else {
                None
            }
        });

        if let Some(target_ge) = goal_explosion_target {
            if chunks.len() > 1 {
                let c1_uncomp_offset = chunks[1].uncomp_offset;
                let c1_comp_offset = chunks[1].comp_offset;
                let c1_comp_size = chunks[1].comp_size;
                let c1_start = c1_comp_offset as usize;
                let c1_end = c1_start + c1_comp_size as usize;
                if c1_end <= file_bytes.len() {
                    if let Ok(mut decomp1) = crate::upk::compression::decompress_chunk(&file_bytes[c1_start..c1_end]) {
                        if let Some(exp) = exports.iter().find(|e| e.name == "SetLoadout" && e.outer_name == "ExplosionPreviewer_TA") {
                            let func_off = (exp.serial_offset - c1_uncomp_offset) as usize;
                            if func_off + 48 <= decomp1.len() {
                                let orig_disk_sz = u32::from_le_bytes(decomp1[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                                if orig_disk_sz >= 67 && func_off + 48 + orig_disk_sz <= decomp1.len() {
                                    let vanilla_code = &decomp1[func_off + 48..func_off + 48 + orig_disk_sz];
                                    if let Ok((payload, mem_sz)) = emit_explosion_previewer_set_loadout_bytecode(
                                        target_ge,
                                        orig_disk_sz,
                                        Some(vanilla_code),
                                    ) {
                                        decomp1[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                        decomp1[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                                    }
                                }
                            }
                        }

                        if let Some(exp_sp) = exports.iter().find(|e| e.name == "SetProduct" && e.outer_name == "ExplosionPreviewer_TA") {
                            let func_off = (exp_sp.serial_offset - c1_uncomp_offset) as usize;
                            if func_off + 48 <= decomp1.len() {
                                let orig_disk_sz = u32::from_le_bytes(decomp1[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                                if orig_disk_sz >= 30 && func_off + 48 + orig_disk_sz <= decomp1.len() {
                                    let owned_ge = swaps.iter().find_map(|s| {
                                        let slot_idx = s.slot_index.map(|i| i as u8).unwrap_or_else(|| crate::presets::upk_slot_index_from_str(&s.slot));
                                        if slot_idx == 15 || s.slot.to_lowercase().contains("goal") || s.slot.to_lowercase().contains("explosion") {
                                            s.owned_id
                                        } else {
                                            None
                                        }
                                    });
                                    if let Ok((payload, mem_sz)) = emit_explosion_previewer_set_product_bytecode(
                                        owned_ge,
                                        target_ge,
                                        orig_disk_sz,
                                    ) {
                                        decomp1[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                        decomp1[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                                    }
                                }
                            }
                        }

                        if let Ok(mut recomp1) = crate::upk::compression::compress_chunk(&decomp1) {
                            let orig_c1_sz = c1_comp_size as usize;
                            if recomp1.len() <= orig_c1_sz {
                                recomp1.resize(orig_c1_sz, 0);
                                file_bytes[c1_start..c1_start + orig_c1_sz].copy_from_slice(&recomp1);
                            } else {
                                return Err(TagameSwapError::Msg(format!(
                                    "Recompressed Chunk 1 ({} bytes) exceeds allocation ({} bytes)",
                                    recomp1.len(),
                                    orig_c1_sz
                                )));
                            }
                        }
                    }
                }
            }
        }





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
        if !should_apply_palette && backup_path.is_file() {
            let _ = fs::copy(&backup_path, &tagame_path);
        }
    }

    // Apply paint overrides across all item types (wheels, boosts, decals, bodies / custom hex)
    for s in swaps {
        let pkg_opt = s.package_name.as_deref();
        let custom_hex_str = s.custom_paint_hex.as_deref();
        let has_custom = custom_hex_str.map(|h| !h.trim().is_empty()).unwrap_or(false);
        let pid = s.paint_id.unwrap_or(0);

        if has_custom || pid > 0 {
            if let Some(pkg) = pkg_opt {
                let _ = apply_item_paint_modification(cooked_dir, pkg, pid, custom_hex_str, keys_map_json);
            }
            let is_body = s.slot.to_lowercase().contains("body")
                || pkg_opt.map_or(false, |p| p.to_lowercase().starts_with("body_"));
            if is_body {
                let _ = apply_item_paint_modification(cooked_dir, "body_grain_SF", pid, custom_hex_str, keys_map_json);
            }
        } else if let Some(pkg) = pkg_opt {
            let mut restore_names = vec![pkg];
            let is_body = s.slot.to_lowercase().contains("body")
                || pkg.to_lowercase().starts_with("body_");
            if is_body {
                restore_names.push("body_grain_SF");
            }
            for restore_name in restore_names {
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
    fn test_emit_car_set_loadout_bytecode() {
        let rules = [
            SlotSwapRule { slot_idx: 0, owned_id: None, target_id: 4284 },
            SlotSwapRule { slot_idx: 2, owned_id: None, target_id: 30 },
            SlotSwapRule { slot_idx: 3, owned_id: None, target_id: 32 },
        ];
        let (bc, mem_sz) = emit_car_set_loadout_bytecode(&rules, 186, None, None, None, None).unwrap();
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
        let (bc, mem_sz) = emit_car_set_loadout_bytecode(&rules, 300, None, None, None, None).unwrap();
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

        let status = apply_tagame_modifications(&temp_dir, &swaps, keys_txt, keys_map_json, None).expect("apply_tagame_modifications must succeed");
        assert!(status.applied);
        assert_eq!(status.active_swaps.len(), 4);

        let modified_bytes = fs::read(&tagame_test).expect("read modified TAGame.upk");
        assert!(modified_bytes.len() > 1024 * 1024);
    }
}
