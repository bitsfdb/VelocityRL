use crate::upk::{crypto, nametable, parser};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PAINT_NAMES: [&str; 13] = [
    "None",
    "Crimson",
    "Lime",
    "Black",
    "Orange",
    "Sky Blue",
    "Cobalt",
    "Saffron",
    "Grey",
    "Pink",
    "Forest Green",
    "Purple",
    "Titanium White",
];

const MAX_UPK_BYTES: u64 = 256 * 1024 * 1024;
const MAX_NAME_COUNT: i32 = 50_000;

fn patch_i32_le(data: &mut [u8], offset: usize, value: i32) {
    if offset + 4 <= data.len() {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn package_base(stem: &str) -> &str {
    stem.trim_end_matches("_sf")
        .trim_end_matches("_SF")
        .trim_end_matches("_Sf")
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Item {
    #[serde(alias = "ID", alias = "id")]
    pub id: i64,
    #[serde(alias = "Product", alias = "label", alias = "long_label", default)]
    pub product: String,
    #[serde(alias = "Slot", alias = "slot", default)]
    pub slot: String,
    #[serde(alias = "AssetPackage", alias = "asset_package", default)]
    pub asset_package: String,
    #[serde(alias = "AssetPath", alias = "asset_path", default)]
    pub asset_path: String,
}

pub struct SwapOptions {
    pub game_dir: PathBuf,
    pub items_json: String,
    pub keys_txt: String,
    pub keys_map_json: String,
}

#[derive(Debug)]
pub enum SwapError {
    Collision(String),
    AlreadySwapped(String),
    Io(std::io::Error),
    Msg(String),
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwapError::Collision(s) | SwapError::AlreadySwapped(s) | SwapError::Msg(s) => {
                write!(f, "{}", s)
            }
            SwapError::Io(e) => write!(f, "{}", explain_io(e)),
        }
    }
}

fn explain_io(e: &std::io::Error) -> String {
    let code = e.raw_os_error();
    if code == Some(32) || code == Some(33) {
        return format!(
            "IO error: {e} — a UPK is locked. Close Rocket League and the Epic launcher, then retry."
        );
    }
    format!("IO error: {e}")
}

impl From<std::io::Error> for SwapError {
    fn from(e: std::io::Error) -> Self {
        SwapError::Io(e)
    }
}
impl From<String> for SwapError {
    fn from(s: String) -> Self {
        SwapError::Msg(s)
    }
}

pub fn paint_label(id: i32) -> &'static str {
    PAINT_NAMES
        .get(id as usize)
        .copied()
        .unwrap_or("Unknown paint")
}

pub fn paint_slugs(id: i32) -> Vec<String> {
    if !(1..=12).contains(&id) {
        return Vec::new();
    }
    let name = paint_label(id);
    let nospace = name.replace(' ', "");
    let underscored = name.replace(' ', "_");
    let mut slugs = vec![
        nospace,
        underscored,
        name.to_string(),
        format!("P{id}"),
        format!("P{id:02}"),
        format!("{id}"),
    ];
    match id {
        1 => slugs.push("C".into()),
        2 => slugs.push("L".into()),
        3 => slugs.push("B".into()),
        4 => slugs.push("O".into()),
        5 => {
            slugs.push("S".into());
            slugs.push("SB".into());
            slugs.push("Sky_Blue".into());
        }
        6 => slugs.push("K".into()),
        7 => slugs.push("Y".into()),
        8 => {
            slugs.push("G".into());
            slugs.push("Gray".into());
        }
        9 => slugs.push("P".into()),
        10 => {
            slugs.push("F".into());
            slugs.push("FG".into());
            slugs.push("Forest_Green".into());
        }
        11 => slugs.push("V".into()),
        12 => {
            slugs.push("TW".into());
            slugs.push("Titanium_White".into());
        }
        _ => {}
    }
    slugs
}

fn is_thumbnail_companion_upk(filename: &str) -> bool {

    file_stem(filename)
        .to_ascii_lowercase()
        .ends_with("_t_sf")
}

pub fn painted_package_candidates(asset_package: &str, paint_id: i32) -> Vec<String> {
    let stem = file_stem(asset_package);
    if stem.is_empty() {
        return Vec::new();
    }
    let base = package_base(&stem);
    let mut out = Vec::new();
    for slug in paint_slugs(paint_id) {
        out.push(format!("{base}_{slug}_SF.upk"));
        out.push(format!("{stem}_{slug}.upk"));
        out.push(format!("{base}_{slug}.upk"));
    }
    out
}

/// Resolves a raw package string (e.g. "Wheel_SoccerBall", "Wheel_SoccerBall.upk", "Wheel_SoccerBall_SF.upk")
/// to the actual file path and canonical file name inside CookedPCConsole.
/// Handles missing .upk extensions, missing _SF companions, and case differences on disk.
pub fn resolve_package_path(game_dir: &Path, raw_pkg: &str) -> Option<(PathBuf, String)> {
    let trimmed = raw_pkg.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut cands = Vec::new();
    cands.push(trimmed.to_string());

    let has_upk = trimmed.to_ascii_lowercase().ends_with(".upk");
    let stem = if has_upk {
        file_stem(trimmed)
    } else {
        trimmed.to_string()
    };

    if !has_upk {
        cands.push(format!("{stem}.upk"));
    }

    let stem_lower = stem.to_ascii_lowercase();
    if !stem_lower.ends_with("_sf") {
        cands.push(format!("{stem}_SF.upk"));
        cands.push(format!("{stem}_sf.upk"));
    } else {
        let base = package_base(&stem);
        if !base.is_empty() {
            cands.push(format!("{base}.upk"));
        }
    }

    // 1. Direct filesystem check
    for cand in &cands {
        let p = game_dir.join(cand);
        if p.is_file() {
            return Some((p, cand.clone()));
        }
    }

    // 2. Case-insensitive filesystem check in game_dir
    if let Ok(entries) = std::fs::read_dir(game_dir) {
        let cand_lowers: Vec<String> = cands.iter().map(|c| c.to_ascii_lowercase()).collect();
        for entry in entries.flatten() {
            let actual_name = entry.file_name().to_string_lossy().into_owned();
            let actual_lower = actual_name.to_ascii_lowercase();
            if cand_lowers.iter().any(|c| c == &actual_lower) {
                let p = entry.path();
                if p.is_file() {
                    return Some((p, actual_name));
                }
            }
        }
    }

    None
}

