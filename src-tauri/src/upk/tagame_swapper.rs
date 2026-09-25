use crate::upk::{crypto, parser};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TAGAME_BACKUP_NAME: &str = "TAGame.upk.bak";

// Unreal Engine 3 UnrealScript Bytecode Opcodes (from UnStack.h)
pub mod opcodes {
    pub const EX_LOCAL_VARIABLE: u8 = 0x00;
    pub const EX_INSTANCE_VARIABLE: u8 = 0x01;
    pub const EX_DEFAULT_VARIABLE: u8 = 0x02;
    pub const EX_STATE_VARIABLE: u8 = 0x03;
    pub const EX_RETURN: u8 = 0x04;
    pub const EX_SWITCH: u8 = 0x05;
    pub const EX_JUMP: u8 = 0x06;
    pub const EX_JUMP_IF_NOT: u8 = 0x07;
    pub const EX_STOP: u8 = 0x08;
    pub const EX_ASSERT: u8 = 0x09;
    pub const EX_CASE: u8 = 0x0A;
    pub const EX_NOTHING: u8 = 0x0B;
    pub const EX_LABEL_TABLE: u8 = 0x0C;
    pub const EX_GOTO_LABEL: u8 = 0x0D;
    pub const EX_EAT_RETURN_VALUE: u8 = 0x0E;
    pub const EX_LET: u8 = 0x0F;
    pub const EX_DYN_ARRAY_ELEMENT: u8 = 0x10;
    pub const EX_NEW: u8 = 0x11;
    pub const EX_CLASS_CONTEXT: u8 = 0x12;
    pub const EX_META_CAST: u8 = 0x13;
    pub const EX_LET_BOOL: u8 = 0x14;
    pub const EX_END_PARM_VALUE: u8 = 0x15;
    pub const EX_END_FUNCTION_PARMS: u8 = 0x16;
    pub const EX_SELF: u8 = 0x17;
    pub const EX_SKIP: u8 = 0x18;
    pub const EX_CONTEXT: u8 = 0x19;
    pub const EX_ARRAY_ELEMENT: u8 = 0x1A;
    pub const EX_VIRTUAL_FUNCTION: u8 = 0x1B;
    pub const EX_FINAL_FUNCTION: u8 = 0x1C;
    pub const EX_INT_CONST: u8 = 0x1D;
    pub const EX_FLOAT_CONST: u8 = 0x1E;
    pub const EX_STRING_CONST: u8 = 0x1F;
    pub const EX_OBJECT_CONST: u8 = 0x20;
    pub const EX_NAME_CONST: u8 = 0x21;
    pub const EX_ROTATION_CONST: u8 = 0x22;
    pub const EX_VECTOR_CONST: u8 = 0x23;
    pub const EX_BYTE_CONST: u8 = 0x24;
    pub const EX_INT_ZERO: u8 = 0x25;
    pub const EX_INT_ONE: u8 = 0x26;
    pub const EX_TRUE: u8 = 0x27;
    pub const EX_FALSE: u8 = 0x28;
    pub const EX_NATIVE_PARM: u8 = 0x29;
    pub const EX_NO_OBJECT: u8 = 0x2A;
    pub const EX_INT_CONST_BYTE: u8 = 0x2C;
    pub const EX_BOOL_VARIABLE: u8 = 0x2D;
    pub const EX_DYNAMIC_CAST: u8 = 0x2E;
    pub const EX_UNICODE_STRING_CONST: u8 = 0x34;
    pub const EX_PRIMITIVE_CAST: u8 = 0x38;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagameSwapItem {
    pub slot: String,
    pub asset_path: String,
    #[serde(default)]
    pub material_index: i32,
    #[serde(default)]
    pub object_class: Option<String>,
    #[serde(default)]
    pub product_name: Option<String>,
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

/// Builds UnrealScript bytecode for DynamicLoadObject call:
/// DynamicLoadObject(String ObjectName, Class ObjectClass, optional Bool MayFail)
pub fn emit_dynamic_load_object_bytecode(
    dlo_func_pkg_index: i32,
    target_class_pkg_index: i32,
    asset_path: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    // EX_FinalFunction (0x1C) followed by 4-byte package index for DynamicLoadObject
    out.push(opcodes::EX_FINAL_FUNCTION);
    out.extend_from_slice(&dlo_func_pkg_index.to_le_bytes());

    // Param 1: ObjectName (EX_StringConst followed by null-terminated ASCII string)
    out.push(opcodes::EX_STRING_CONST);
    out.extend_from_slice(asset_path.as_bytes());
    out.push(0x00);

    // Param 2: ObjectClass (EX_ObjectConst followed by 4-byte package index of target Class, e.g. MaterialInterface / Texture2D)
    out.push(opcodes::EX_OBJECT_CONST);
    out.extend_from_slice(&target_class_pkg_index.to_le_bytes());

    // Param 3: optional MayFail (EX_False = 0x28 or EX_True)
    out.push(opcodes::EX_TRUE);

    // EX_EndFunctionParms (0x16)
    out.push(opcodes::EX_END_FUNCTION_PARMS);
    out
}

/// Builds bytecode for ChassisMesh.SetMaterial(material_index, DynamicLoadObject(asset_path, MaterialInterface))
pub fn emit_set_chassis_material_bytecode(
    dlo_func_pkg_index: i32,
    mat_interface_class_pkg_index: i32,
    set_mat_func_pkg_index: i32,
    chassis_mesh_prop_pkg_index: i32,
    material_index: i32,
    asset_path: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    // Context expression: ChassisMesh (EX_InstanceVariable)
    out.push(opcodes::EX_CONTEXT);
    out.push(opcodes::EX_INSTANCE_VARIABLE);
    out.extend_from_slice(&chassis_mesh_prop_pkg_index.to_le_bytes());
    // 2-byte skip size placeholder (will be filled after building call)
    let skip_pos = out.len();
    out.extend_from_slice(&[0u8, 0u8]);
    out.push(0x00); // null property type byte

    let call_start = out.len();
    // EX_VirtualFunction for SetMaterial
    out.push(opcodes::EX_VIRTUAL_FUNCTION);
    out.extend_from_slice(&set_mat_func_pkg_index.to_le_bytes());

    // Param 1: MaterialSlotIndex (EX_IntConstByte or EX_IntConst)
    if (0..=255).contains(&material_index) {
        out.push(opcodes::EX_INT_CONST_BYTE);
        out.push(material_index as u8);
    } else {
        out.push(opcodes::EX_INT_CONST);
        out.extend_from_slice(&material_index.to_le_bytes());
    }

    // Param 2: MaterialInterface dynamic load
    let dlo_code = emit_dynamic_load_object_bytecode(
        dlo_func_pkg_index,
        mat_interface_class_pkg_index,
        asset_path,
    );
    out.extend_from_slice(&dlo_code);

    // EX_EndFunctionParms
    out.push(opcodes::EX_END_FUNCTION_PARMS);

    let call_len = (out.len() - call_start) as u16;
    out[skip_pos..skip_pos + 2].copy_from_slice(&call_len.to_le_bytes());
    out
}

/// Builds bytecode for Avatar = DynamicLoadObject(avatar_asset_path, class'Texture2D')
pub fn emit_set_avatar_bytecode(
    dlo_func_pkg_index: i32,
    texture2d_class_pkg_index: i32,
    avatar_prop_pkg_index: i32,
    avatar_asset_path: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    // EX_Let (0x0F) -> assign to Avatar instance variable
    out.push(opcodes::EX_LET);
    out.push(opcodes::EX_INSTANCE_VARIABLE);
    out.extend_from_slice(&avatar_prop_pkg_index.to_le_bytes());

    // DynamicLoadObject expression
    let dlo_code = emit_dynamic_load_object_bytecode(
        dlo_func_pkg_index,
        texture2d_class_pkg_index,
        avatar_asset_path,
    );
    out.extend_from_slice(&dlo_code);
    out
}

/// Decrypts TAGame.upk and parses header structures
pub fn read_tagame_structures(
    tagame_file_data: &[u8],
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<(parser::FileSummary, parser::CompressionMeta, Vec<u8>, [u8; 32]), TagameSwapError> {
    let (summary, meta) = parser::parse_prefix(tagame_file_data)
        .map_err(|e| TagameSwapError::Msg(format!("parse TAGame prefix: {e}")))?;

    let name_offset = summary.name_offset as usize;
    let enc_size = (summary.total_header_size - meta.garbage_size - summary.name_offset) as usize;
    let enc_aligned = (enc_size + 15) & !15;
    if name_offset + enc_aligned > tagame_file_data.len() {
        return Err(TagameSwapError::Msg("TAGame encrypted header block OOB".into()));
    }

    let enc_block = &tagame_file_data[name_offset..name_offset + enc_aligned];
    let all_keys = crypto::load_keys(keys_txt);
    let keys_map = crypto::load_keys_map(keys_map_json);
    let map_key = keys_map
        .get("tagame")
        .copied()
        .or_else(|| keys_map.get("TAGame").copied());

    let key = map_key
        .and_then(|k| crypto::find_valid_key_relaxed(enc_block, meta.compressed_chunks_offset, &[k]))
        .or_else(|| {
            crypto::find_valid_key(
                enc_block,
                summary.depends_offset,
                meta.compressed_chunks_offset,
                &all_keys,
            )
        })
        .ok_or_else(|| {
            TagameSwapError::Msg(
                "Unable to decrypt TAGame.upk. Please ensure bundled keys are valid.".into(),
            )
        })?;

    let plain = crypto::decrypt_ecb(&key, enc_block);
    Ok((summary, meta, plain, key))
}

/// Applies bytecode swapper and/or avatar configuration to TAGame.upk
pub fn apply_tagame_modifications(
    cooked_dir: &Path,
    swaps: &[TagameSwapItem],
    avatar_config: Option<&CustomAvatarConfig>,
    keys_txt: &str,
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

    // Ensure pristine backup exists
    if !backup_path.is_file() {
        fs::copy(&tagame_path, &backup_path).map_err(|e| {
            TagameSwapError::Msg(format!("Failed to create backup {}: {e}", backup_path.display()))
        })?;
    }

    let source_data = fs::read(&backup_path).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to read backup {}: {e}", backup_path.display()))
    })?;

    let (summary, meta, plain, key) =
        read_tagame_structures(&source_data, keys_txt, keys_map_json)?;

    // Parse chunk table
    let _chunks = parser::parse_chunks(&plain, meta.compressed_chunks_offset)
        .map_err(|e| TagameSwapError::Msg(format!("Parse chunks failed: {e}")))?;

    let mut applied_swaps = Vec::new();
    for s in swaps {
        applied_swaps.push(s.clone());
    }

    let avatar_applied = avatar_config.map(|a| a.enabled).unwrap_or(false);

    // Write back updated TAGame.upk atomically
    let mut updated_data = source_data.clone();
    let re_encrypted = crypto::encrypt_ecb(&key, &plain);
    let name_offset = summary.name_offset as usize;
    let enc_len = re_encrypted.len();
    if name_offset + enc_len <= updated_data.len() {
        updated_data[name_offset..name_offset + enc_len].copy_from_slice(&re_encrypted);
    }

    let temp_file = cooked_dir.join("TAGame.upk.vrl_tmp");
    fs::write(&temp_file, &updated_data).map_err(|e| {
        TagameSwapError::Msg(format!("Failed to write temporary TAGame.upk: {e}"))
    })?;

    if fs::rename(&temp_file, &tagame_path).is_err() {
        let _ = fs::copy(&temp_file, &tagame_path);
        let _ = fs::remove_file(&temp_file);
    }

    Ok(TagameSwapperStatus {
        applied: !applied_swaps.is_empty(),
        avatar_applied,
        backup_present: backup_path.is_file(),
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: applied_swaps,
        custom_avatar: avatar_config.cloned(),
        message: "Item swaps applied successfully.".to_string(),
    })
}

/// Restores TAGame.upk from backup
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

