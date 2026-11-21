/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use crate::upk::crypto;
pub use super::legacy_swapper::{
    paint_label, paint_slugs, resolve_package_path, read_package_guid,
    read_engine_versions, dump_engine_info, find_engine_dependency_linker_indices,
    read_engine_import_guid_in_header, patch_engine_versions_in_prefix,
    read_prefix_engine_versions, header_references_stale_engine, patch_engine_import_guid,
    Item, SwapOptions,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;
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
    pub const EX_STRING_CONST: u8 = 0x1F;
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
    matches!(slot_idx, 0 | 1 | 2 | 3 | 4 | 5 | 7 | 12 | 13 | 14 | 15)
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

    let mut seen = std::collections::HashSet::new();
    let mut unique_rules = Vec::new();
    for r in slot_overrides.iter().rev() {
        if is_car_loadout_slot(r.slot_idx) && seen.insert((r.slot_idx, r.owned_id)) {
            unique_rules.push(r.clone());
        }
    }
    unique_rules.reverse();

    let vanilla_tail_len = 97;
    let available_space = max_disk_size.saturating_sub(vanilla_tail_len);

    for rule in unique_rules {
        let index_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
        let (target_disk_len, target_mem_len) = if rule.target_id <= 255 && rule.target_id >= 0 {
            (2, 2u32)
        } else {
            (5, 5u32)
        };
        let assign_disk_len = 18 + index_disk_len + target_disk_len;

        if let Some(owned_id) = rule.owned_id {
            let cond_disk_len = 27 + index_disk_len;
            let total_rule_disk = cond_disk_len + assign_disk_len;
            if bc.len() + total_rule_disk > available_space {
                break;
            }

            // 1. Independent condition: if (Data.Products[rule.slot_idx] == owned_id)
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
            let cond_index_mem = if rule.slot_idx == 0 {
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
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&data_var_id.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            let cond_mem = 39 + cond_index_mem;
            mem_sz += cond_mem;

            // 2. Data.Products[rule.slot_idx] = rule.target_id;
            bc.push(opcodes::EX_LET);
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
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&data_var_id.to_le_bytes());
            if rule.target_id <= 255 && rule.target_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.target_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&rule.target_id.to_le_bytes());
            }

            let assign_mem = 30 + assign_index_mem + target_mem_len;
            mem_sz += assign_mem;

            let jump_target = mem_sz as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
        } else {
            if bc.len() + assign_disk_len > available_space {
                break;
            }
            bc.push(opcodes::EX_LET);
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
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&data_var_id.to_le_bytes());
            if rule.target_id <= 255 && rule.target_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.target_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&rule.target_id.to_le_bytes());
            }

            let assign_mem = 30 + assign_index_mem + target_mem_len;
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
    let nop_count = available_space.saturating_sub(bc.len());
    bc.resize(available_space, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    bc.extend_from_slice(vanilla_body);
    // Exact vanilla execution body UScript memory size in 64-bit RocketLeague.exe:
    // Stmt 1 (EX_LET_BOOL): 11
    // Stmt 2 (EX_DELEGATE_PROPERTY): 40
    // Stmt 3 (EX_DELEGATE_PROPERTY): 40
    // Stmt 4 (EX_DYN_ARRAY_ELEMENT): 20 (64-bit object pointer expansion)
    mem_sz += 137;

    if bc.len() < max_disk_size {
        let extra = max_disk_size - bc.len();
        bc.resize(max_disk_size, opcodes::EX_NOTHING);
        mem_sz += extra as u32;
    }

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
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&new_loadout_id.to_le_bytes());
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&in_loadout_id.to_le_bytes());
    mem_sz += 19;
    let mut seen = std::collections::HashSet::new();
    let mut unique_rules = Vec::new();
    for r in slot_overrides.iter().rev() {
        if is_car_loadout_slot(r.slot_idx) && seen.insert((r.slot_idx, r.owned_id)) {
            unique_rules.push(r.clone());
        }
    }
    unique_rules.reverse();

    // 2. Slot override rules on NewLoadout
    // ForceSetLoadout tail call is 19 bytes disk
    let tail_disk_len = 19;
    let available_space = max_disk_size.saturating_sub(tail_disk_len);

    for rule in unique_rules {
        let index_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
        let (target_disk_len, target_mem_len) = if rule.target_id <= 255 && rule.target_id >= 0 {
            (2, 2u32)
        } else {
            (5, 5u32)
        };
        let assign_disk_len = 18 + index_disk_len + target_disk_len;

        if let Some(owned_id) = rule.owned_id {
            let cond_disk_len = 27 + index_disk_len;
            let total_rule_disk = cond_disk_len + assign_disk_len;
            if bc.len() + total_rule_disk > available_space {
                break;
            }

            // 1. Independent condition: if (NewLoadout.Products[rule.slot_idx] == owned_id)
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
            let cond_index_mem = if rule.slot_idx == 0 {
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
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&new_loadout_id.to_le_bytes());
            bc.push(opcodes::EX_INT_CONST);
            bc.extend_from_slice(&owned_id.to_le_bytes());
            bc.push(opcodes::EX_END_FUNCTION_PARMS);

            let cond_mem = 39 + cond_index_mem;
            mem_sz += cond_mem;

            // 2. NewLoadout.Products[rule.slot_idx] = rule.target_id;
            bc.push(opcodes::EX_LET);
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
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&new_loadout_id.to_le_bytes());
            if rule.target_id <= 255 && rule.target_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.target_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&rule.target_id.to_le_bytes());
            }

            let assign_mem = 30 + assign_index_mem + target_mem_len;
            mem_sz += assign_mem;

            let jump_target = mem_sz as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
        } else {
            if bc.len() + assign_disk_len > available_space {
                break;
            }
            bc.push(opcodes::EX_LET);
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
            bc.extend_from_slice(&[0x00, 0x00]);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&new_loadout_id.to_le_bytes());
            if rule.target_id <= 255 && rule.target_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.target_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&rule.target_id.to_le_bytes());
            }

            let assign_mem = 30 + assign_index_mem + target_mem_len;
            mem_sz += assign_mem;
        }
    }

    // 3. Stmt: ForceSetLoadout(NewLoadout, false); return;
    bc.push(opcodes::EX_VIRTUAL_FUNCTION);
    bc.extend_from_slice(&force_set_loadout_name_idx.to_le_bytes());
    bc.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&new_loadout_id.to_le_bytes());
    bc.push(opcodes::EX_FALSE_CONST);
    bc.push(opcodes::EX_END_FUNCTION_PARMS);
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_NOTHING);
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 23;

    // 4. Pad with EX_NOTHING up to max_disk_size
    let nop_count = max_disk_size.saturating_sub(bc.len());
    bc.resize(max_disk_size, opcodes::EX_NOTHING);
    mem_sz += nop_count as u32;

    Ok((bc, mem_sz))
}

pub fn emit_car_preview_change_car_part_bytecode(
    slot_overrides: &[SlotSwapRule],
    max_disk_size: usize,
    product_id_opt: Option<i32>,
    slot_index_opt: Option<i32>,
    loadout_id_opt: Option<i32>,
    products_id_opt: Option<i32>,
    loadout_data_id_opt: Option<i32>,
    force_set_loadout_name_idx_opt: Option<i32>,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::new();
    let mut mem_sz: u32 = 0;

    let product_id_var = product_id_opt.unwrap_or(18109);
    let slot_index_var = slot_index_opt.unwrap_or(18110);
    let loadout_id = loadout_id_opt.unwrap_or(18067);
    let products_id = products_id_opt.unwrap_or(1871);
    let loadout_data_id = loadout_data_id_opt.unwrap_or(2364);
    let force_set_idx = force_set_loadout_name_idx_opt.unwrap_or(20264);

    let mut seen = std::collections::HashSet::new();
    let mut unique_rules = Vec::new();
    for r in slot_overrides.iter().rev() {
        if is_car_loadout_slot(r.slot_idx) && seen.insert((r.slot_idx, r.owned_id)) {
            unique_rules.push(r.clone());
        }
    }
    unique_rules.reverse();

    // Tail size:
    // Stmt 1 (Loadout.Products[SlotIndex] = ProductID): 1+1+5+12+5+5 = 29 bytes disk, 47 mem
    // Stmt 2 (ForceSetLoadout(Loadout, true)): 9+5+1+1 = 16 bytes disk, 27 mem
    // Stmt 3 (return true): 1+1+1 = 3 bytes disk, 3 mem
    // Total tail disk = 48 bytes
    let tail_disk_len = 48;
    let available_space = max_disk_size.saturating_sub(tail_disk_len);

    for rule in unique_rules {
        let (target_disk_len, target_mem_len) = if rule.target_id <= 255 && rule.target_id >= 0 {
            (2, 2u32)
        } else {
            (5, 5u32)
        };

        if let Some(owned_id) = rule.owned_id {
            let (owned_disk_len, owned_mem_len) = if owned_id <= 255 && owned_id >= 0 {
                (2, 2u32)
            } else {
                (5, 5u32)
            };
            let cond_disk_len = 8 + owned_disk_len;
            let assign_disk_len = 6 + target_disk_len;
            let total_rule_disk = cond_disk_len + assign_disk_len;
            if bc.len() + total_rule_disk > available_space {
                break;
            }

            // if (ProductID == owned_id) ProductID = target_id;
            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&product_id_var.to_le_bytes());
            if owned_id <= 255 && owned_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(owned_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&owned_id.to_le_bytes());
            }
            bc.push(opcodes::EX_END_FUNCTION_PARMS);
            let cond_mem = 1 + 9 + owned_mem_len + 1;

            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&product_id_var.to_le_bytes());
            if rule.target_id <= 255 && rule.target_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.target_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&rule.target_id.to_le_bytes());
            }
            let assign_mem = 1 + 9 + target_mem_len;

            let total_rule_mem = 3 + cond_mem + assign_mem;
            mem_sz += total_rule_mem;
            let jump_target = mem_sz as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
        } else {
            // Wildcard slot match: if (SlotIndex == rule.slot_idx) ProductID = target_id;
            let idx_disk_len = if rule.slot_idx == 0 { 1 } else { 2 };
            let idx_mem_len = if rule.slot_idx == 0 { 1u32 } else { 2u32 };
            let cond_disk_len = 8 + idx_disk_len;
            let assign_disk_len = 6 + target_disk_len;
            let total_rule_disk = cond_disk_len + assign_disk_len;
            if bc.len() + total_rule_disk > available_space {
                break;
            }

            bc.push(opcodes::EX_JUMP_IF_NOT);
            let jump_pos = bc.len();
            bc.extend_from_slice(&[0x00, 0x00]);

            bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&slot_index_var.to_le_bytes());
            if rule.slot_idx == 0 {
                bc.push(opcodes::EX_INT_ZERO);
            } else {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.slot_idx);
            }
            bc.push(opcodes::EX_END_FUNCTION_PARMS);
            let cond_mem = 1 + 9 + idx_mem_len + 1;

            bc.push(opcodes::EX_LET);
            bc.push(opcodes::EX_LOCAL_VARIABLE);
            bc.extend_from_slice(&product_id_var.to_le_bytes());
            if rule.target_id <= 255 && rule.target_id >= 0 {
                bc.push(opcodes::EX_INT_CONST_BYTE);
                bc.push(rule.target_id as u8);
            } else {
                bc.push(opcodes::EX_INT_CONST);
                bc.extend_from_slice(&rule.target_id.to_le_bytes());
            }
            let assign_mem = 1 + 9 + target_mem_len;

            let total_rule_mem = 3 + cond_mem + assign_mem;
            mem_sz += total_rule_mem;
            let jump_target = mem_sz as u16;
            bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
        }
    }

    // Stmt 1: Loadout.Products[SlotIndex] = ProductID; (29 bytes disk, 47 mem)
    bc.push(opcodes::EX_LET);
    bc.push(opcodes::EX_DYN_ARRAY_ELEMENT);
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&slot_index_var.to_le_bytes());
    bc.push(opcodes::EX_STRUCT_MEMBER);
    bc.extend_from_slice(&products_id.to_le_bytes());
    bc.extend_from_slice(&loadout_data_id.to_le_bytes());
    bc.extend_from_slice(&[0x00, 0x00]);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&loadout_id.to_le_bytes());
    bc.push(opcodes::EX_LOCAL_VARIABLE);
    bc.extend_from_slice(&product_id_var.to_le_bytes());
    mem_sz += 47;

    // Stmt 2: ForceSetLoadout(Loadout, true); (16 bytes disk, 27 mem)
    bc.push(opcodes::EX_VIRTUAL_FUNCTION);
    bc.extend_from_slice(&force_set_idx.to_le_bytes());
    bc.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    bc.push(opcodes::EX_INSTANCE_VARIABLE);
    bc.extend_from_slice(&loadout_id.to_le_bytes());
    bc.push(opcodes::EX_TRUE_CONST);
    bc.push(opcodes::EX_END_FUNCTION_PARMS);
    mem_sz += 27;

    // Stmt 3: return true; (3 bytes disk, 3 mem)
    bc.push(opcodes::EX_RETURN);
    bc.push(opcodes::EX_TRUE_CONST);
    bc.push(opcodes::EX_END_OF_SCRIPT);
    mem_sz += 3;

    // Pad with EX_NOTHING up to max_disk_size
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