fn find_painted_package(game_dir: &Path, asset_package: &str, paint_id: i32) -> Option<String> {
    for cand in painted_package_candidates(asset_package, paint_id) {
        if is_thumbnail_companion_upk(&cand) {
            continue;
        }
        if let Some((_, resolved)) = resolve_package_path(game_dir, &cand) {
            return Some(resolved);
        }
    }
    None
}

fn suffix_asset_path(path: &str, slug: &str) -> String {
    path.split('.')
        .filter(|s| !s.is_empty())
        .map(|part| format!("{part}_{slug}"))
        .collect::<Vec<_>>()
        .join(".")
}

fn add_pair(pairs: &mut Vec<(String, String)>, old: String, new: String) {
    if old.is_empty() || new.is_empty() || old == new {
        return;
    }
    if !pairs.contains(&(old.clone(), new.clone())) {
        pairs.push((old, new));
    }
}

fn load_items(json: &str) -> Result<Vec<Item>, SwapError> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| SwapError::Msg(format!("items.json is not valid JSON: {e}")))?;
    let arr = v
        .get("Items")
        .or_else(|| v.get("items"))
        .and_then(|a| a.as_array())
        .cloned()
        .or_else(|| v.as_array().cloned())
        .ok_or_else(|| SwapError::Msg("items.json has no Items array".into()))?;
    let items: Vec<Item> = arr
        .iter()
        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
        .filter(|i: &Item| !i.asset_package.is_empty())
        .collect();
    if items.is_empty() {
        return Err(SwapError::Msg(
            "items.json has no usable items (every entry needs AssetPackage)".into(),
        ));
    }
    Ok(items)
}

fn find_item_by_id(items: &[Item], id: i64) -> Option<&Item> {
    items.iter().find(|i| i.id == id)
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join("_")
}

fn infer_name_pairs(target: &Item, donor: &Item) -> Vec<(String, String)> {
    let donor_stem = file_stem(&donor.asset_package);
    let target_stem = file_stem(&target.asset_package);
    let donor_base = package_base(&donor_stem);
    let target_base = package_base(&target_stem);

    let mut donor_path_synthesized = donor.asset_path.clone();
    if donor_path_synthesized.is_empty() && !donor_base.is_empty() {
        donor_path_synthesized = format!("{donor_base}.{donor_base}");
    }
    let mut target_path_synthesized = target.asset_path.clone();
    if target_path_synthesized.is_empty() && !target_base.is_empty() {
        target_path_synthesized = format!("{target_base}.{target_base}");
    }

    let donor_parts: Vec<&str> = donor_path_synthesized
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();
    let target_parts: Vec<&str> = target_path_synthesized
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();

    let mut pairs: Vec<(String, String)> = Vec::new();

    if !donor_parts.is_empty() && !target_parts.is_empty() {
        let donor_obj = donor_parts.last().unwrap().to_string();
        let target_obj = target_parts.last().unwrap().to_string();
        add_pair(&mut pairs, donor_obj.clone(), target_obj.clone());
        add_pair(&mut pairs, format!("{donor_obj}_TA"), format!("{target_obj}_TA"));
        add_pair(&mut pairs, format!("{donor_obj}_archetype"), format!("{target_obj}_archetype"));

        add_pair(&mut pairs, donor_parts[0].to_string(), target_parts[0].to_string());
    }

    let len = donor_parts.len().min(target_parts.len());
    for i in 0..len {
        add_pair(
            &mut pairs,
            donor_parts[i].to_string(),
            target_parts[i].to_string(),
        );
    }

    if !donor_base.is_empty() && !target_base.is_empty() {
        let donor_pascal = to_pascal_case(donor_base);
        let target_pascal = to_pascal_case(target_base);

        // 1. Base package names (unadorned)
        add_pair(&mut pairs, donor_base.to_string(), target_base.to_string());
        add_pair(&mut pairs, donor_base.to_string(), target_pascal.clone());
        add_pair(&mut pairs, donor_pascal.clone(), target_base.to_string());
        add_pair(&mut pairs, donor_pascal.clone(), target_pascal.clone());

        // 2. Package file companions (_SF and _sf)
        add_pair(&mut pairs, format!("{donor_base}_SF"), format!("{target_base}_SF"));
        add_pair(&mut pairs, format!("{donor_base}_sf"), format!("{target_base}_sf"));
        add_pair(&mut pairs, format!("{donor_pascal}_SF"), format!("{target_pascal}_SF"));
        add_pair(&mut pairs, format!("{donor_pascal}_sf"), format!("{target_pascal}_sf"));

        // 3. Thumbnails (_Thumbnail)
        add_pair(&mut pairs, format!("{donor_base}_Thumbnail"), format!("{target_base}_Thumbnail"));
        add_pair(&mut pairs, format!("{donor_pascal}_Thumbnail"), format!("{target_pascal}_Thumbnail"));

        // 4. Material instances and archetypes
        add_pair(&mut pairs, format!("{donor_base}_TA"), format!("{target_pascal}_TA"));
        add_pair(&mut pairs, format!("{donor_pascal}_TA"), format!("{target_pascal}_TA"));
        add_pair(&mut pairs, format!("{donor_base}_archetype"), format!("{target_pascal}_archetype"));
        add_pair(&mut pairs, format!("{donor_pascal}_archetype"), format!("{target_pascal}_archetype"));
        add_pair(&mut pairs, format!("MIC_{donor_base}"), format!("MIC_{target_base}"));
        add_pair(&mut pairs, format!("MIC_{donor_pascal}"), format!("MIC_{target_pascal}"));
        add_pair(&mut pairs, format!("MIC_WHEEL_{donor_base}"), format!("MIC_WHEEL_{target_base}"));
        add_pair(&mut pairs, format!("MIC_WHEEL_{donor_pascal}"), format!("MIC_WHEEL_{target_pascal}"));
    }
    pairs
}