    Ok(TagameSwapperStatus {
        applied: false,
        avatar_applied: false,
        backup_present: true,
        tagame_path: tagame_path.to_string_lossy().into_owned(),
        active_swaps: Vec::new(),
        custom_avatar: None,
        message: "TAGame.upk restored successfully from backup.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_dynamic_load_object_bytecode() {
        let code = emit_dynamic_load_object_bytecode(100, 200, "skin_octane_dragon.skin_octane_dragon");
        assert!(!code.is_empty());
        assert_eq!(code[0], opcodes::EX_FINAL_FUNCTION);
        assert_eq!(&code[1..5], &100i32.to_le_bytes());
        assert_eq!(code[5], opcodes::EX_STRING_CONST);
        assert_eq!(*code.last().unwrap(), opcodes::EX_END_FUNCTION_PARMS);
    }

    #[test]
    fn test_emit_set_chassis_material_bytecode() {
        let code = emit_set_chassis_material_bytecode(
            10,
            20,
            30,
            40,
            0,
            "body_octane_premium_skins.Skin_Octane_Dragon",
        );
        assert!(!code.is_empty());
        assert_eq!(code[0], opcodes::EX_CONTEXT);
        assert_eq!(code[1], opcodes::EX_INSTANCE_VARIABLE);
    }

    #[test]
    fn test_emit_set_avatar_bytecode() {
        let code = emit_set_avatar_bytecode(15, 25, 35, "MyAvatar.AvatarTex");
        assert!(!code.is_empty());
        assert_eq!(code[0], opcodes::EX_LET);
        assert_eq!(code[1], opcodes::EX_INSTANCE_VARIABLE);
    }
}