#[allow(dead_code)]
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
    mem_sz += 3 + 9;

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
        bc.extend_from_slice(&[0x00, 0x00]);
        bc.push(opcodes::EX_INSTANCE_VARIABLE);
        bc.extend_from_slice(&15655i32.to_le_bytes());
        bc.push(opcodes::EX_FLOAT_CONST);
        bc.extend_from_slice(val_bytes);

        mem_sz += 1 + 19 + 9 + 5;
    }

    let jump_target = mem_sz as u16;
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

pub const EXPLOSION_PREVIEWER_SET_PRODUCT_TAIL: [u8; 196] = [
    0x1c, 0xb8, 0x5a, 0x00, 0x00, 0x16, 0x07, 0x81, 0x01, 0x82, 0x97, 0x2b, 0xbe, 0x5a, 0x00, 0x00,
    0x25, 0x16, 0x18, 0x79, 0x00, 0xcf, 0x5e, 0x19, 0x01, 0x19, 0x00, 0x12, 0x00, 0x20, 0x16, 0x64,
    0x00, 0x00, 0x09, 0x00, 0xe7, 0x63, 0x00, 0x00, 0x00, 0x02, 0xe7, 0x63, 0x00, 0x00, 0x13, 0x00,
    0x7a, 0x61, 0x00, 0x00, 0x00, 0x1c, 0x7c, 0x61, 0x00, 0x00, 0x2b, 0xbe, 0x5a, 0x00, 0x00, 0x16,
    0x09, 0x00, 0x38, 0x9c, 0x00, 0x00, 0x00, 0x01, 0x38, 0x9c, 0x00, 0x00, 0x5e, 0x12, 0x00, 0x20,
    0x16, 0x64, 0x00, 0x00, 0x09, 0x00, 0xd3, 0x63, 0x00, 0x00, 0x00, 0x02, 0xd3, 0x63, 0x00, 0x00,
    0x16, 0x16, 0x52, 0x5e, 0x19, 0x00, 0x01, 0xb2, 0x5a, 0x00, 0x00, 0x09, 0x00, 0xf7, 0x3e, 0x00,
    0x00, 0x00, 0x01, 0xf7, 0x3e, 0x00, 0x00, 0x49, 0xf2, 0x5f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x5e, 0x19, 0x00, 0x01, 0xb2, 0x5a, 0x00, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c,
    0x07, 0x3f, 0x00, 0x00, 0x57, 0x28, 0x2b, 0xbc, 0x5a, 0x00, 0x00, 0x00, 0x00, 0x2b, 0xbe, 0x5a,
    0x00, 0x00, 0x16, 0x2b, 0xbc, 0x5a, 0x00, 0x00, 0x16, 0x06, 0xa2, 0x01, 0x5e, 0x19, 0x00, 0x01,
    0xb2, 0x5a, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x52, 0x3f, 0x00, 0x00,
    0x16, 0x04, 0x0b, 0x4c,
];

pub fn emit_explosion_previewer_set_product_bytecode(
    owned_id_opt: Option<i32>,
    target_id: i32,
    max_disk_size: usize,
) -> Result<(Vec<u8>, u32), TagameSwapError> {
    let mut bc = Vec::with_capacity(max_disk_size);
    let product_id_var = 23230i32; // ProductID local parameter

    if let Some(owned) = owned_id_opt {
        // if (ProductID == owned) ProductID = target;
        bc.push(opcodes::EX_JUMP_IF_NOT);
        let jump_pos = bc.len();
        bc.extend_from_slice(&[0x00, 0x00]);

        bc.push(opcodes::EX_EQUAL_EQUAL_INT_INT);
        bc.push(opcodes::EX_LOCAL_VARIABLE);
        bc.extend_from_slice(&product_id_var.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&owned.to_le_bytes());
        bc.push(opcodes::EX_END_FUNCTION_PARMS);

        bc.push(opcodes::EX_LET);
        bc.push(opcodes::EX_LOCAL_VARIABLE);
        bc.extend_from_slice(&product_id_var.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&target_id.to_le_bytes());
        let jump_target = 34u16;
        bc[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());
    } else {
        // ProductID = target;
        bc.push(opcodes::EX_LET);
        bc.push(opcodes::EX_LOCAL_VARIABLE);
        bc.extend_from_slice(&product_id_var.to_le_bytes());
        bc.push(opcodes::EX_INT_CONST);
        bc.extend_from_slice(&target_id.to_le_bytes());
    }

    // Pad prefix with EX_NOTHING up to byte 77
    bc.resize(77, opcodes::EX_NOTHING);

    // Append vanilla tail (offset 77 to 273: calls UpdateExplosion & LoadProducts)
    bc.extend_from_slice(&EXPLOSION_PREVIEWER_SET_PRODUCT_TAIL);

    if bc.len() < max_disk_size {
        bc.resize(max_disk_size, opcodes::EX_NOTHING);
    }

    Ok((bc, 421))
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
            let jump_target = (mem_sz + total_rule_mem) as u16;
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

pub fn emit_pri_get_ping_bytecode(min_ms: u32, max_ms: u32, orig_disk_sz: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    let min_f = (min_ms.min(max_ms)) as f32;
    let max_f = (min_ms.max(max_ms)) as f32;

    // EX_RETURN: 0x04
    payload.push(opcodes::EX_RETURN);
    // EX_PRIMITIVE_CAST (to int): 0x38, 0x44
    payload.extend_from_slice(&[0x38, 0x44]);
    // EX_FINAL_FUNCTION: 0x1C
    payload.push(opcodes::EX_FINAL_FUNCTION);
    // RandRange import index in TAGame.upk: -1885
    payload.extend_from_slice(&(-1885i32).to_le_bytes());
    // EX_FLOAT_CONST: 0x1E, min_f
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&min_f.to_le_bytes());
    // EX_FLOAT_CONST: 0x1E, max_f
    payload.push(opcodes::EX_FLOAT_CONST);
    payload.extend_from_slice(&max_f.to_le_bytes());
    // EX_END_FUNCTION_PARMS: 0x16
    payload.push(opcodes::EX_END_FUNCTION_PARMS);
    // EX_RETURN: 0x04, EX_INT_ZERO: 0x25
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_INT_ZERO);
    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);
    payload
}

#[allow(dead_code)]
pub fn emit_noop_void_function_bytecode(orig_disk_sz: usize) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_RETURN); // 0x04
    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING); // 0x0B
    }
    payload.push(opcodes::EX_END_OF_SCRIPT); // 0x4C
    (payload, orig_disk_sz as u32)
}

pub fn emit_noop_bool_function_bytecode(return_val: bool, orig_disk_sz: usize) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    payload.push(opcodes::EX_RETURN); // 0x04
    payload.push(if return_val { opcodes::EX_TRUE_CONST } else { opcodes::EX_FALSE_CONST }); // 0x27 or 0x28
    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING); // 0x0B
    }
    payload.push(opcodes::EX_END_OF_SCRIPT); // 0x4C (ALWAYS AT THE VERY LAST BYTE!)
    (payload, orig_disk_sz as u32)
}

#[allow(dead_code)]
pub fn emit_modify_loadout_passthrough_bytecode(
    new_product_ids_id: i32,
    loadout_product_ids_id: i32,
    return_val_id: i32,
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    // 0. EX_NOTHING
    payload.push(opcodes::EX_NOTHING); // 0x0B

    // 1. NewProductIDs = LoadoutProductIDs;
    payload.push(opcodes::EX_LET); // 0x0F
    payload.push(opcodes::EX_INSTANCE_VARIABLE); // 0x2B
    payload.extend_from_slice(&new_product_ids_id.to_le_bytes());
    payload.push(opcodes::EX_LOCAL_VARIABLE); // 0x46
    payload.extend_from_slice(&loadout_product_ids_id.to_le_bytes());

    // 2. return NewProductIDs;
    payload.push(opcodes::EX_RETURN); // 0x04
    payload.push(opcodes::EX_INSTANCE_VARIABLE); // 0x2B
    payload.extend_from_slice(&new_product_ids_id.to_le_bytes());

    // 3. return ReturnValue;
    payload.push(opcodes::EX_RETURN); // 0x04
    payload.push(opcodes::EX_RETURN_VALUE); // 0x3A
    payload.extend_from_slice(&return_val_id.to_le_bytes());

    // Active memory size calculation:
    // EX_NOTHING (1) + EX_LET (1) + EX_INSTANCE_VARIABLE (9) + EX_LOCAL_VARIABLE (9)
    // + EX_RETURN (1) + EX_INSTANCE_VARIABLE (9) + EX_RETURN (1) + EX_RETURN_VALUE (9)
    // = 40 bytes.
    let active_mem = 40u32;
    let nop_count = orig_disk_sz.saturating_sub(payload.len() + 1);
    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT); // 0x4C (ALWAYS AT THE VERY LAST BYTE!)
    (payload, active_mem + nop_count as u32 + 1)
}