fn extend_paint_name_pairs(pairs: &mut Vec<(String, String)>, paint_id: i32) {
    let base = pairs.clone();
    for (old, new) in &base {
        add_pair(pairs, format!("{old}_Painted"), format!("{new}_Painted"));
        add_pair(pairs, format!("{old}_P"), format!("{new}_P"));
        for slug in paint_slugs(paint_id.max(0)) {
            add_pair(pairs, format!("{old}_{slug}"), format!("{new}_{slug}"));
        }
    }
}

fn calc_relative_offset(abs: i32, base: i32, label: &str) -> Result<usize, SwapError> {
    let d = abs
        .checked_sub(base)
        .ok_or_else(|| SwapError::Msg(format!("{label} offset overflow")))?;
    if d < 0 {
        return Err(SwapError::Msg(format!(
            "{label} ({abs}) is before the name table ({base})"
        )));
    }
    Ok(d as usize)
}

fn read_upk(path: &Path) -> Result<Vec<u8>, SwapError> {
    let meta = std::fs::metadata(path).map_err(|e| {
        SwapError::Msg(format!("cannot read {}: {}", path.display(), explain_io(&e)))
    })?;
    if meta.len() > MAX_UPK_BYTES {
        return Err(SwapError::Msg(format!(
            "{} is too large ({} bytes) to swap safely",
            path.display(),
            meta.len()
        )));
    }
    std::fs::read(path).map_err(|e| {
        SwapError::Msg(format!("cannot read {}: {}", path.display(), explain_io(&e)))
    })
}

fn name_table_has(header: &[u8], name_count: i32, needle: &str) -> bool {
    let count = name_count.clamp(0, MAX_NAME_COUNT);
    match parser::parse_name_table(header, 0, count) {
        Ok(names) => names.iter().any(|n| n.name.eq_ignore_ascii_case(needle)),
        Err(_) => false,
    }
}

pub fn read_package_guid(path: &Path) -> Option<[u8; 16]> {
    let data = read_upk(path).ok()?;
    let (summary, _) = parser::parse_prefix(&data).ok()?;
    Some(summary.guid)
}

pub fn dump_engine_info(game_dir: &Path) {
    let engine_path = game_dir.join("Engine.upk");
    match read_upk(&engine_path) {
        Ok(data) => match parser::parse_prefix(&data) {
            Ok((s, m)) => {
                let guid_hex: String = s.guid.iter().map(|b| format!("{:02X}", b)).collect();
                crate::applog::event(&format!(
                    "engine_dump: Engine.upk guid={} engine_version={} cooker_version={} header_size={} name_count={} import_count={} export_count={} garbage_size={}",
                    guid_hex, s.engine_version, s.cooker_version, s.total_header_size,
                    s.name_count, s.import_count, s.export_count, m.garbage_size,
                ));
            }
            Err(e) => crate::applog::event(&format!("engine_dump: failed to parse Engine.upk: {e}")),
        },
        Err(e) => crate::applog::event(&format!("engine_dump: failed to read Engine.upk: {e}")),
    }
}

pub fn read_engine_versions(path: &Path) -> Option<(u32, u32)> {
    let data = read_upk(path).ok()?;
    let (summary, _) = parser::parse_prefix(&data).ok()?;
    Some((summary.engine_version, summary.cooker_version))
}

pub const IMPORT_ENTRY_SIZE: usize = 28;

fn build_name_table(header: &[u8], name_count: i32) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(name_count.max(0) as usize);
    let mut pos = 0usize;
    for _ in 0..name_count.max(0) {
        if pos + 4 > header.len() {
            break;
        }
        let fstr_len = i32::from_le_bytes(header[pos..pos + 4].try_into().unwrap_or([0; 4]));
        let (capacity, name) = if fstr_len > 0 {
            let cap = fstr_len as usize;
            if pos + 4 + cap > header.len() {
                break;
            }
            let bytes = &header[pos + 4..pos + 4 + cap];
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(cap);
            (cap, String::from_utf8_lossy(&bytes[..end]).into_owned())
        } else if fstr_len < 0 {
            let bc = (-fstr_len as usize) * 2;
            if pos + 4 + bc > header.len() {
                break;
            }
            let bytes = &header[pos + 4..pos + 4 + bc];
            let words: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let end = words.iter().position(|&w| w == 0).unwrap_or(words.len());
            (bc, String::from_utf16_lossy(&words[..end]).to_owned())
        } else {
            (0, String::new())
        };
        names.push(name);
        pos += 4 + capacity + 8;
    }
    names
}

fn name_at(names: &[String], idx: i32) -> String {
    names.get(idx.max(0) as usize).cloned().unwrap_or_default()
}

