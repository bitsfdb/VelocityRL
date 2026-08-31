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
        5 => {
            slugs.push("SB".into());
            slugs.push("Sky_Blue".into());
        }
        8 => slugs.push("Gray".into()),
        10 => {
            slugs.push("FG".into());
            slugs.push("Forest_Green".into());
        }
        12 => {
            slugs.push("TW".into());
            slugs.push("Titanium_White".into());
        }
        _ => {}
    }
    slugs
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

fn find_painted_package(game_dir: &Path, asset_package: &str, paint_id: i32) -> Option<String> {
    for cand in painted_package_candidates(asset_package, paint_id) {
        if game_dir.join(&cand).is_file() {
            return Some(cand);
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

fn infer_name_pairs(target: &Item, donor: &Item) -> Vec<(String, String)> {
    let donor_stem = file_stem(&donor.asset_package);
    let target_stem = file_stem(&target.asset_package);

    let donor_parts: Vec<&str> = donor
        .asset_path
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();
    let target_parts: Vec<&str> = target
        .asset_path
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();

    let mut pairs: Vec<(String, String)> = Vec::new();
    let len = donor_parts.len().min(target_parts.len());
    for i in 0..len {
        add_pair(
            &mut pairs,
            donor_parts[i].to_string(),
            target_parts[i].to_string(),
        );
    }
    if !donor_stem.is_empty() && !target_stem.is_empty() {
        add_pair(&mut pairs, donor_stem, target_stem);
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

fn rel_off(abs: i32, base: i32, label: &str) -> Result<usize, SwapError> {
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

fn any_old_name_present(header: &[u8], name_count: i32, pairs: &[(String, String)]) -> bool {
    pairs
        .iter()
        .any(|(old, _)| name_table_has(header, name_count, old))
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

    if std::fs::rename(&tmp, target).is_err() {
        if let Err(e) = std::fs::write(target, data) {
            let _ = std::fs::copy(backup, target);
            let _ = std::fs::remove_file(&tmp);
            return Err(SwapError::Msg(format!(
                "Failed to write output to {}: {}",
                target.display(),
                explain_io(&e)
            )));
        }
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(())
}

pub fn swap_asset(
    target_id: &str,
    donor_id: &str,
    paint_id: i32,
    opts: &SwapOptions,
) -> Result<String, SwapError> {
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
    let target = find_item_by_id(&items, tid)
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

    let paint_name = paint_label(paint_id);
    let mut used_painted_file = false;
    if paint_id > 0 {
        if let Some(pkg) = find_painted_package(&opts.game_dir, &donor.asset_package, paint_id) {
            let slug = paint_slugs(paint_id)
                .into_iter()
                .next()
                .unwrap_or_else(|| paint_name.replace(' ', ""));
            donor.asset_path = suffix_asset_path(&donor.asset_path, &slug);
            donor.asset_package = pkg;
            used_painted_file = true;
        }
    }

    let donor_path = opts.game_dir.join(&donor.asset_package);
    let target_path = opts.game_dir.join(&target.asset_package);
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
        return Err(SwapError::AlreadySwapped(format!(
            "{} is already swapped — restore it first.",
            if target.product.is_empty() {
                target.asset_package.as_str()
            } else {
                target.product.as_str()
            }
        )));
    }
    if !donor_path.exists() {
        return Err(SwapError::Msg(format!(
            "donor file not found: {}",
            donor_path.display()
        )));
    }
    if !target_path.exists() {
        return Err(SwapError::Msg(format!(
            "target file not found: {}",
            target_path.display()
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

    let import_off = rel_off(
        donor_summary.import_offset,
        donor_summary.name_offset,
        "import_offset",
    )?;
    let export_off = rel_off(
        donor_summary.export_offset,
        donor_summary.name_offset,
        "export_offset",
    )?;
    let depends_off = rel_off(
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
    let orig_donor_stem = file_stem(&orig_donor.asset_package);
    if !target_stem.is_empty()
        && !orig_donor_stem.eq_ignore_ascii_case(&target_stem)
        && !name_table_has(&new_header_plain, donor_summary.name_count, &target_stem)
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

    fn item(id: i64, pkg: &str, path: &str) -> Item {
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
    }

    #[test]
    fn painted_candidates_use_sf_suffix() {
        let c = painted_package_candidates("Body_Octane_SF.upk", 1);
        assert!(c.iter().any(|p| p == "Body_Octane_Crimson_SF.upk"));
    }

    #[test]
    fn infer_pairs_from_asset_path() {
        let target = item(1, "Body_Octane_SF.upk", "Body_Octane.Body_Octane");
        let donor = item(2, "Body_S5Fennec_SF.upk", "Body_S5Fennec.Body_S5Fennec");
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
}