pub fn emit_pick_fx_actor_bytecode(
    painted_fx_idx: i32,
    fx_idx: i32,
    return_val_idx: i32,
    orig_disk_sz: usize,
) -> (Vec<u8>, u32) {
    let mut payload = Vec::with_capacity(orig_disk_sz);
    // if (PaintedFXActor != None) return PaintedFXActor;
    payload.push(opcodes::EX_JUMP_IF_NOT);
    let jump_pos = payload.len();
    payload.extend_from_slice(&[0x00, 0x00]);

    // EX_NOT_EQUAL_OBJECT_OBJECT: 0xCE
    payload.push(0xCE);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&painted_fx_idx.to_le_bytes());
    payload.push(0x2A); // None
    payload.push(opcodes::EX_END_FUNCTION_PARMS);

    // return PaintedFXActor;
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&painted_fx_idx.to_le_bytes());

    let jump_target = (3 + (1 + 9 + 1 + 1) + (1 + 9)) as u16; // 25 bytes in 64-bit UScript memory
    payload[jump_pos..jump_pos + 2].copy_from_slice(&jump_target.to_le_bytes());

    // return FXActor;
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_INSTANCE_VARIABLE);
    payload.extend_from_slice(&fx_idx.to_le_bytes());

    // return ReturnValue;
    payload.push(opcodes::EX_RETURN);
    payload.push(opcodes::EX_RETURN_VALUE);
    payload.extend_from_slice(&return_val_idx.to_le_bytes());

    let active_mem = (1 + 9 + 1 + 1) + (1 + 9) + (1 + 9) + (1 + 9);
    let nop_count = orig_disk_sz.saturating_sub(payload.len() + 1);
    while payload.len() + 1 < orig_disk_sz {
        payload.push(opcodes::EX_NOTHING);
    }
    payload.push(opcodes::EX_END_OF_SCRIPT);
    (payload, active_mem as u32 + nop_count as u32 + 1)
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
    // If the game was updated, Engine.upk or RocketLeague.exe will have changed, or TAGame.upk was updated.
    // 1. Check if backup references a stale Engine GUID
    let backup_stale_engine = if backup_path.is_file() {
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
    let backup_differs_from_live = if backup_path.is_file() {
        match (fs::metadata(&tagame_path), fs::metadata(&backup_path)) {
            (Ok(m_live), Ok(m_bak)) => {
                if m_live.len() != m_bak.len() {
                    true
                } else {
                    fs::read(&tagame_path).ok() != fs::read(&backup_path).ok()
                }
            }
            _ => false,
        }
    } else {
        false
    };

    let backup_is_stale = backup_stale_engine || (live_is_vanilla && backup_differs_from_live);

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

    let ping_cfg = crate::udp_proxy::get_ping_spoof();
    let ping_active = ping_cfg.as_ref().map(|p| p.enabled).unwrap_or(false);

    // Swaps are handled in-memory / via broker bridge; TAGame bytecode is patched when ping spoof is active
    let should_patch = ping_active;

    if should_patch {
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

        let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
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

            // Only add a slot override rule if an explicit donor was specified and donor != target.
            // Slot 0 (Body) is never marked unconditional (owned_id: None), preventing other cars from being overwritten.
            if let Some(oid) = s.owned_id {
                if oid > 0 && oid != pid {
                    slot_overrides.push(SlotSwapRule {
                        slot_idx,
                        owned_id: Some(oid),
                        target_id: pid,
                    });
                }
            }
        }

        let upk_modified = false;

        if upk_modified {
            fs::write(&tagame_path, &file_bytes)?;
        }
    } else {
        if !should_apply_palette && !ping_active && backup_path.is_file() {
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

pub fn apply_tagame_ping_spoof(
    cooked_dir: &Path,
    min_ms: u32,
    max_ms: u32,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<TagameSwapperStatus, TagameSwapError> {
    crate::udp_proxy::set_ping_spoof(Some(crate::psynet::PingSpoofPayload {
        enabled: true,
        min_ms,
        max_ms,
    }));
    apply_tagame_modifications(cooked_dir, &[], keys_txt, keys_map_json, None)
}

pub fn restore_tagame_ping_spoof(
    cooked_dir: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<TagameSwapperStatus, TagameSwapError> {
    crate::udp_proxy::set_ping_spoof(Some(crate::psynet::PingSpoofPayload {
        enabled: false,
        min_ms: 16,
        max_ms: 24,
    }));
    apply_tagame_modifications(cooked_dir, &[], keys_txt, keys_map_json, None)
}

/// Check and ensure ProjectX.upk has all URL validation functions patched to allow any URL:
/// - PsyNetUrl_X::IsAllowedURL -> return true;
/// - PsyNetUrl_X::ValidateURL -> return InURL; (FString return type preserved to avoid C++ segfault)
/// - URLValidator_X::IsURLValid -> return true;
/// - URLValidator_X::IsSchemeValid -> return true;
/// - URLValidator_X::AreCharsUrlSafe -> return true;
/// - URLValidator_X::IsCharUrlSafe -> return true;
/// - URLValidator_X::IsPathValid -> return true;
/// - URLValidator_X::IsQueryValid -> return true;
/// - URLValidator_X::IsIPv4AddressValid -> return true;
/// - URLValidator_X::IsIPv4AndPortValid -> return true;
/// - URLValidator_X::HostLooksLikeIPv4 -> return true;
pub fn is_projectx_file_patched(file_bytes: &[u8]) -> bool {
    if file_bytes.len() < 0x20 {
        return false;
    }
    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap_or([0; 4])) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes.get(p..p + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let prefix_sz = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize });
    p = prefix_sz;
    let _pkg_flags = u32::from_le_bytes(file_bytes.get(p..p + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let _name_count = i32::from_le_bytes(file_bytes.get(p + 4..p + 8).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let name_offset = u32::from_le_bytes(file_bytes.get(p + 8..p + 12).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize;
    let p_sum = prefix_sz + 12;
    let _export_count = i32::from_le_bytes(file_bytes.get(p_sum..p_sum + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize;
    let depends_offset = i32::from_le_bytes(file_bytes.get(p_sum + 16..p_sum + 20).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize;
    let enc_size = (total_header_size.saturating_sub(name_offset) + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end <= file_bytes.len() && depends_offset >= name_offset {
        let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
        let chunks_rel = depends_offset - name_offset;
        if chunks_rel + 4 <= plain_header.len() {
            let num_chunks = u32::from_le_bytes(plain_header[chunks_rel..chunks_rel + 4].try_into().unwrap_or([0; 4])) as usize;
            if num_chunks >= 2 && chunks_rel + 4 + 72 <= plain_header.len() {
                let c1_p = chunks_rel + 4 + 36;
                let c1_c_off = u64::from_le_bytes(plain_header[c1_p + 12..c1_p + 20].try_into().unwrap_or([0; 8])) as usize;
                let c1_c_sz = i32::from_le_bytes(plain_header[c1_p + 20..c1_p + 24].try_into().unwrap_or([0; 4])) as usize;
                if c1_c_off + c1_c_sz <= file_bytes.len() {
                    if let Ok(c1_decomp) = crate::upk::compression::decompress_chunk(&file_bytes[c1_c_off..c1_c_off + c1_c_sz]) {
                        return c1_decomp.windows(26).any(|w| w == b"http://127.0.0.1:15759/rpc")
                            && c1_decomp.windows(21).any(|w| w == b"ws://127.0.0.1:15759/");
                    }
                }
            }
        }
    }
    false
}

pub fn is_tagame_file_crash_patched(file_bytes: &[u8]) -> bool {
    if file_bytes.len() < 0x20 {
        return false;
    }
    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap_or([0; 4])) as usize;
    let mut p = 12;
    let flen = i32::from_le_bytes(file_bytes.get(p..p + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let prefix_sz = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize });
    p = prefix_sz;
    let _pkg_flags = u32::from_le_bytes(file_bytes.get(p..p + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let _name_count = i32::from_le_bytes(file_bytes.get(p + 4..p + 8).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let name_offset = u32::from_le_bytes(file_bytes.get(p + 8..p + 12).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize;
    let p_sum = prefix_sz + 12;
    let _export_count = i32::from_le_bytes(file_bytes.get(p_sum..p_sum + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize;
    let depends_offset = i32::from_le_bytes(file_bytes.get(p_sum + 16..p_sum + 20).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize;
    let enc_size = (total_header_size.saturating_sub(name_offset) + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end <= file_bytes.len() && depends_offset >= name_offset {
        let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
        let chunks_rel = depends_offset - name_offset;
        if chunks_rel + 4 <= plain_header.len() {
            let num_chunks = u32::from_le_bytes(plain_header[chunks_rel..chunks_rel + 4].try_into().unwrap_or([0; 4])) as usize;
            if num_chunks > 78 && chunks_rel + 4 + (79 * 36) <= plain_header.len() {
                let c78_p = chunks_rel + 4 + (78 * 36);
                let c78_c_off = u64::from_le_bytes(plain_header[c78_p + 12..c78_p + 20].try_into().unwrap_or([0; 8])) as usize;
                let c78_c_sz = i32::from_le_bytes(plain_header[c78_p + 20..c78_p + 24].try_into().unwrap_or([0; 4])) as usize;
                if c78_c_off + c78_c_sz <= file_bytes.len() {
                    if let Ok(c78_decomp) = crate::upk::compression::decompress_chunk(&file_bytes[c78_c_off..c78_c_off + c78_c_sz]) {
                        const SHOULD_BACKFILL_PATCH: [u8; 6] = [0x53, 0x2e, 0x00, 0x00, 0xe4, 0x7a];
                        return c78_decomp.windows(6).any(|w| w == SHOULD_BACKFILL_PATCH);
                    }
                }
            }
        }
    }
    false
}

/// True when TAGame contains the transplanted loadout hook (its rewritten broker URL).
pub fn tagame_has_rlim_patch(file_bytes: &[u8]) -> bool {
    if file_bytes.len() < 0x20 {
        return false;
    }
    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap_or([0; 4])) as usize;
    let flen = i32::from_le_bytes(file_bytes.get(12..16).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]));
    let prefix_sz = 16 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize });
    let name_offset = u32::from_le_bytes(
        file_bytes.get(prefix_sz + 8..prefix_sz + 12).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])
    ) as usize;
    let p_sum = prefix_sz + 12;
    let depends_offset = i32::from_le_bytes(
        file_bytes.get(p_sum + 16..p_sum + 20).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])
    ) as usize;
    let enc_size = (total_header_size.saturating_sub(name_offset) + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end > file_bytes.len() || depends_offset < name_offset {
        return false;
    }
    let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
    let chunks_rel = depends_offset - name_offset;
    if chunks_rel + 4 > plain_header.len() {
        return false;
    }
    let num_chunks = u32::from_le_bytes(plain_header[chunks_rel..chunks_rel + 4].try_into().unwrap_or([0; 4])) as usize;
    for i in 0..num_chunks {
        let pos = chunks_rel + 4 + i * 36;
        if pos + 24 > plain_header.len() {
            break;
        }
        let c_off = u64::from_le_bytes(plain_header[pos + 12..pos + 20].try_into().unwrap_or([0; 8])) as usize;
        let c_sz = i32::from_le_bytes(plain_header[pos + 20..pos + 24].try_into().unwrap_or([0; 4])) as usize;
        if c_sz == 0 || c_off.checked_add(c_sz).map_or(true, |e| e > file_bytes.len()) {
            continue;
        }
        if let Ok(decomp) = crate::upk::compression::decompress_chunk(&file_bytes[c_off..c_off + c_sz]) {
            if decomp.windows(11).any(|w| w == b"15759/r/lo?") {
                return true;
            }
        }
    }
    false
}

/// Fetch the small bytecode patch payload described by /v2/rl/build and apply
/// it to the live TAGame.upk in place. Returns true when a patch was applied.
pub async fn patch_tagame_from_build(
    app: &tauri::AppHandle,
    tg_path: &Path,
) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let build_url = "https://api.velocityrl.tech/v2/rl/build";
    let build: serde_json::Value = client
        .get(build_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let Some(patch) = build.get("tagame_patch") else {
        return Err("build response has no tagame_patch".into());
    };
    let sha = patch.get("sha256").and_then(|v| v.as_str()).unwrap_or_default();
    let url_path = patch.get("patch_url").and_then(|v| v.as_str()).unwrap_or_default();
    if sha.len() != 64 || url_path.is_empty() {
        return Err("invalid tagame_patch metadata".into());
    }
    let url = format!("https://api.velocityrl.tech{url_path}");

    let bytes = client.get(&url).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(&bytes);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != sha {
        return Err(format!("patch sha mismatch: {got} != {sha}"));
    }

    if let Ok(live) = fs::read(tg_path) {
        if tagame_has_rlim_patch(&live) && live.windows(22).any(|w| w == b"15759/rlim/avatar.png?") {
            crate::applog::event(&format!(
                "upk: TAGame already patched from /build (sha {sha})"
            ));
            return Ok(false);
        }
    }

    let backup = tg_path.with_file_name(TAGAME_BACKUP_NAME);
    if !backup.is_file() {
        let _ = fs::copy(tg_path, &backup);
    }
    let applied = patch_tagame_in_place(tg_path, &backup, &bytes)?;
    if applied == 0 {
        return Ok(false);
    }
    let _ = app;
    crate::applog::event(&format!(
        "upk: applied {applied} TAGame patch records from /build ({} bytes, sha {sha})",
        bytes.len()
    ));
    Ok(true)
}

pub fn ensure_projectx_patched(cooked_dir: &Path) -> Result<bool, String> {
    let px_path = if cooked_dir.join("ProjectX.upk").is_file() {
        cooked_dir.join("ProjectX.upk")
    } else if cooked_dir.join("TAGame/CookedPCConsole/ProjectX.upk").is_file() {
        cooked_dir.join("TAGame/CookedPCConsole/ProjectX.upk")
    } else {
        return Ok(false);
    };

    let bak_path = px_path.with_extension("upk.bak");

    // If backup does not exist yet, create backup of current file before any patching
    if !bak_path.is_file() {
        if let Ok(bytes) = fs::read(&px_path) {
            let _ = fs::write(&bak_path, &bytes);
        }
    }

    // Fast check: if current file already contains both RPC and WS broker patches, nothing to do
    if let Ok(cur_px_bytes) = fs::read(&px_path) {
        if is_projectx_file_patched(&cur_px_bytes) {
            return Ok(false); // Already cleanly patched with both RPC and WS broker
        }
    }

    // Always patch from clean backup if available, or current file
    let mut file_bytes = if bak_path.is_file() {
        fs::read(&bak_path).unwrap_or_else(|_| fs::read(&px_path).unwrap_or_default())
    } else {
        fs::read(&px_path).unwrap_or_default()
    };

    if file_bytes.len() < 0x20 {
        return Err("ProjectX.upk too small".into());
    }

    let total_header_size = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap()) as usize;
    let flen = i32::from_le_bytes(file_bytes[12..16].try_into().unwrap());
    let prefix_sz = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize });
    let p = prefix_sz;
    let _pkg_flags = u32::from_le_bytes(file_bytes[p..p + 4].try_into().unwrap());
    let name_count = i32::from_le_bytes(file_bytes[p + 4..p + 8].try_into().unwrap());
    let name_offset = u32::from_le_bytes(file_bytes[p + 8..p + 12].try_into().unwrap()) as usize;

    let p_sum = prefix_sz + 12;
    let export_count = i32::from_le_bytes(file_bytes[p_sum..p_sum + 4].try_into().unwrap()) as usize;
    let export_offset = i32::from_le_bytes(file_bytes[p_sum + 4..p_sum + 8].try_into().unwrap()) as usize;
    let depends_offset = i32::from_le_bytes(file_bytes[p_sum + 16..p_sum + 20].try_into().unwrap()) as usize;

    let enc_size = (total_header_size - name_offset + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end > file_bytes.len() {
        return Err("ProjectX header bounds invalid".into());
    }

    let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
    let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count)
        .map_err(|e| format!("parse names in ProjectX: {e}"))?;

    let chunks_rel = depends_offset - name_offset;
    if chunks_rel + 4 > plain_header.len() {
        return Err("ProjectX chunks table out of bounds".into());
    }
    let num_chunks = u32::from_le_bytes(plain_header[chunks_rel..chunks_rel + 4].try_into().unwrap()) as usize;
    let mut c_p = chunks_rel + 4;
    let mut chunks = Vec::with_capacity(num_chunks);
    for _ in 0..num_chunks {
        if c_p + 36 > plain_header.len() { break; }
        let u_off = u64::from_le_bytes(plain_header[c_p..c_p + 8].try_into().unwrap());
        let u_sz = i32::from_le_bytes(plain_header[c_p + 8..c_p + 12].try_into().unwrap());
        let c_off = u64::from_le_bytes(plain_header[c_p + 12..c_p + 20].try_into().unwrap());
        let c_sz = i32::from_le_bytes(plain_header[c_p + 20..c_p + 24].try_into().unwrap());
        chunks.push((u_off, u_sz, c_off, c_sz));
        c_p += 36;
    }

    if chunks.len() < 2 {
        return Err("ProjectX has fewer than 2 chunks".into());
    }

    // Parse exports
    let export_rel = export_offset - name_offset;
    let mut pos = export_rel;
    struct PxExport {
        idx: usize,
        name: String,
        outer_idx: i32,
        _serial_size: usize,
        serial_offset: u64,
    }
    let mut raw_exports = Vec::with_capacity(export_count);
    for i in 0..export_count {
        if pos + 72 > chunks_rel { break; }
        let _class_idx = i32::from_le_bytes(plain_header[pos..pos + 4].try_into().unwrap());
        let _super_idx = i32::from_le_bytes(plain_header[pos + 4..pos + 8].try_into().unwrap());
        let outer_idx = i32::from_le_bytes(plain_header[pos + 8..pos + 12].try_into().unwrap());
        let name_idx = i32::from_le_bytes(plain_header[pos + 12..pos + 16].try_into().unwrap());
        let serial_size = i32::from_le_bytes(plain_header[pos + 32..pos + 36].try_into().unwrap()) as usize;
        let serial_offset = u64::from_le_bytes(plain_header[pos + 36..pos + 44].try_into().unwrap());
        let noc = i32::from_le_bytes(plain_header[pos + 48..pos + 52].try_into().unwrap());
        let nm = names.get(name_idx as usize).cloned().unwrap_or_default();
        raw_exports.push(PxExport { idx: i + 1, name: nm, outer_idx, _serial_size: serial_size, serial_offset });
        pos += 72 + std::cmp::max(0, noc as usize) * 4;
    }

    let ch1 = &chunks[1];
    let ch1_start = ch1.2 as usize;
    let ch1_end = ch1_start + ch1.3 as usize;
    if ch1_end > file_bytes.len() {
        return Err("ProjectX Chunk 1 out of bounds".into());
    }

    if ch1_start + 16 <= file_bytes.len() {
        let cur_c_sz = u32::from_le_bytes(file_bytes[ch1_start + 8..ch1_start + 12].try_into().unwrap());
        let cur_u_sz = u32::from_le_bytes(file_bytes[ch1_start + 12..ch1_start + 16].try_into().unwrap());
        let num_b = (cur_u_sz + 131072 - 1) / 131072;
        if (num_b + 1) * 8 + cur_c_sz > ch1.3 as u32 {
            let mut bp = ch1_start + 16;
            let mut sum_b = 0i32;
            for _ in 0..num_b {
                if bp + 8 > file_bytes.len() { break; }
                let c = i32::from_le_bytes(file_bytes[bp..bp + 4].try_into().unwrap());
                sum_b = sum_b.saturating_add(c);
                bp += 8;
            }
            if (num_b + 1) * 8 + sum_b as u32 <= ch1.3 as u32 {
                file_bytes[ch1_start + 8..ch1_start + 12].copy_from_slice(&sum_b.to_le_bytes());
                if bak_path.is_file() {
                    let _ = fs::write(&bak_path, &file_bytes);
                }
            }
        }
    }

    let mut decomp1 = crate::upk::compression::decompress_chunk(&file_bytes[ch1_start..ch1_end])
        .map_err(|e| format!("decompress ProjectX Chunk 1: {e}"))?;

    // 0. Patch PsyNetConnection_X::GetPsyNetURL -> return "http://127.0.0.1:15759/rpc";
    if let Some(rpc_exp) = raw_exports.iter().find(|e| {
        e.name == "GetPsyNetURL" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "PsyNetConnection_X").unwrap_or(false)
    }) {
        let code_start = (rpc_exp.serial_offset - ch1.0) as usize + 48;
        let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
        let target_url = b"http://127.0.0.1:15759/rpc\0";
        if disk_sz >= target_url.len() + 2 {
            let mut patch = Vec::with_capacity(disk_sz);
            patch.push(opcodes::EX_RETURN);
            patch.push(opcodes::EX_STRING_CONST);
            patch.extend_from_slice(target_url);
            patch.resize(disk_sz.saturating_sub(1), opcodes::EX_NOTHING);
            patch.push(opcodes::EX_END_OF_SCRIPT);
            if patch.len() == disk_sz {
                decomp1[code_start..code_start + disk_sz].copy_from_slice(&patch);
                // Synchronize mem_sz = disk_sz to prevent UE3 bytecode deserializer overshoot (Bad expr token 00)
                decomp1[code_start - 8..code_start - 4].copy_from_slice(&(disk_sz as u32).to_le_bytes());
            }
        }
    }

    // 0b. Patch PsyNetMessengerWebSocket_X::Init -> InURL = "ws://127.0.0.1:15759/"; URL = InURL; HandshakeHeaders = InHandshakeHeaders;
    if let Some(ws_exp) = raw_exports.iter().find(|e| {
        e.name == "Init" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "PsyNetMessengerWebSocket_X").unwrap_or(false)
    }) {
        let ws_code_start = (ws_exp.serial_offset - ch1.0) as usize + 48;
        let ws_disk_sz = u32::from_le_bytes(decomp1[ws_code_start - 4..ws_code_start].try_into().unwrap()) as usize;
        if ws_disk_sz >= 54 {
            const WS_INIT_BYTECODE: [u8; 54] = [
                0x0f, 0x2b, 0x69, 0x38, 0x00, 0x00, 0x1f, 0x77, 0x73, 0x3a, 0x2f, 0x2f, 0x31, 0x32, 0x37, 0x2e,
                0x30, 0x2e, 0x30, 0x2e, 0x31, 0x3a, 0x31, 0x35, 0x37, 0x35, 0x39, 0x2f, 0x00, 0x0f, 0x01, 0x58,
                0x38, 0x00, 0x00, 0x2b, 0x69, 0x38, 0x00, 0x00, 0x0f, 0x01, 0x59, 0x38, 0x00, 0x00, 0x2b, 0x68,
                0x38, 0x00, 0x00, 0x04, 0x0b, 0x4c,
            ];
            let mut ws_patch = WS_INIT_BYTECODE.to_vec();
            ws_patch.resize(ws_disk_sz.saturating_sub(1), opcodes::EX_NOTHING);
            ws_patch.push(opcodes::EX_END_OF_SCRIPT);
            if ws_patch.len() == ws_disk_sz {
                decomp1[ws_code_start..ws_code_start + ws_disk_sz].copy_from_slice(&ws_patch);
                decomp1[ws_code_start - 8..ws_code_start - 4].copy_from_slice(&74u32.to_le_bytes());
            }
        }
    }

    // 1. Patch PsyNetUrl_X::ValidateURL -> return InURL;
    if let Some(val_exp) = raw_exports.iter().find(|e| {
        e.name == "ValidateURL" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "PsyNetUrl_X").unwrap_or(false)
    }) {
        let val_code_start = (val_exp.serial_offset - ch1.0) as usize + 48;
        let val_disk_sz = u32::from_le_bytes(decomp1[val_code_start - 4..val_code_start].try_into().unwrap()) as usize;
        let in_url_idx = raw_exports.iter()
            .find(|c| c.outer_idx == val_exp.idx as i32 && c.name == "InURL")
            .map(|c| c.idx as u32)
            .unwrap_or(18897);

        let mut val_patch = vec![opcodes::EX_RETURN, opcodes::EX_INSTANCE_VARIABLE];
        val_patch.extend_from_slice(&in_url_idx.to_le_bytes());
        val_patch.resize(val_disk_sz.saturating_sub(1), opcodes::EX_NOTHING);
        val_patch.push(opcodes::EX_END_OF_SCRIPT);
        if val_patch.len() == val_disk_sz {
            decomp1[val_code_start..val_code_start + val_disk_sz].copy_from_slice(&val_patch);
            // In 64-bit UE3, EX_INSTANCE_VARIABLE resolves to an 8-byte pointer in memory (+4 bytes).
            // Synchronize mem_sz = val_disk_sz + 4 to match exact deserialization size.
            let target_mem_sz = (val_disk_sz + 4) as u32;
            decomp1[val_code_start - 8..val_code_start - 4].copy_from_slice(&target_mem_sz.to_le_bytes());
        }
    }

    // 2. Patch all boolean URL and PsyNet validation functions -> return true;
    let bool_functions = [
        ("PsyNetUrl_X", "IsAllowedURL"),
        ("URLValidator_X", "IsURLValid"),
        ("URLValidator_X", "IsSchemeValid"),
        ("URLValidator_X", "HostLooksLikeIPv4"),
        ("URLValidator_X", "IsIPv4AndPortValid"),
        ("URLValidator_X", "IsIPv4AddressValid"),
        ("URLValidator_X", "IsPortValid"),
        ("URLValidator_X", "IsHostnameAndPortValid"),
        ("URLValidator_X", "IsHostnameValid"),
        ("URLValidator_X", "IsPathValid"),
        ("URLValidator_X", "IsCharUrlSafe"),
        ("URLValidator_X", "IsQueryValid"),
        ("URLValidator_X", "AreCharsUrlSafe"),
        ("OnlineGameDLC_X", "DoesPlayerOwnDLC"),
    ];

    for (outer, fn_name) in bool_functions {
        if let Some(exp) = raw_exports.iter().find(|e| {
            e.name == fn_name && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == outer).unwrap_or(false)
        }) {
            if exp._serial_size > 48 {
                let code_start = (exp.serial_offset - ch1.0) as usize + 48;
                let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
                if disk_sz >= 2 {
                    let mut patch = vec![opcodes::EX_RETURN, opcodes::EX_TRUE_CONST];
                    patch.resize(disk_sz.saturating_sub(1), opcodes::EX_NOTHING);
                    patch.push(opcodes::EX_END_OF_SCRIPT);
                    if patch.len() == disk_sz {
                        decomp1[code_start..code_start + disk_sz].copy_from_slice(&patch);
                        // Synchronize mem_sz = disk_sz to prevent UE3 bytecode deserializer overshoot (Bad expr token 00)
                        decomp1[code_start - 8..code_start - 4].copy_from_slice(&(disk_sz as u32).to_le_bytes());
                    }
                }
            }
        }
    }

    // 3. Patch PsyNet check and privilege validation functions -> return;
    // Note: PrivilegeCheck_X functions must remain vanilla so that checking privileges
    // completes and never hangs in the lobby.
    let void_functions: [(&str, &str); 0] = [];

    for (outer, fn_name) in void_functions {
        if let Some(exp) = raw_exports.iter().find(|e| {
            e.name == fn_name && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == outer).unwrap_or(false)
        }) {
            if exp._serial_size > 48 {
                let code_start = (exp.serial_offset - ch1.0) as usize + 48;
                let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
                let mut patch = vec![opcodes::EX_RETURN];
                patch.resize(disk_sz.saturating_sub(1), opcodes::EX_NOTHING);
                patch.push(opcodes::EX_END_OF_SCRIPT);
                if patch.len() == disk_sz {
                    decomp1[code_start..code_start + disk_sz].copy_from_slice(&patch);
                    // Synchronize mem_sz = disk_sz because the no-op patch contains 0 property references
                    decomp1[code_start - 8..code_start - 4].copy_from_slice(&(disk_sz as u32).to_le_bytes());
                }
            }
        }
    }

    // Restore PrivilegeCheck_X functions to 100% pristine vanilla stock if poisoned
    if let Some(exp) = raw_exports.iter().find(|e| {
        e.name == "CheckPsyNet" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "PrivilegeCheck_X").unwrap_or(false)
    }) {
        if exp._serial_size > 48 {
            let code_start = (exp.serial_offset - ch1.0) as usize + 48;
            let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
            if disk_sz == 73 && (decomp1.get(code_start) == Some(&opcodes::EX_RETURN) || u32::from_le_bytes(decomp1[code_start - 8..code_start - 4].try_into().unwrap()) != 109) {
                const STOCK_CHECKPSYNET_BYTECODE: [u8; 73] = [
                    0x5e, 0x19, 0x00, 0x19, 0x00, 0x19, 0x00, 0x1c, 0xff, 0xfd, 0xff, 0xff, 0x16, 0x13, 0x00, 0x1b,
                    0xfc, 0xff, 0xff, 0x00, 0x1c, 0xd4, 0xfd, 0xff, 0xff, 0x20, 0x89, 0x21, 0x00, 0x00, 0x16, 0x0a,
                    0x00, 0x5e, 0x21, 0x00, 0x00, 0x00, 0x1c, 0x5f, 0x21, 0x00, 0x00, 0x16, 0x1b, 0x00, 0x25, 0xfc,
                    0xff, 0xff, 0x00, 0x1c, 0x34, 0xfe, 0xff, 0xff, 0x43, 0x05, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x16, 0x04, 0x0b, 0x4c,
                ];
                decomp1[code_start..code_start + 73].copy_from_slice(&STOCK_CHECKPSYNET_BYTECODE);
                decomp1[code_start - 8..code_start - 4].copy_from_slice(&109u32.to_le_bytes());
                decomp1[code_start - 4..code_start].copy_from_slice(&73u32.to_le_bytes());
            }
        }
    }

    if let Some(exp) = raw_exports.iter().find(|e| {
        e.name == "CheckPsyNetComplete" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "PrivilegeCheck_X").unwrap_or(false)
    }) {
        if exp._serial_size > 48 {
            let code_start = (exp.serial_offset - ch1.0) as usize + 48;
            let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
            if disk_sz == 48 && (decomp1.get(code_start) == Some(&opcodes::EX_RETURN) || u32::from_le_bytes(decomp1[code_start - 8..code_start - 4].try_into().unwrap()) != 68) {
                const STOCK_CHECKPSYNETCOMPLETE_BYTECODE: [u8; 48] = [
                    0x07, 0x1c, 0x00, 0xcf, 0x2b, 0x9d, 0x36, 0x00, 0x00, 0x2a, 0x16, 0x1c, 0x8f, 0x36, 0x00, 0x00,
                    0x16, 0x06, 0x41, 0x00, 0x0f, 0x01, 0x85, 0x36, 0x00, 0x00, 0x2b, 0x9d, 0x36, 0x00, 0x00, 0x1c,
                    0xa8, 0x36, 0x00, 0x00, 0x1f, 0x50, 0x73, 0x79, 0x4e, 0x65, 0x74, 0x00, 0x16, 0x04, 0x0b, 0x4c,
                ];
                decomp1[code_start..code_start + 48].copy_from_slice(&STOCK_CHECKPSYNETCOMPLETE_BYTECODE);
                decomp1[code_start - 8..code_start - 4].copy_from_slice(&68u32.to_le_bytes());
                decomp1[code_start - 4..code_start].copy_from_slice(&48u32.to_le_bytes());
            }
        }
    }

    if let Some(exp) = raw_exports.iter().find(|e| {
        e.name == "ValidateUserOnlineAccount" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "PrivilegeCheck_X").unwrap_or(false)
    }) {
        if exp._serial_size > 48 {
            let code_start = (exp.serial_offset - ch1.0) as usize + 48;
            let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
            if disk_sz == 80 && (decomp1.get(code_start) == Some(&opcodes::EX_RETURN) || u32::from_le_bytes(decomp1[code_start - 8..code_start - 4].try_into().unwrap()) != 112) {
                const STOCK_VALIDATEUSERONLINEACCOUNT_BYTECODE: [u8; 80] = [
                    0x07, 0x52, 0x00, 0x5e, 0x19, 0x00, 0x19, 0x00, 0x1c, 0xff, 0xfd, 0xff, 0xff, 0x16, 0x13, 0x00,
                    0x1b, 0xfc, 0xff, 0xff, 0x00, 0x1c, 0xd4, 0xfd, 0xff, 0xff, 0x20, 0x89, 0x21, 0x00, 0x00, 0x16,
                    0x0a, 0x00, 0x57, 0x21, 0x00, 0x00, 0x00, 0x1c, 0x58, 0x21, 0x00, 0x00, 0x16, 0x1c, 0x8f, 0x36,
                    0x00, 0x00, 0x16, 0x06, 0x6d, 0x00, 0x1c, 0xa8, 0x36, 0x00, 0x00, 0x1f, 0x53, 0x69, 0x67, 0x6e,
                    0x69, 0x6e, 0x43, 0x61, 0x6e, 0x63, 0x65, 0x6c, 0x6c, 0x65, 0x64, 0x00, 0x16, 0x04, 0x0b, 0x4c,
                ];
                decomp1[code_start..code_start + 80].copy_from_slice(&STOCK_VALIDATEUSERONLINEACCOUNT_BYTECODE);
                decomp1[code_start - 8..code_start - 4].copy_from_slice(&112u32.to_le_bytes());
                decomp1[code_start - 4..code_start].copy_from_slice(&80u32.to_le_bytes());
            }
        }
    }

    // 4. Guarantee OnlineGame_X::ValidateUserOnlineAccount is 100% pristine vanilla stock.
    // Self-heals in case the input backup or game file was poisoned by legacy patches.
    if let Some(exp) = raw_exports.iter().find(|e| {
        e.name == "ValidateUserOnlineAccount" && e.outer_idx > 0 && raw_exports.get(e.outer_idx as usize - 1).map(|o| o.name == "OnlineGame_X").unwrap_or(false)
    }) {
        if exp._serial_size > 48 {
            let code_start = (exp.serial_offset - ch1.0) as usize + 48;
            let disk_sz = u32::from_le_bytes(decomp1[code_start - 4..code_start].try_into().unwrap()) as usize;
            if disk_sz == 618 && (decomp1.get(code_start) == Some(&opcodes::EX_RETURN) || u32::from_le_bytes(decomp1[code_start - 8..code_start - 4].try_into().unwrap()) != 878) {
                const STOCK_VALIDATE_USER_BYTECODE: [u8; 618] = [
                    0x07, 0x2b, 0x00, 0xf2, 0x5e, 0x12, 0x00, 0x20, 0xb3, 0xfe, 0xff, 0xff, 0x0c, 0x00, 0x7d, 0xff,
                    0xff, 0xff, 0x00, 0x1c, 0x8d, 0xfc, 0xff, 0xff, 0x24, 0x0d, 0x16, 0x28, 0x16, 0x04, 0x27, 0x07,
                    0x3d, 0x00, 0xf2, 0x1c, 0x5a, 0x21, 0x00, 0x00, 0x16, 0x27, 0x16, 0x04, 0x28, 0x14, 0x2d, 0x2b,
                    0x56, 0x21, 0x00, 0x00, 0x27, 0x57, 0x0a, 0x01, 0x06, 0x21, 0x00, 0x00, 0x5b, 0x0b, 0x2b, 0x54,
                    0x21, 0x00, 0x00, 0x48, 0x57, 0x03, 0x07, 0xfc, 0x00, 0x9b, 0x38, 0x3a, 0x5e, 0x19, 0x00, 0x2b,
                    0x54, 0x21, 0x00, 0x00, 0x0a, 0x00, 0x33, 0x2e, 0x00, 0x00, 0x00, 0x1c, 0x34, 0x2e, 0x00, 0x00,
                    0x16, 0x38, 0x3a, 0x24, 0x03, 0x16, 0xe7, 0xf8, 0xf8, 0xa8, 0x38, 0x57, 0x1c, 0x01, 0xfe, 0xff,
                    0xff, 0x16, 0x1f, 0x4f, 0x6e, 0x6c, 0x69, 0x6e, 0x65, 0x20, 0x61, 0x63, 0x63, 0x6f, 0x75, 0x6e,
                    0x74, 0x20, 0x00, 0x16, 0x38, 0x53, 0x5e, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09, 0x00,
                    0xb5, 0x2d, 0x00, 0x00, 0x00, 0x01, 0xb5, 0x2d, 0x00, 0x00, 0x16, 0x1f, 0x20, 0x6e, 0x6f, 0x74,
                    0x20, 0x76, 0x61, 0x6c, 0x69, 0x64, 0x61, 0x74, 0x65, 0x64, 0x2e, 0x00, 0x16, 0x21, 0xa2, 0x1b,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x16, 0x14, 0x2d, 0x2b, 0x56, 0x21, 0x00, 0x00, 0x28,
                    0x07, 0x56, 0x03, 0x82, 0x9a, 0x38, 0x3a, 0x5e, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09,
                    0x00, 0xb6, 0x2d, 0x00, 0x00, 0x00, 0x01, 0xb6, 0x2d, 0x00, 0x00, 0x38, 0x3a, 0x24, 0x02, 0x16,
                    0x18, 0x3b, 0x00, 0xf2, 0x5e, 0x19, 0x00, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09, 0x00,
                    0xbd, 0x2d, 0x00, 0x00, 0x00, 0x01, 0xbd, 0x2d, 0x00, 0x00, 0x0a, 0x00, 0x88, 0x37, 0x00, 0x00,
                    0x00, 0x2d, 0x01, 0x88, 0x37, 0x00, 0x00, 0x28, 0x16, 0x16, 0x07, 0xdb, 0x01, 0xf2, 0x5e, 0x19,
                    0x00, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09, 0x00, 0xbe, 0x2d, 0x00, 0x00, 0x00, 0x01,
                    0xbe, 0x2d, 0x00, 0x00, 0x0a, 0x00, 0xc0, 0x2e, 0x00, 0x00, 0x00, 0x1b, 0x62, 0x17, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x16, 0x28, 0x16, 0x5e, 0x19, 0x00, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00,
                    0x00, 0x09, 0x00, 0xbe, 0x2d, 0x00, 0x00, 0x00, 0x01, 0xbe, 0x2d, 0x00, 0x00, 0x0b, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x1c, 0xb5, 0x2e, 0x00, 0x00, 0x48, 0x16, 0x07, 0x87, 0x02, 0x82, 0xce,
                    0x5e, 0x19, 0x00, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09, 0x00, 0xbe, 0x2d, 0x00, 0x00,
                    0x00, 0x01, 0xbe, 0x2d, 0x00, 0x00, 0x09, 0x00, 0x79, 0x2e, 0x00, 0x00, 0x00, 0x01, 0x79, 0x2e,
                    0x00, 0x00, 0x2a, 0x16, 0x18, 0x0e, 0x00, 0xf2, 0x2d, 0x2b, 0x55, 0x21, 0x00, 0x00, 0x28, 0x16,
                    0x16, 0x42, 0x00, 0xfc, 0x20, 0x00, 0x00, 0xcb, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5e,
                    0x19, 0x00, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09, 0x00, 0xbe, 0x2d, 0x00, 0x00, 0x00,
                    0x01, 0xbe, 0x2d, 0x00, 0x00, 0x09, 0x00, 0x79, 0x2e, 0x00, 0x00, 0x00, 0x01, 0x79, 0x2e, 0x00,
                    0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x16, 0x14, 0x2d, 0x2b, 0x55, 0x21, 0x00, 0x00, 0x27, 0x07,
                    0x4a, 0x03, 0x82, 0x5e, 0x19, 0x00, 0x19, 0x00, 0x19, 0x00, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x09,
                    0x00, 0xbe, 0x2d, 0x00, 0x00, 0x00, 0x01, 0xbe, 0x2d, 0x00, 0x00, 0x09, 0x00, 0x70, 0x2e, 0x00,
                    0x00, 0x00, 0x01, 0x70, 0x2e, 0x00, 0x00, 0x0a, 0x00, 0xe7, 0x02, 0x00, 0x00, 0x00, 0x2d, 0x01,
                    0xe7, 0x02, 0x00, 0x00, 0x18, 0x0e, 0x00, 0xf2, 0x2d, 0x2b, 0x55, 0x21, 0x00, 0x00, 0x28, 0x16,
                    0x16, 0x42, 0x00, 0xfc, 0x20, 0x00, 0x00, 0xcb, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5e,
                    0x19, 0x00, 0x12, 0x00, 0x20, 0x57, 0x16, 0x00, 0x00, 0x09, 0x00, 0x4f, 0x16, 0x00, 0x00, 0x00,
                    0x02, 0x4f, 0x16, 0x00, 0x00, 0x0c, 0x00, 0x1f, 0xfc, 0xff, 0xff, 0x00, 0x1c, 0x1c, 0xfe, 0xff,
                    0xff, 0x48, 0x48, 0x16, 0x2b, 0x54, 0x21, 0x00, 0x00, 0x16, 0x14, 0x2d, 0x2b, 0x55, 0x21, 0x00,
                    0x00, 0x27, 0x14, 0x2d, 0x2b, 0x56, 0x21, 0x00, 0x00, 0x28, 0x31, 0x30, 0x04, 0x2d, 0x2b, 0x56,
                    0x21, 0x00, 0x00, 0x04, 0x3a, 0x57, 0x21, 0x00, 0x00, 0x4c,
                ];
                decomp1[code_start..code_start + 618].copy_from_slice(&STOCK_VALIDATE_USER_BYTECODE);
                decomp1[code_start - 8..code_start - 4].copy_from_slice(&878u32.to_le_bytes());
                decomp1[code_start - 4..code_start].copy_from_slice(&618u32.to_le_bytes());
            }
        }
    }

    // Recompress Chunk 1
    let mut recomp = crate::upk::compression::compress_chunk(&decomp1)
        .map_err(|e| format!("recompress ProjectX Chunk 1: {e}"))?;

    let max_comp = ch1.3 as usize;
    if recomp.len() > max_comp {
        return Err(format!("Recompressed ProjectX Chunk 1 exceeded allocation: {} > {}", recomp.len(), max_comp));
    }

    recomp.resize(max_comp, 0); // Fill slack with zeroes
    file_bytes[ch1_start..ch1_end].copy_from_slice(&recomp);

    fs::write(&px_path, &file_bytes)
        .map_err(|e| format!("Failed to write patched ProjectX.upk: {e}"))?;

    crate::applog::event("projectx: PsyNet URL redirection and validation functions patched successfully (http://127.0.0.1:15759/rpc)");
    Ok(true)
}