pub fn find_package_import_linker_indices(
    header: &[u8],
    name_offset: i32,
    name_count: i32,
    import_offset: i32,
    import_count: i32,
    package_name: &str,
) -> Vec<i32> {
    let names = build_name_table(header, name_count);
    let io = (import_offset - name_offset) as usize;
    let mut result = Vec::new();
    for i in 0..import_count.max(0) as usize {
        let off = io + i * IMPORT_ENTRY_SIZE;
        if off + IMPORT_ENTRY_SIZE > header.len() {
            break;
        }
        let object_name_idx =
            i32::from_le_bytes(header[off + 20..off + 24].try_into().unwrap_or([0; 4]));
        if name_at(&names, object_name_idx).eq_ignore_ascii_case(package_name) {
            result.push(-((i as i32) + 1));
        }
    }
    result
}

pub fn find_import_linker_indices(
    header: &[u8],
    name_offset: i32,
    name_count: i32,
    import_offset: i32,
    import_count: i32,
    target_name: &str,
) -> Vec<i32> {
    let names = build_name_table(header, name_count);
    let io = (import_offset - name_offset) as usize;
    let mut result = Vec::new();
    for i in 0..import_count.max(0) as usize {
        let off = io + i * IMPORT_ENTRY_SIZE;
        if off + IMPORT_ENTRY_SIZE > header.len() {
            break;
        }
        let class_pkg_idx = i32::from_le_bytes(header[off..off + 4].try_into().unwrap_or([0; 4]));
        if name_at(&names, class_pkg_idx).eq_ignore_ascii_case(target_name) {
            result.push(-((i as i32) + 1));
        }
    }
    result
}

pub fn find_engine_dependency_linker_indices(
    header: &[u8],
    name_offset: i32,
    name_count: i32,
    import_offset: i32,
    import_count: i32,
) -> Vec<i32> {
    let mut out = find_engine_package_linker_indices(
        header,
        name_offset,
        name_count,
        import_offset,
        import_count,
    );
    if out.is_empty() {
        out = find_import_linker_indices(
            header,
            name_offset,
            name_count,
            import_offset,
            import_count,
            "Engine",
        );
    }
    out
}

fn find_engine_package_linker_indices(
    header: &[u8],
    name_offset: i32,
    name_count: i32,
    import_offset: i32,
    import_count: i32,
) -> Vec<i32> {
    let names = build_name_table(header, name_count);
    let io = (import_offset - name_offset) as usize;
    let mut result = Vec::new();
    for i in 0..import_count.max(0) as usize {
        let off = io + i * IMPORT_ENTRY_SIZE;
        if off + IMPORT_ENTRY_SIZE > header.len() {
            break;
        }
        let class_name_idx =
            i32::from_le_bytes(header[off + 8..off + 12].try_into().unwrap_or([0; 4]));
        let object_name_idx =
            i32::from_le_bytes(header[off + 20..off + 24].try_into().unwrap_or([0; 4]));
        if name_at(&names, class_name_idx).eq_ignore_ascii_case("Package")
            && name_at(&names, object_name_idx).eq_ignore_ascii_case("Engine")
        {
            result.push(-((i as i32) + 1));
        }
    }
    result
}

pub fn read_engine_import_guid_in_header(
    header: &[u8],
    summary: &parser::FileSummary,
    name_offset: i32,
    header_delta: i32,
) -> Option<[u8; 16]> {
    let engine_indices = find_engine_dependency_linker_indices(
        header,
        name_offset,
        summary.name_count,
        summary.import_offset,
        summary.import_count,
    );
    if engine_indices.is_empty() {
        return None;
    }
    let guid_section_start = (summary.import_export_guids_offset as i64
        - name_offset as i64
        + header_delta as i64) as usize;
    let entry_size = 20;
    for i in 0..summary.import_guids_count as usize {
        let off = guid_section_start + i * entry_size;
        if off + entry_size > header.len() {
            break;
        }
        let pkg_idx = i32::from_le_bytes(header[off..off + 4].try_into().unwrap_or([0; 4]));
        if engine_indices.contains(&pkg_idx) {
            let mut guid = [0u8; 16];
            guid.copy_from_slice(&header[off + 4..off + 20]);
            return Some(guid);
        }
    }
    None
}

pub fn patch_engine_versions_in_prefix(
    data: &mut [u8],
    summary: &parser::FileSummary,
    eng_ver: u32,
    cook_ver: u32,
) -> bool {
    let off = summary.engine_version_offset;
    if off + 8 > data.len() {
        return false;
    }
    let cur_eng = u32::from_le_bytes(data[off..off + 4].try_into().unwrap_or([0; 4]));
    let cur_cook = u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap_or([0; 4]));
    if cur_eng == eng_ver && cur_cook == cook_ver {
        return false;
    }
    data[off..off + 4].copy_from_slice(&eng_ver.to_le_bytes());
    data[off + 4..off + 8].copy_from_slice(&cook_ver.to_le_bytes());
    true
}

pub fn read_prefix_engine_versions(data: &[u8]) -> Option<(u32, u32)> {
    let (summary, _) = parser::parse_prefix(data).ok()?;
    let off = summary.engine_version_offset;
    if off + 8 > data.len() {
        return None;
    }
    Some((
        u32::from_le_bytes(data[off..off + 4].try_into().ok()?),
        u32::from_le_bytes(data[off + 4..off + 8].try_into().ok()?),
    ))
}

pub fn header_references_stale_engine(
    header: &[u8],
    summary: &parser::FileSummary,
    name_offset: i32,
    header_delta: i32,
    current_engine_guid: &[u8; 16],
) -> bool {
    if summary.import_export_guids_offset < 0 || summary.import_guids_count <= 0 {
        return false;
    }
    let engine_indices = find_engine_dependency_linker_indices(
        header,
        name_offset,
        summary.name_count,
        summary.import_offset,
        summary.import_count,
    );
    if engine_indices.is_empty() {
        return false;
    }
    let guid_section_start = (summary.import_export_guids_offset as i64
        - name_offset as i64
        + header_delta as i64) as usize;
    let entry_size = 20;
    for i in 0..summary.import_guids_count as usize {
        let off = guid_section_start + i * entry_size;
        if off + entry_size > header.len() {
            break;
        }
        let pkg_idx = i32::from_le_bytes(header[off..off + 4].try_into().unwrap_or([0; 4]));
        if engine_indices.contains(&pkg_idx) {
            let guid = &header[off + 4..off + 20];
            if guid != current_engine_guid.as_slice() {
                return true;
            }
        }
    }
    false
}

pub fn patch_engine_import_guid(
    header: &mut [u8],
    summary: &parser::FileSummary,
    name_offset: i32,
    header_delta: i32,
    engine_guid: [u8; 16],
) -> bool {
    if summary.import_export_guids_offset < 0 || summary.import_guids_count <= 0 {
        return false;
    }

    let engine_indices = find_engine_dependency_linker_indices(
        header,
        name_offset,
        summary.name_count,
        summary.import_offset,
        summary.import_count,
    );
    if engine_indices.is_empty() {
        return false;
    }

    let guid_section_start =
        (summary.import_export_guids_offset as i64 - name_offset as i64 + header_delta as i64) as usize;
    let entry_size = 20;

    let mut patched = false;
    for i in 0..summary.import_guids_count as usize {
        let off = guid_section_start + i * entry_size;
        if off + entry_size > header.len() { break; }
        let pkg_idx = i32::from_le_bytes(
            header[off..off + 4].try_into().unwrap_or([0; 4])
        );
        if engine_indices.contains(&pkg_idx) {
            header[off + 4..off + 20].copy_from_slice(&engine_guid);
            patched = true;
        }
    }
    patched
}

fn replace_file_atomic(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    crate::winprobe::replace_file_atomic(from, to)
}

fn write_swap_atomically(target: &Path, backup: &Path, data: &[u8]) -> Result<(), SwapError> {
    let tmp_name = format!(
        "{}.vrl.tmp",
        target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "swap.upk".into())
    );
    let tmp = target.with_file_name(tmp_name);
    if let Err(e) = std::fs::write(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SwapError::Msg(format!(
            "Failed to stage swap at {}: {}",
            tmp.display(),
            explain_io(&e)
        )));
    }
    if let Err(e) = std::fs::copy(target, backup) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SwapError::Msg(format!(
            "Failed to create backup at {}: {}",
            backup.display(),
            explain_io(&e)
        )));
    }

    if let Err(e) = replace_file_atomic(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SwapError::Msg(format!(
            "Failed to atomically replace {}: {}",
            target.display(),
            explain_io(&e)
        )));
    }
    Ok(())
}