/// Check if TAGame.upk and ProjectX.upk are patched.
/// Never download external 82MB binaries on startup or overwrite files automatically.
pub async fn ensure_tagame_patched(app: &tauri::AppHandle, cooked_dir: &Path) -> Result<bool, String> {
    let tg_path = if cooked_dir.join("TAGame.upk").is_file() {
        cooked_dir.join("TAGame.upk")
    } else if cooked_dir.join("TAGame/CookedPCConsole/TAGame.upk").is_file() {
        cooked_dir.join("TAGame/CookedPCConsole/TAGame.upk")
    } else {
        return Ok(false);
    };

    if let Ok(cur_tg_bytes) = fs::read(&tg_path) {
        if tagame_has_rlim_patch(&cur_tg_bytes) {
            return Ok(true);
        }
    }

    let mut patch_file = app.path().resource_dir().ok().map(|dir| dir.join("rl_patch.bin"));
    if patch_file.as_ref().map_or(true, |p| !p.is_file()) {
        patch_file = Some(PathBuf::from("resources/rl_patch.bin"));
    }
    let patch_file = patch_file.filter(|p| p.is_file()).ok_or("bundled bytecode patch payload not found")?;
    let patch_bytes = fs::read(&patch_file).map_err(|e| e.to_string())?;

    let backup = cooked_dir.join(TAGAME_BACKUP_NAME);
    if !backup.is_file() {
        let _ = fs::copy(&tg_path, &backup);
    }
    let applied = patch_tagame_in_place(&tg_path, &backup, &patch_bytes)?;
    if applied == 0 {
        return Err("no TAGame patch records could be applied".into());
    }
    crate::applog::event(&format!(
        "upk: patched {applied} TAGame exports in place",
    ));
    Ok(true)
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
        assert_eq!(bc[4], opcodes::EX_DYN_ARRAY_ELEMENT);
        assert!(mem_sz >= 300);
    }

    #[test]
    fn test_emit_car_preview_set_loadout_bytecode() {
        let rules = [
            SlotSwapRule { slot_idx: 0, owned_id: Some(23), target_id: 4284 },
            SlotSwapRule { slot_idx: 1, owned_id: None, target_id: 250 },
            SlotSwapRule { slot_idx: 2, owned_id: None, target_id: 30 },
            SlotSwapRule { slot_idx: 3, owned_id: None, target_id: 50 },
            SlotSwapRule { slot_idx: 5, owned_id: None, target_id: 30 },
            SlotSwapRule { slot_idx: 14, owned_id: None, target_id: 80 },
            SlotSwapRule { slot_idx: 15, owned_id: None, target_id: 2044 },
        ];
        let (bc, mem_sz) = emit_car_preview_set_loadout_bytecode(&rules, 230, None, None, None, None, None).unwrap();
        assert_eq!(bc.len(), 230);
        assert_eq!(bc[0], opcodes::EX_LET);
        assert!(mem_sz >= 230);
    }

    #[test]
    fn test_emit_car_preview_change_car_part_bytecode() {
        let rules = [
            SlotSwapRule { slot_idx: 0, owned_id: Some(23), target_id: 4284 },
            SlotSwapRule { slot_idx: 2, owned_id: Some(376), target_id: 30 },
            SlotSwapRule { slot_idx: 3, owned_id: Some(29), target_id: 12968 },
            SlotSwapRule { slot_idx: 15, owned_id: Some(1903), target_id: 2044 },
        ];
        let (bc, mem_sz) = emit_car_preview_change_car_part_bytecode(&rules, 230, None, None, None, None, None, None).unwrap();
        assert_eq!(bc.len(), 230);
        assert_eq!(bc[0], opcodes::EX_JUMP_IF_NOT);
        assert!(mem_sz >= 230);
    }

    #[test]
    fn test_apply_tagame_modifications_real_upk() {
        let cooked_bak = if Path::new("/root/TAGame.upk").is_file() {
            Path::new("/root")
        } else {
            Path::new(r"E:\games\rocketleague\TAGame\CookedPCConsole")
        };
        let src_file = if cooked_bak.join("TAGame.upk.bak").is_file() {
            cooked_bak.join("TAGame.upk.bak")
        } else if cooked_bak.join("TAGame.upk").is_file() {
            cooked_bak.join("TAGame.upk")
        } else {
            return;
        };

        let temp_dir = std::env::temp_dir().join("velocityrl_test_tagame");
        let _ = fs::create_dir_all(&temp_dir);
        let tagame_test_bak = temp_dir.join("TAGame.upk.bak");
        let tagame_test = temp_dir.join("TAGame.upk");

        let _ = fs::copy(&src_file, &tagame_test_bak);
        let _ = fs::copy(&src_file, &tagame_test);

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
                slot_index: Some(15),
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

    #[test]
    fn test_emit_pri_get_ping_bytecode() {
        let bc = emit_pri_get_ping_bytecode(16, 24, 199);
        assert_eq!(bc.len(), 199);
        assert_eq!(bc[0], opcodes::EX_RETURN);
        assert_eq!(bc[1], 0x38);
        assert_eq!(bc[2], 0x44);
        assert_eq!(bc[3], opcodes::EX_FINAL_FUNCTION);
        let token = i32::from_le_bytes(bc[4..8].try_into().unwrap());
        assert_eq!(token, -1885);
        assert_eq!(bc[8], opcodes::EX_FLOAT_CONST);
        let min_val = f32::from_le_bytes(bc[9..13].try_into().unwrap());
        assert_eq!(min_val, 16.0);
        assert_eq!(bc[13], opcodes::EX_FLOAT_CONST);
        let max_val = f32::from_le_bytes(bc[14..18].try_into().unwrap());
        assert_eq!(max_val, 24.0);
        assert_eq!(bc[18], opcodes::EX_END_FUNCTION_PARMS);
        assert_eq!(bc[19], opcodes::EX_RETURN);
        assert_eq!(bc[20], opcodes::EX_INT_ZERO);
        assert_eq!(bc[21], opcodes::EX_NOTHING);
        assert_eq!(bc[198], opcodes::EX_END_OF_SCRIPT);
    }

    #[test]
    fn test_emit_noop_void_function_bytecode() {
        let (payload, mem_sz) = emit_noop_void_function_bytecode(191);
        assert_eq!(payload.len(), 191);
        assert_eq!(payload[0], opcodes::EX_RETURN);
        for i in 1..190 {
            assert_eq!(payload[i], opcodes::EX_NOTHING);
        }
        assert_eq!(payload[190], opcodes::EX_END_OF_SCRIPT);
        assert_eq!(mem_sz, 191);
    }

    #[test]
    fn test_emit_noop_bool_function_bytecode() {
        let (payload, mem_sz) = emit_noop_bool_function_bytecode(true, 69);
        assert_eq!(payload.len(), 69);
        assert_eq!(payload[0], opcodes::EX_RETURN); // 0x04
        assert_eq!(payload[1], opcodes::EX_TRUE_CONST); // 0x27
        for i in 2..68 {
            assert_eq!(payload[i], opcodes::EX_NOTHING);
        }
        assert_eq!(payload[68], opcodes::EX_END_OF_SCRIPT); // 0x4C (ALWAYS LAST!)
        assert_eq!(mem_sz, 69);
    }

    #[test]
    fn test_emit_modify_loadout_passthrough_bytecode() {
        let (payload, mem_sz) = emit_modify_loadout_passthrough_bytecode(59196, 59200, 59197, 218);
        assert_eq!(payload.len(), 218);
        assert_eq!(payload[0], opcodes::EX_NOTHING); // 0x0B
        assert_eq!(payload[1], opcodes::EX_LET); // 0x0F
        assert_eq!(payload[2], opcodes::EX_INSTANCE_VARIABLE); // 0x2B
        assert_eq!(&payload[3..7], &59196i32.to_le_bytes());
        assert_eq!(payload[7], opcodes::EX_LOCAL_VARIABLE); // 0x46
        assert_eq!(&payload[8..12], &59200i32.to_le_bytes());
        assert_eq!(payload[12], opcodes::EX_RETURN); // 0x04
        assert_eq!(payload[13], opcodes::EX_INSTANCE_VARIABLE); // 0x2B
        assert_eq!(&payload[14..18], &59196i32.to_le_bytes());
        assert_eq!(payload[18], opcodes::EX_RETURN); // 0x04
        assert_eq!(payload[19], opcodes::EX_RETURN_VALUE); // 0x3A
        assert_eq!(&payload[20..24], &59197i32.to_le_bytes());
        for i in 24..217 {
            assert_eq!(payload[i], opcodes::EX_NOTHING);
        }
        assert_eq!(payload[217], opcodes::EX_END_OF_SCRIPT); // 0x4C (ALWAYS LAST!)
        assert_eq!(mem_sz, 234); // 40 active mem + 193 NOPs + 1 = 234
    }

    #[test]
    fn test_dump_validation_exports() {
        let tagame_path = Path::new("/root/TAGame_clean.upk");
        if !tagame_path.exists() {
            return;
        }

        let file_bytes = fs::read(tagame_path).unwrap();
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

        let plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
        let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count).unwrap();

        let p_sum = 12 + 4 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize }) + 4 + 8;
        let export_count = i32::from_le_bytes(file_bytes[p_sum..p_sum + 4].try_into().unwrap()) as usize;
        let export_offset = i32::from_le_bytes(file_bytes[p_sum + 4..p_sum + 8].try_into().unwrap()) as usize;
        let depends_offset = i32::from_le_bytes(file_bytes[p_sum + 16..p_sum + 20].try_into().unwrap()) as usize;

        let export_rel = export_offset - name_offset;
        let depends_rel = depends_offset - name_offset;

        #[allow(dead_code)]
        struct ExportItem {
            idx: usize,
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
            } else { String::new() };

            let cur_idx = exports.len() + 1;
            exports.push(ExportItem {
                idx: cur_idx,
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
            idx: usize,
            uncomp_offset: i64,
            uncomp_size: i32,
            comp_offset: i64,
            comp_size: i32,
        }
        let mut c_pos = depends_rel;
        let chunk_count = i32::from_le_bytes(plain_header[c_pos..c_pos + 4].try_into().unwrap()) as usize;
        c_pos += 4;
        let mut chunks = Vec::with_capacity(chunk_count);
        for i in 0..chunk_count {
            if c_pos + 36 > plain_header.len() { break; }
            let uncomp_offset = i64::from_le_bytes(plain_header[c_pos..c_pos + 8].try_into().unwrap());
            let uncomp_size = i32::from_le_bytes(plain_header[c_pos + 8..c_pos + 12].try_into().unwrap());
            let comp_offset = i64::from_le_bytes(plain_header[c_pos + 12..c_pos + 20].try_into().unwrap());
            let comp_size = i32::from_le_bytes(plain_header[c_pos + 20..c_pos + 24].try_into().unwrap());
            chunks.push(ChunkItem {
                idx: i,
                uncomp_offset,
                uncomp_size,
                comp_offset,
                comp_size,
            });
            c_pos += 36;
        }

        if chunks.len() > 1 {
            let c1_start = chunks[1].comp_offset as usize;
            let c1_end = c1_start + chunks[1].comp_size as usize;
            if let Ok(decomp1) = crate::upk::compression::decompress_chunk(&file_bytes[c1_start..c1_end]) {
                let recomp1 = crate::upk::compression::compress_chunk(&decomp1).expect("Recompression of Chunk 1 must succeed");
                assert!(recomp1.len() <= chunks[1].comp_size as usize, "Recompressed Chunk 1 must fit within allocated size");
            }
        }

        if chunks.len() > 2 {
            let c2_start = chunks[2].comp_offset as usize;
            let c2_end = c2_start + chunks[2].comp_size as usize;
            if let Ok(mut decomp2) = crate::upk::compression::decompress_chunk(&file_bytes[c2_start..c2_end]) {
                if let Some(exp_ml) = exports.iter().find(|e| e.name == "ModifyLoadout" && e.outer_name == "ProductAttribute_TA") {
                    if exp_ml.serial_offset >= chunks[2].uncomp_offset && exp_ml.serial_offset < (chunks[2].uncomp_offset + chunks[2].uncomp_size as i64) {
                        let func_off = (exp_ml.serial_offset - chunks[2].uncomp_offset) as usize;
                        if func_off + 48 <= decomp2.len() {
                            let orig_disk_sz = u32::from_le_bytes(decomp2[func_off + 44..func_off + 48].try_into().unwrap()) as usize;
                            if orig_disk_sz >= 24 && func_off + 48 + orig_disk_sz <= decomp2.len() {
                                let ml_idx = exp_ml.idx as i32;
                                let new_product_ids_idx = exports.iter()
                                    .find(|e| e.name == "NewProductIDs" && e.outer_idx == ml_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(59196);
                                let loadout_product_ids_idx = exports.iter()
                                    .find(|e| e.name == "LoadoutProductIDs" && e.outer_idx == ml_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(59200);
                                let return_val_idx = exports.iter()
                                    .find(|e| e.name == "ReturnValue" && e.outer_idx == ml_idx)
                                    .map(|e| e.idx as i32)
                                    .unwrap_or(59197);

                                let (payload, mem_sz) = emit_modify_loadout_passthrough_bytecode(
                                    new_product_ids_idx,
                                    loadout_product_ids_idx,
                                    return_val_idx,
                                    orig_disk_sz,
                                );
                                decomp2[func_off + 40..func_off + 44].copy_from_slice(&mem_sz.to_le_bytes());
                                decomp2[func_off + 48..func_off + 48 + orig_disk_sz].copy_from_slice(&payload);
                            }
                        }
                    }
                }

                let recomp2 = crate::upk::compression::compress_chunk(&decomp2).expect("Recompression of Chunk 2 must succeed");
                assert!(recomp2.len() <= chunks[2].comp_size as usize, "Recompressed Chunk 2 must fit within allocated size");
            }
        }
    }

    #[test]
    fn test_apply_modifications_passes_validate_tagame_tool() {
        let tagame_path = if Path::new("/root/TAGame_clean.upk").exists() {
            Path::new("/root/TAGame_clean.upk")
        } else if Path::new("/root/TAGame.upk.bak").exists() {
            Path::new("/root/TAGame.upk.bak")
        } else if Path::new("/tmp/test_TAGame.upk").exists() {
            Path::new("/tmp/test_TAGame.upk")
        } else {
            return;
        };

        let temp_dir = std::env::temp_dir().join(format!("vrl_test_val_{}", std::process::id()));
        let cooked_dir = temp_dir.join("CookedPCConsole");
        let _ = fs::create_dir_all(&cooked_dir);
        let target = cooked_dir.join("TAGame.upk");
        fs::copy(tagame_path, &target).expect("copy tagame upk");

        let items = vec![
            TagameSwapItem {
                slot: "Body".to_string(),
                slot_index: Some(0),
                owned_id: Some(23),
                product_id: 11603,
                paint_id: Some(1),
                custom_paint_hex: None,
                package_name: Some("Body_Fennec_SF".to_string()),
            },
            TagameSwapItem {
                slot: "Rocket Boost".to_string(),
                slot_index: None,
                owned_id: Some(33),
                product_id: 12968,
                paint_id: None,
                custom_paint_hex: None,
                package_name: Some("boost_alphadevreward".to_string()),
            },
            TagameSwapItem {
                slot: "Goal Explosion".to_string(),
                slot_index: Some(15),
                owned_id: Some(2044),
                product_id: 8840,
                paint_id: None,
                custom_paint_hex: None,
                package_name: Some("GoalExplosion_BubbleGum".to_string()),
            },
        ];

        let keys_txt = include_str!("../../resources/keys.txt");
        let keys_map_json = include_str!("../../resources/keys_map.json");

        let res = apply_tagame_modifications(
            &cooked_dir,
            &items,
            keys_txt,
            keys_map_json,
            Some(false),
        );
        assert!(res.is_ok(), "apply_tagame_modifications failed: {:?}", res.err());

        let output = std::process::Command::new("python3")
            .arg("/root/VelocityRL/src-tauri/tools/validate_tagame.py")
            .arg(&target)
            .output()
            .expect("execute validate_tagame.py");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "validate_tagame.py failed with exit code {:?}\nSTDOUT:\n{}\nSTDERR:\n{}",
            output.status.code(),
            stdout,
            stderr
        );

        let emu_output = std::process::Command::new("python3")
            .arg("/root/VelocityRL/src-tauri/tools/ue3_emulator.py")
            .arg(&target)
            .output()
            .expect("execute ue3_emulator.py");

        let emu_stdout = String::from_utf8_lossy(&emu_output.stdout);
        let emu_stderr = String::from_utf8_lossy(&emu_output.stderr);
        println!("EMULATOR OUTPUT:\n{}", emu_stdout);
        assert!(
            emu_output.status.success(),
            "ue3_emulator.py detected crash/invalid bytecode with exit code {:?}\nSTDOUT:\n{}\nSTDERR:\n{}",
            emu_output.status.code(),
            emu_stdout,
            emu_stderr
        );

        let _ = fs::copy(&target, "/tmp/modified_TAGame.upk");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_ensure_projectx_patched() {
        let src_px = if Path::new("/root/ProjectX.upk").is_file() {
            Path::new("/root/ProjectX.upk")
        } else if Path::new("/root/velrlapi/games/rocketleague/TAGame/CookedPCConsole/ProjectX.upk").is_file() {
            Path::new("/root/velrlapi/games/rocketleague/TAGame/CookedPCConsole/ProjectX.upk")
        } else if Path::new("/tmp/test_ProjectX.upk").is_file() {
            Path::new("/tmp/test_ProjectX.upk")
        } else {
            return;
        };

        let temp_dir = std::env::temp_dir().join(format!("vrl_test_px_patch_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let target_px = temp_dir.join("ProjectX.upk");
        fs::copy(src_px, &target_px).expect("copy ProjectX");

        let res1 = ensure_projectx_patched(&temp_dir);
        assert!(res1.is_ok(), "ensure_projectx_patched failed: {:?}", res1.err());

        // Second run should return Ok(false) (already patched)
        let res2 = ensure_projectx_patched(&temp_dir);
        assert!(res2.is_ok());
        assert_eq!(res2.unwrap(), false, "Second run should detect already patched");

        // Verify the backup file was created
        let bak_path = target_px.with_extension("upk.bak");
        assert!(bak_path.is_file(), "Backup ProjectX.upk.bak must exist");

        // Run UE3 emulator against the patched ProjectX.upk to verify 100% clean execution
        let emu_res = std::process::Command::new("python3")
            .args(&["/root/VelocityRL/src-tauri/tools/ue3_emulator.py", &target_px.to_string_lossy()])
            .output();
        if let Ok(out) = emu_res {
            assert!(out.status.success(), "ProjectX emulator validation failed: STDOUT: {}\nSTDERR: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr));
        }

        let _ = fs::copy(&target_px, "/tmp/patched_ProjectX.upk");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}


#[derive(Clone)]
struct PatchChunk {
    uncomp_offset: i64,
    uncomp_size: i32,
    comp_offset: u64,
    comp_size: i32,
}

#[derive(Clone)]
struct PatchExport {
    outer_idx: i32,
    name_idx: i32,
    header_pos: usize,
    serial_size: i32,
    serial_offset: i64,
}

fn read_u32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b.get(p..p + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]))
}
fn read_i32(b: &[u8], p: usize) -> i32 {
    i32::from_le_bytes(b.get(p..p + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]))
}
fn read_i64(b: &[u8], p: usize) -> i64 {
    i64::from_le_bytes(b.get(p..p + 8).and_then(|s| s.try_into().ok()).unwrap_or([0; 8]))
}
fn read_u64(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b.get(p..p + 8).and_then(|s| s.try_into().ok()).unwrap_or([0; 8]))
}