pub fn swap_asset(
    target_id: &str,
    donor_id: &str,
    paint_id: i32,
    opts: &SwapOptions,
) -> Result<String, SwapError> {
    dump_engine_info(&opts.game_dir);
    if !(0..=12).contains(&paint_id) {
        return Err(SwapError::Msg(format!(
            "invalid paint id {paint_id} (use 0 for None, or 1–12)"
        )));
    }

    let items = load_items(&opts.items_json)?;
    let tid: i64 = target_id
        .parse()
        .map_err(|_| SwapError::Msg(format!("invalid target id: {}", target_id)))?;
    let did: i64 = donor_id
        .parse()
        .map_err(|_| SwapError::Msg(format!("invalid donor id: {}", donor_id)))?;
    if tid == did {
        return Err(SwapError::Msg(
            "owned item and target asset are the same — nothing to swap".into(),
        ));
    }
    let mut target = find_item_by_id(&items, tid)
        .ok_or_else(|| SwapError::Msg(format!("target item {tid} not found")))?
        .clone();
    let mut donor = find_item_by_id(&items, did)
        .ok_or_else(|| SwapError::Msg(format!("donor item {did} not found")))?
        .clone();

    if target.slot != donor.slot {
        return Err(SwapError::Msg(format!(
            "slot mismatch: target='{}' donor='{}'",
            target.slot, donor.slot
        )));
    }

    let (donor_path, resolved_donor_pkg) = resolve_package_path(&opts.game_dir, &donor.asset_package)
        .ok_or_else(|| {
            SwapError::Msg(format!(
                "Wanted item file not found: '{}' ({}). Path: {}",
                donor.asset_package,
                if donor.product.is_empty() { "item" } else { &donor.product },
                opts.game_dir.join(&donor.asset_package).display()
            ))
        })?;
    donor.asset_package = resolved_donor_pkg;

    let (target_path, resolved_target_pkg) = resolve_package_path(&opts.game_dir, &target.asset_package)
        .ok_or_else(|| {
            SwapError::Msg(format!(
                "Owned item file not found: '{}' ({}). Path: {}",
                target.asset_package,
                if target.product.is_empty() { "item" } else { &target.product },
                opts.game_dir.join(&target.asset_package).display()
            ))
        })?;
    target.asset_package = resolved_target_pkg;

    let mut donor_path = donor_path;
    let paint_name = paint_label(paint_id);
    let mut used_painted_file = false;
    if paint_id > 0 {
        if let Some(pkg) = find_painted_package(&opts.game_dir, &donor.asset_package, paint_id) {
            let slug = paint_slugs(paint_id)
                .into_iter()
                .next()
                .unwrap_or_else(|| paint_name.replace(' ', ""));
            donor.asset_path = suffix_asset_path(&donor.asset_path, &slug);
            donor.asset_package = pkg.clone();
            donor_path = opts.game_dir.join(pkg);
            used_painted_file = true;
        }
    }

    let backup_name = {
        let name = target_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return Err(SwapError::Msg("target package has no file name".into()));
        }
        format!("{name}.bak")
    };
    let backup_path = target_path.with_file_name(backup_name);

    if backup_path.exists() {
        crate::applog::event(&format!(
            "swap: blocked by existing backup {}",
            backup_path.display()
        ));
        return Err(SwapError::AlreadySwapped(format!(
            "{} is already swapped — open the Restore tab and click Restore on it first, then swap again.",
            if target.product.is_empty() {
                target.asset_package.as_str()
            } else {
                target.product.as_str()
            }
        )));
    }

    let all_keys = crypto::load_keys(&opts.keys_txt);
    let keys_map = crypto::load_keys_map(&opts.keys_map_json);

    let donor_file = read_upk(&donor_path)?;
    let (donor_summary, donor_meta) = parser::parse_prefix(&donor_file)
        .map_err(|e| SwapError::Msg(format!("parse donor: {e}")))?;

    if donor_summary.name_count < 0 || donor_summary.name_count > MAX_NAME_COUNT {
        return Err(SwapError::Msg(format!(
            "donor name_count {} is implausible",
            donor_summary.name_count
        )));
    }
    if donor_summary.name_offset < 0 {
        return Err(SwapError::Msg("donor name_offset is negative".into()));
    }

    let name_offset = donor_summary.name_offset as usize;
    let enc_size = donor_summary
        .total_header_size
        .checked_sub(donor_meta.garbage_size)
        .and_then(|v| v.checked_sub(donor_summary.name_offset))
        .ok_or_else(|| SwapError::Msg("donor encrypted-block size underflow".into()))?;
    if enc_size <= 0 {
        return Err(SwapError::Msg("donor encrypted block is empty".into()));
    }
    let enc_size = enc_size as usize;
    let enc_size_aligned = (enc_size + 15) & !15;
    if name_offset
        .checked_add(enc_size_aligned)
        .map(|end| end > donor_file.len())
        .unwrap_or(true)
    {
        return Err(SwapError::Msg("donor encrypted block OOB".into()));
    }
    let enc_block = &donor_file[name_offset..name_offset + enc_size_aligned];

    let donor_stem = file_stem(&donor.asset_package).to_lowercase();
    let donor_stem_no_sf = package_base(&donor_stem).to_string();
    let map_key = keys_map
        .get(&donor_stem)
        .or_else(|| keys_map.get(&donor_stem_no_sf))
        .copied();

    let donor_key = map_key
        .and_then(|k| {
            crypto::find_valid_key_relaxed(enc_block, donor_meta.compressed_chunks_offset, &[k])
        })
        .or_else(|| {
            crypto::find_valid_key(
                enc_block,
                donor_summary.depends_offset,
                donor_meta.compressed_chunks_offset,
                &all_keys,
            )
        })
        .ok_or_else(|| {
            SwapError::Msg(format!(
                "No decryption key for {}. [keys={} enc_block_len={}]",
                donor.asset_package,
                all_keys.len() + map_key.is_some() as usize,
                enc_block.len(),
            ))
        })?;

    let header_plain = crypto::decrypt_ecb(&donor_key, enc_block);

    let orig_donor = find_item_by_id(&items, did)
        .cloned()
        .ok_or_else(|| SwapError::Msg(format!("donor item {did} not found")))?;
    let mut pairs = infer_name_pairs(&target, &orig_donor);
    if used_painted_file {
        for p in infer_name_pairs(&target, &donor) {
            add_pair(&mut pairs, p.0, p.1);
        }
    }
    let _base_pair_count = pairs.len();
    extend_paint_name_pairs(&mut pairs, paint_id);

    if pairs.is_empty() {
        return Err(SwapError::Msg(
            "no name remaps between these items — they already share the same asset names".into(),
        ));
    }

    let import_off = calc_relative_offset(
        donor_summary.import_offset,
        donor_summary.name_offset,
        "import_offset",
    )?;
    let export_off = calc_relative_offset(
        donor_summary.export_offset,
        donor_summary.name_offset,
        "export_offset",
    )?;
    let depends_off = calc_relative_offset(
        donor_summary.depends_offset,
        donor_summary.name_offset,
        "depends_offset",
    )?;
    if import_off > export_off || export_off > depends_off {
        return Err(SwapError::Msg(
            "donor header table offsets are not in name < import < export < depends order".into(),
        ));
    }
    if depends_off > header_plain.len() {
        return Err(SwapError::Msg("donor header tables overrun decrypted block".into()));
    }

    let (mut new_header_plain, header_delta) = nametable::apply_header_renames(
        header_plain,
        import_off,
        export_off,
        depends_off,
        donor_summary.name_count,
        &pairs,
    )
    .map_err(|e| {
        if e.contains("already references") {
            SwapError::Collision(e)
        } else {
            SwapError::Msg(e)
        }
    })?;

    let target_stem = file_stem(&target.asset_package);
    let target_base = package_base(&target_stem);
    let target_pascal = to_pascal_case(target_base);
    let orig_donor_stem = file_stem(&orig_donor.asset_package);
    let orig_donor_base = package_base(&orig_donor_stem);
    let has_target = name_table_has(&new_header_plain, donor_summary.name_count, &target_stem)
        || (!target_base.is_empty() && (
            name_table_has(&new_header_plain, donor_summary.name_count, target_base)
            || name_table_has(&new_header_plain, donor_summary.name_count, &target_pascal)
        ));

    if !target_stem.is_empty()
        && !orig_donor_stem.eq_ignore_ascii_case(&target_stem)
        && !orig_donor_base.eq_ignore_ascii_case(target_base)
        && !has_target
    {
        return Err(SwapError::Msg(format!(
            "Could not remap package names ('{orig_donor_stem}' → '{target_stem}'). Swap aborted so the game will not crash."
        )));
    }

    let pkg_stem = file_stem(&target.asset_package).to_lowercase();
    let no_sf = package_base(&pkg_stem).to_string();
    let output_key = keys_map
        .get(&pkg_stem)
        .or_else(|| keys_map.get(&no_sf))
        .copied()
        .or_else(|| {
            read_upk(&target_path).ok().and_then(|tfile| {
                let (ts, tm) = parser::parse_prefix(&tfile).ok()?;
                if ts.name_offset < 0 {
                    return None;
                }
                let tn = ts.name_offset as usize;
                let te = ts
                    .total_header_size
                    .checked_sub(tm.garbage_size)
                    .and_then(|v| v.checked_sub(ts.name_offset))?;
                if te <= 0 {
                    return None;
                }
                let te_al = (te as usize + 15) & !15;
                if tn.checked_add(te_al).map(|end| end <= tfile.len())? {
                    crypto::find_valid_key(
                        &tfile[tn..tn + te_al],
                        ts.depends_offset,
                        tm.compressed_chunks_offset,
                        &all_keys,
                    )
                } else {
                    None
                }
            })
        })
        .unwrap_or(donor_key);

    let new_enc_size_aligned = (new_header_plain.len() + 15) & !15;
    let size_growth = new_enc_size_aligned as i64 - enc_size_aligned as i64;

    if size_growth > donor_meta.garbage_size as i64 {
        return Err(SwapError::Msg(format!(
            "Header grew by {} bytes but only {} bytes of padding available.",
            size_growth, donor_meta.garbage_size
        )));
    }

    new_header_plain.resize(new_enc_size_aligned, 0u8);
    let new_enc_block = crypto::encrypt_ecb(&output_key, &new_header_plain);

    let mut output = donor_file;
    let old_enc_end = name_offset + enc_size_aligned;
    let new_enc_end = name_offset + new_enc_size_aligned;
    if old_enc_end > output.len() {
        return Err(SwapError::Msg("donor encrypted block OOB during splice".into()));
    }
    output.splice(name_offset..old_enc_end, new_enc_block.iter().copied());

    if size_growth > 0 {
        let gap_start = new_enc_end;
        let trim = size_growth as usize;
        if gap_start
            .checked_add(trim)
            .map(|end| end > output.len())
            .unwrap_or(true)
        {
            return Err(SwapError::Msg(
                "Not enough gap bytes to absorb header growth.".into(),
            ));
        }
        output.drain(gap_start..gap_start + trim);
    }

    if header_delta != 0 || size_growth != 0 {
        let offsets = parser::find_summary_offsets(&output)
            .map_err(|e| SwapError::Msg(format!("find_summary_offsets: {e}")))?;
        if header_delta != 0 {
            patch_i32_le(
                &mut output,
                offsets.import_offset_offset,
                donor_summary.import_offset + header_delta as i32,
            );
            patch_i32_le(
                &mut output,
                offsets.export_offset_offset,
                donor_summary.export_offset + header_delta as i32,
            );
            patch_i32_le(
                &mut output,
                offsets.depends_offset_offset,
                donor_summary.depends_offset + header_delta as i32,
            );
        }
        let meta_off = donor_meta.meta_file_offset;
        if size_growth != 0 && meta_off + 8 <= output.len() {
            let new_garbage = donor_meta.garbage_size - size_growth as i32;
            patch_i32_le(&mut output, meta_off, new_garbage);
        }
        if header_delta != 0 && meta_off + 8 <= output.len() {
            let new_chunks_off = donor_meta.compressed_chunks_offset + header_delta as i32;
            patch_i32_le(&mut output, meta_off + 4, new_chunks_off);
        }
    }

    write_swap_atomically(&target_path, &backup_path, &output)?;

    let paint_note = if paint_id > 0 {
        if used_painted_file {
            format!(" Paint: {paint_name} (dedicated UPK).")
        } else {
            format!(
                " Paint: {paint_name} materials remapped — in-game color still follows the paint on the item you equip."
            )
        }
    } else {
        String::new()
    };

    Ok(format!(
        "Swap complete: {} bytes written. Backup saved to: {}.{}",
        output.len(),
        backup_path.display(),
        paint_note
    ))
}

pub fn restore_single(path: &str) -> Result<(), SwapError> {
    if path.trim().is_empty() {
        return Err(SwapError::Msg("restore path is empty".into()));
    }
    let (orig, bak) = if path.ends_with(".bak") {
        let bak_p = PathBuf::from(path);
        let orig_name = bak_p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .trim_end_matches(".bak")
            .to_string();
        if orig_name.is_empty() {
            return Err(SwapError::Msg("backup file name is invalid".into()));
        }
        (bak_p.with_file_name(orig_name), bak_p)
    } else {
        let p = PathBuf::from(path);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return Err(SwapError::Msg("restore path has no file name".into()));
        }
        let bak_p = p.with_file_name(format!("{name}.bak"));
        (p, bak_p)
    };

    if !bak.exists() {
        return Err(SwapError::Msg(format!(
            "no backup found for {}",
            orig.display()
        )));
    }
    std::fs::copy(&bak, &orig).map_err(|e| {
        SwapError::Msg(format!(
            "failed to restore {}: {}",
            orig.display(),
            explain_io(&e)
        ))
    })?;
    std::fs::remove_file(&bak).map_err(|e| {
        SwapError::Msg(format!(
            "restored {} but could not delete backup: {}",
            orig.display(),
            explain_io(&e)
        ))
    })?;
    Ok(())
}