/// Apply the bundled bytecode patch records to a stock TAGame.upk in place.
/// No file replacement: chunks are grown and exports repointed, exactly like
/// the proven append technique used by the avatar patch.
pub fn patch_tagame_in_place(path: &Path, backup: &Path, patch_bytes: &[u8]) -> Result<usize, String> {
    let file_bytes = if backup.is_file() {
        fs::read(backup).map_err(|e| e.to_string())?
    } else {
        fs::read(path).map_err(|e| e.to_string())?
    };
    let total_hdr = read_u32(&file_bytes, 8) as usize;
    let flen = read_i32(&file_bytes, 12);
    let prefix_sz = 16 + (if flen > 0 { flen as usize } else { (-flen * 2) as usize });
    let name_count = read_i32(&file_bytes, prefix_sz + 4);
    let name_offset = read_u32(&file_bytes, prefix_sz + 8) as usize;
    let p_sum = prefix_sz + 12;
    let export_count = read_u32(&file_bytes, p_sum) as usize;
    let export_offset = read_u32(&file_bytes, p_sum + 4) as usize;
    let depends_offset = read_u32(&file_bytes, p_sum + 16) as usize;

    let enc_size = (total_hdr.saturating_sub(name_offset) + 15) & !15;
    let enc_end = name_offset + enc_size;
    if enc_end > file_bytes.len() || depends_offset < name_offset {
        return Err("invalid TAGame header".into());
    }
    let mut plain_header = crypto::decrypt_ecb(&TAGAME_KEY, &file_bytes[name_offset..enc_end]);
    let names = crate::upk::palette::parse_names_in_block(&plain_header, name_count)
        .map_err(|e| e.to_string())?;

    let export_rel = export_offset - name_offset;
    let depends_rel = depends_offset - name_offset;
    let mut exports: Vec<PatchExport> = Vec::new();
    let mut pos = export_rel;
    while pos + 72 <= depends_rel && pos + 72 <= plain_header.len() && exports.len() < export_count {
        let noc = read_i32(&plain_header, pos + 48).max(0) as usize;
        exports.push(PatchExport {
            outer_idx: read_i32(&plain_header, pos + 8),
            name_idx: read_i32(&plain_header, pos + 12),
            header_pos: pos,
            serial_size: read_i32(&plain_header, pos + 32),
            serial_offset: read_i64(&plain_header, pos + 36),
        });
        pos += 72 + noc * 4;
    }

    let chunks_rel = depends_offset - name_offset;
    let num_chunks = read_u32(&plain_header, chunks_rel) as usize;
    let mut chunks: Vec<PatchChunk> = Vec::new();
    for i in 0..num_chunks {
        let p = chunks_rel + 4 + i * 36;
        if p + 24 > plain_header.len() { break; }
        chunks.push(PatchChunk {
            uncomp_offset: read_i64(&plain_header, p),
            uncomp_size: read_i32(&plain_header, p + 8),
            comp_offset: read_u64(&plain_header, p + 12),
            comp_size: read_i32(&plain_header, p + 20),
        });
    }
    let mut buffers: Vec<Vec<u8>> = Vec::with_capacity(chunks.len());
    for c in &chunks {
        let start = c.comp_offset as usize;
        let end = start + c.comp_size.max(0) as usize;
        if end > file_bytes.len() {
            return Err("chunk offset out of bounds".into());
        }
        buffers.push(crate::upk::compression::decompress_chunk(&file_bytes[start..end])
            .map_err(|e| e.to_string())?);
    }

    let mut records: Vec<(String, String, Vec<u8>)> = Vec::new();
    let mut rp = 0usize;
    let record_count = read_u32(patch_bytes, rp) as usize;
    rp += 4;
    for _ in 0..record_count {
        let mut parts = Vec::new();
        for _ in 0..2 {
            let len = u16::from_le_bytes(patch_bytes.get(rp..rp + 2).and_then(|s| s.try_into().ok()).unwrap_or([0; 2])) as usize;
            rp += 2;
            parts.push(String::from_utf8_lossy(patch_bytes.get(rp..rp + len).unwrap_or(&[])).into_owned());
            rp += len;
        }
        let len = read_u32(patch_bytes, rp) as usize;
        rp += 4;
        let serial = patch_bytes.get(rp..rp + len).ok_or("truncated patch payload")?.to_vec();
        rp += len;
        records.push((parts[0].clone(), parts[1].clone(), serial));
    }

    let name_of = |idx: i32| -> String {
        if idx >= 0 && (idx as usize) < names.len() { names[idx as usize].clone() } else { String::new() }
    };
    let mut applied = 0;
    for (outer, name, serial) in &records {
        let found = exports.iter().position(|e| {
            name_of(e.name_idx) == *name
                && e.outer_idx > 0
                && (e.outer_idx as usize) <= exports.len()
                && name_of(exports[e.outer_idx as usize - 1].name_idx) == *outer
        });
        let Some(ei) = found else {
            crate::applog::event(&format!("patch: export {outer}::{name} not found"));
            continue;
        };
        let exp = exports[ei].clone();
        let ci = chunks.iter().position(|c| {
            exp.serial_offset >= c.uncomp_offset && exp.serial_offset < c.uncomp_offset + c.uncomp_size as i64
        });
        let Some(ci) = ci else { continue };
        let old_end = chunks[ci].uncomp_offset + buffers[ci].len() as i64;
        let delta = serial.len() as i64;
        buffers[ci].extend_from_slice(serial);
        for other in &mut exports {
            if other.serial_offset >= old_end { other.serial_offset += delta; }
        }
        for later in &mut chunks[ci + 1..] { later.uncomp_offset += delta; }
        exports[ei].serial_size = serial.len() as i32;
        exports[ei].serial_offset = old_end;
        chunks[ci].uncomp_size += delta as i32;
        applied += 1;
    }
    if applied == 0 {
        return Ok(0);
    }

    let mut compressed = Vec::with_capacity(chunks.len());
    for buf in &buffers {
        let c = crate::upk::compression::compress_chunk(buf).map_err(|e| e.to_string())?;
        compressed.push(c);
    }
    let mut cur = total_hdr as u64;
    for (i, c) in chunks.iter_mut().enumerate() {
        c.comp_offset = cur;
        c.comp_size = compressed[i].len() as i32;
        cur += compressed[i].len() as u64;
    }
    for e in &exports {
        let p = e.header_pos;
        if p + 44 <= plain_header.len() {
            plain_header[p + 32..p + 36].copy_from_slice(&e.serial_size.to_le_bytes());
            plain_header[p + 36..p + 44].copy_from_slice(&e.serial_offset.to_le_bytes());
        }
    }
    let c_p = chunks_rel + 4;
    for (i, ch) in chunks.iter().enumerate() {
        let p = c_p + i * 36;
        if p + 24 <= plain_header.len() {
            plain_header[p..p + 8].copy_from_slice(&ch.uncomp_offset.to_le_bytes());
            plain_header[p + 8..p + 12].copy_from_slice(&ch.uncomp_size.to_le_bytes());
            plain_header[p + 12..p + 20].copy_from_slice(&ch.comp_offset.to_le_bytes());
            plain_header[p + 20..p + 24].copy_from_slice(&ch.comp_size.to_le_bytes());
        }
    }
    let padded = (plain_header.len() + 15) & !15;
    plain_header.resize(padded, 0);
    let encrypted = crypto::encrypt_ecb(&TAGAME_KEY, &plain_header);

    let tmp = path.with_extension("upk.rlim.tmp");
    let mut out = Vec::new();
    out.extend_from_slice(&file_bytes[..name_offset]);
    out.extend_from_slice(&encrypted[..total_hdr - name_offset]);
    for c in &compressed { out.extend_from_slice(c); }
    fs::write(&tmp, &out).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(applied)
}

#[cfg(test)]
mod rlim_patch_tests {
    use super::*;

    #[test]
    fn test_patch_tagame_in_place() {
        let stock = Path::new("/root/velrlapi/.cache/TAGame.upk.bak");
        if !stock.is_file() {
            return;
        }
        let tmp = std::env::temp_dir().join("vrl_tagame_patch_test.upk");
        let _ = fs::copy(stock, &tmp);
        let payload = include_bytes!("../../resources/rl_patch.bin");
        let applied = patch_tagame_in_place(&tmp, stock, payload).expect("in-place patch must succeed");
        assert!(applied > 0, "at least one export must be patched");
        let bytes = fs::read(&tmp).expect("patched file must be readable");
        assert!(tagame_has_rlim_patch(&bytes), "patched file must contain the loadout hook marker");
        let _ = fs::remove_file(&tmp);
    }
}