pub fn restore_all(game_dir: &str) -> Result<usize, SwapError> {
    if game_dir.trim().is_empty() {
        return Err(SwapError::Msg("game directory is empty".into()));
    }
    let dir = Path::new(game_dir);
    if !dir.is_dir() {
        return Err(SwapError::Msg(format!(
            "game directory not found: {}",
            dir.display()
        )));
    }
    let mut count = 0;
    let entries = std::fs::read_dir(dir).map_err(|e| {
        SwapError::Msg(format!(
            "cannot list {}: {}",
            dir.display(),
            explain_io(&e)
        ))
    })?;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.ends_with(".upk.bak") {
            continue;
        }
        let orig_name = name.trim_end_matches(".bak");
        if orig_name.is_empty() {
            continue;
        }
        let orig_path = dir.join(orig_name);
        if let Err(e) = std::fs::copy(&path, &orig_path) {
            return Err(SwapError::Msg(format!(
                "failed to restore {}: {}",
                orig_path.display(),
                explain_io(&e)
            )));
        }
        if let Err(e) = std::fs::remove_file(&path) {
            return Err(SwapError::Msg(format!(
                "restored {} but could not delete backup: {}",
                orig_path.display(),
                explain_io(&e)
            )));
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_item(id: i64, pkg: &str, path: &str) -> Item {
        Item {
            id,
            product: format!("item{id}"),
            slot: "Body".into(),
            asset_package: pkg.into(),
            asset_path: path.into(),
        }
    }

    #[test]
    fn paint_slugs_cover_standard_ids() {
        assert!(paint_slugs(0).is_empty());
        assert!(paint_slugs(13).is_empty());
        let crimson = paint_slugs(1);
        assert!(crimson.iter().any(|s| s == "Crimson"));
        assert!(crimson.iter().any(|s| s == "P1"));
        let tw = paint_slugs(12);
        assert!(tw.iter().any(|s| s == "TW" || s == "TitaniumWhite"));
        assert!(!tw.iter().any(|s| s == "T"));
    }

    #[test]
    fn thumbnail_companion_is_not_treated_as_paint() {
        assert!(is_thumbnail_companion_upk("Body_Octane_T_SF.upk"));
        assert!(!is_thumbnail_companion_upk("Body_Octane_TW_SF.upk"));
        let c = painted_package_candidates("Body_Octane_SF.upk", 12);
        assert!(!c.iter().any(|p| p == "Body_Octane_T_SF.upk"));
    }

    #[test]
    fn infer_pairs_from_asset_path() {
        let target = mock_item(1, "Body_Octane_SF.upk", "Body_Octane.Body_Octane");
        let donor = mock_item(2, "Body_S5Fennec_SF.upk", "Body_S5Fennec.Body_S5Fennec");
        let pairs = infer_name_pairs(&target, &donor);
        assert!(pairs
            .iter()
            .any(|(o, n)| o == "Body_S5Fennec" && n == "Body_Octane"));
        assert!(pairs
            .iter()
            .any(|(o, n)| o == "Body_S5Fennec_SF" && n == "Body_Octane_SF"));
    }

    #[test]
    fn paint_pairs_add_painted_suffix() {
        let mut pairs = vec![("Body_Fennec".into(), "Body_Octane".into())];
        extend_paint_name_pairs(&mut pairs, 12);
        assert!(pairs
            .iter()
            .any(|(o, n)| o == "Body_Fennec_Painted" && n == "Body_Octane_Painted"));
        assert!(pairs
            .iter()
            .any(|(o, n)| o.contains("TitaniumWhite") && n.contains("TitaniumWhite")));
    }

    #[test]
    fn load_items_rejects_garbage() {
        assert!(load_items("not json").is_err());
        assert!(load_items("[]").is_err());
    }

    #[test]
    fn infer_pairs_with_bare_api_package() {
        let target = mock_item(380, "WHEEL_Triad_SF.upk", "");
        let donor = mock_item(386, "Wheel_SoccerBall", "");
        let pairs = infer_name_pairs(&target, &donor);
        assert!(pairs.iter().any(|(o, n)| o == "Wheel_SoccerBall" && n == "WHEEL_Triad"));
        assert!(pairs.iter().any(|(o, n)| o == "Wheel_SoccerBall_SF" && n == "WHEEL_Triad_SF"));
        assert!(pairs.iter().any(|(o, n)| o == "Wheel_SoccerBall_TA" && n == "WHEEL_Triad_TA"));
    }

    #[test]
    fn test_resolve_package_path_variants() {
        let temp_dir = std::env::temp_dir().join(format!("vrl_test_swapper_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let sample_sf = temp_dir.join("Wheel_SoccerBall_SF.upk");
        std::fs::write(&sample_sf, b"dummy").unwrap();

        // 1. Exact match
        let res = resolve_package_path(&temp_dir, "Wheel_SoccerBall_SF.upk");
        assert!(res.is_some());
        assert_eq!(res.unwrap().1, "Wheel_SoccerBall_SF.upk");

        // 2. Bare stem without _SF and without .upk
        let res = resolve_package_path(&temp_dir, "Wheel_SoccerBall");
        assert!(res.is_some());
        assert_eq!(res.unwrap().1, "Wheel_SoccerBall_SF.upk");

        // 3. Stem with .upk but without _SF
        let res = resolve_package_path(&temp_dir, "Wheel_SoccerBall.upk");
        assert!(res.is_some());
        assert_eq!(res.unwrap().1, "Wheel_SoccerBall_SF.upk");

        // 4. Case-insensitive
        let res = resolve_package_path(&temp_dir, "wheel_soccerball");
        assert!(res.is_some());

        // 5. Non-existent
        let res = resolve_package_path(&temp_dir, "NonExistentPackage_12345");
        assert!(res.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
