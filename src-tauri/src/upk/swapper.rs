use crate::upk::{crypto, parser, nametable};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn patch_i32_le(data: &mut [u8], offset: usize, value: i32) {
    if offset + 4 <= data.len() {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
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
            SwapError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
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

/// Load items from the JSON. Supports both CrunchyRL format `{"Items":[...]}` and flat array.
fn load_items(json: &str) -> Vec<Item> {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = v
        .get("Items")
        .or_else(|| v.get("items"))
        .and_then(|a| a.as_array())
        .cloned()
        .or_else(|| v.as_array().cloned())
        .unwrap_or_default();
    arr.iter()
        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
        .filter(|i: &Item| !i.asset_package.is_empty())
        .collect()
}

/// Find an item by numeric ID.
fn find_item_by_id(items: &[Item], id: i64) -> Option<&Item> {
    items.iter().find(|i| i.id == id)
}

/// Infer the name rename pairs for a swap.
/// Pairs are derived from the dot-separated asset_path parts and the package stem.
fn infer_name_pairs(target: &Item, donor: &Item) -> Vec<(String, String)> {
    let donor_stem = Path::new(&donor.asset_package)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let target_stem = Path::new(&target.asset_package)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

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
        let old = donor_parts[i].to_string();
        let new = target_parts[i].to_string();
        if !old.is_empty() && !new.is_empty() && old != new {
            if !pairs.contains(&(old.clone(), new.clone())) {
                pairs.push((old, new));
            }
        }
    }
    // Package stem pair
    if !donor_stem.is_empty() && !target_stem.is_empty() && donor_stem != target_stem {
        let pair = (donor_stem, target_stem);
        if !pairs.contains(&pair) {
            pairs.push(pair);
        }
    }
    pairs
}

/// Perform the UPK asset swap using Shift's approach:
/// decrypt only the AES header block, rename name entries in-place,
/// re-encrypt the header, write it back — body/chunks are never touched.
pub fn swap_asset(
    target_id: &str,
    donor_id: &str,
    opts: &SwapOptions,
) -> Result<String, SwapError> {
    // ── Load items ───────────────────────────────────────────────────────────────
    let items = load_items(&opts.items_json);
    let tid: i64 = target_id
        .parse()
        .map_err(|_| SwapError::Msg(format!("invalid target id: {}", target_id)))?;
    let did: i64 = donor_id
        .parse()
        .map_err(|_| SwapError::Msg(format!("invalid donor id: {}", donor_id)))?;
    let target = find_item_by_id(&items, tid)
        .ok_or_else(|| SwapError::Msg(format!("target item {} not found", tid)))?;
    let donor = find_item_by_id(&items, did)
        .ok_or_else(|| SwapError::Msg(format!("donor item {} not found", did)))?;

    if target.slot != donor.slot {
        return Err(SwapError::Msg(format!(
            "slot mismatch: target='{}' donor='{}'",
            target.slot, donor.slot
        )));
    }

    let donor_path = opts.game_dir.join(&donor.asset_package);
    let target_path = opts.game_dir.join(&target.asset_package);
    let backup_path = {
        let mut p = target_path.clone();
        let mut name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        name.push_str(".bak");
        p.set_file_name(name);
        p
    };

    if backup_path.exists() {
        return Err(SwapError::AlreadySwapped(format!(
            "{} is already swapped — restore it first.",
            target.asset_package
        )));
    }
    if !donor_path.exists() {
        return Err(SwapError::Msg(format!("donor file not found: {}", donor_path.display())));
    }
    if !target_path.exists() {
        return Err(SwapError::Msg(format!("target file not found: {}", target_path.display())));
    }

    // ── Load keys ────────────────────────────────────────────────────────────────
    let all_keys = crypto::load_keys(&opts.keys_txt);
    let keys_map = crypto::load_keys_map(&opts.keys_map_json);

    // ── Read donor file, locate AES-encrypted header block ───────────────────────
    let donor_file = std::fs::read(&donor_path)?;
    let (donor_summary, donor_meta) = parser::parse_prefix(&donor_file)
        .map_err(|e| SwapError::Msg(format!("parse donor: {}", e)))?;

    let name_offset = donor_summary.name_offset as usize;
    let enc_size = (donor_summary.total_header_size
        - donor_meta.garbage_size
        - donor_summary.name_offset) as usize;
    let enc_size_aligned = (enc_size + 15) & !15;
    if name_offset + enc_size_aligned > donor_file.len() {
        return Err(SwapError::Msg("donor encrypted block OOB".into()));
    }
    let enc_block = &donor_file[name_offset..name_offset + enc_size_aligned];

    // ── Find donor AES key ────────────────────────────────────────────────────────
    let donor_stem = Path::new(&donor.asset_package)
        .file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
    let donor_stem_no_sf = donor_stem.trim_end_matches("_sf").to_string();
    let map_key = keys_map.get(&donor_stem).or_else(|| keys_map.get(&donor_stem_no_sf)).copied();

    let donor_key = map_key
        .and_then(|k| crypto::find_valid_key_relaxed(enc_block, donor_meta.compressed_chunks_offset, &[k]))
        .or_else(|| crypto::find_valid_key(enc_block, donor_summary.depends_offset, donor_meta.compressed_chunks_offset, &all_keys))
        .ok_or_else(|| SwapError::Msg(format!(
            "No decryption key for {}. [keys={} enc_block_len={}]",
            donor.asset_package,
            all_keys.len() + map_key.is_some() as usize,
            enc_block.len(),
        )))?;

    // ── Decrypt header, rename, re-encrypt ───────────────────────────────────────
    // Shift's approach: patch only the AES header block. The compressed body is
    // never touched — it stays exactly as-is from the donor file.
    // If the new name is longer than the old slot, we rebuild the name/import/export
    // tables and absorb the growth by shrinking garbage_size (body doesn't move).
    let header_plain = crypto::decrypt_ecb(&donor_key, enc_block);

    let pairs = infer_name_pairs(target, donor);
    let import_off = (donor_summary.import_offset - donor_summary.name_offset) as usize;
    let export_off = (donor_summary.export_offset - donor_summary.name_offset) as usize;
    let depends_off = (donor_summary.depends_offset - donor_summary.name_offset) as usize;

    let (mut new_header_plain, header_delta) = nametable::apply_header_renames(
        header_plain,
        import_off,
        export_off,
        depends_off,
        donor_summary.name_count,
        &pairs,
    )
    .map_err(|e| {
        if e.contains("already references") { SwapError::Collision(e) }
        else { SwapError::Msg(e) }
    })?;

    // ── Find output key for target file ──────────────────────────────────────────
    let pkg_stem = Path::new(&target.asset_package)
        .file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
    let no_sf = pkg_stem.trim_end_matches("_sf").to_string();
    let output_key = keys_map
        .get(&pkg_stem).or_else(|| keys_map.get(&no_sf)).copied()
        .or_else(|| {
            std::fs::read(&target_path).ok().and_then(|tfile| {
                let (ts, tm) = parser::parse_prefix(&tfile).ok()?;
                let tn = ts.name_offset as usize;
                let te_al = ((ts.total_header_size - tm.garbage_size - ts.name_offset) as usize + 15) & !15;
                if tn + te_al <= tfile.len() {
                    crypto::find_valid_key(&tfile[tn..tn + te_al], ts.depends_offset, tm.compressed_chunks_offset, &all_keys)
                } else { None }
            })
        })
        .unwrap_or(donor_key);

    // ── Pad header to AES block boundary and compute new sizes ───────────────────
    let new_enc_size_aligned = (new_header_plain.len() + 15) & !15;
    let size_growth = new_enc_size_aligned as i64 - enc_size_aligned as i64;

    if size_growth > donor_meta.garbage_size as i64 {
        return Err(SwapError::Msg(format!(
            "Header grew by {} bytes but only {} bytes of padding available.",
            size_growth, donor_meta.garbage_size
        )));
    }

    // Pad to AES alignment
    new_header_plain.resize(new_enc_size_aligned, 0u8);
    let new_enc_block = crypto::encrypt_ecb(&output_key, &new_header_plain);

    // ── Build output: replace header block in donor file ─────────────────────────
    // The compressed body stays at the same file offsets; only the header and
    // the gap (garbage) change size. total_header_size = name_off + enc + gap + ...
    // Since chunks don't move: new_gap = old_gap - size_growth, total unchanged.
    let mut output = donor_file.clone();

    // Splice in the new (possibly different-size) encrypted header
    let old_enc_end = name_offset + enc_size_aligned;
    let new_enc_end = name_offset + new_enc_size_aligned;
    output.splice(name_offset..old_enc_end, new_enc_block.iter().copied());

    // If header grew, trim the same number of bytes from the garbage region
    if size_growth > 0 {
        let gap_start = new_enc_end;
        let trim = size_growth as usize;
        if gap_start + trim > output.len() {
            return Err(SwapError::Msg("Not enough gap bytes to absorb header growth.".into()));
        }
        output.drain(gap_start..gap_start + trim);
    }

    // Patch prefix: import/export/depends offsets shift by header_delta;
    // garbage_size and compressed_chunks_offset shift by size_growth.
    if header_delta != 0 || size_growth != 0 {
        let offsets = parser::find_summary_offsets(&output)
            .map_err(|e| SwapError::Msg(format!("find_summary_offsets: {}", e)))?;
        if header_delta != 0 {
            patch_i32_le(&mut output, offsets.import_offset_offset,
                donor_summary.import_offset + header_delta as i32);
            patch_i32_le(&mut output, offsets.export_offset_offset,
                donor_summary.export_offset + header_delta as i32);
            patch_i32_le(&mut output, offsets.depends_offset_offset,
                donor_summary.depends_offset + header_delta as i32);
        }
        // Meta fields: garbage_size and compressed_chunks_offset
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

    // ── Backup target first, then write ──────────────────────────────────────────
    std::fs::copy(&target_path, &backup_path)
        .map_err(|e| SwapError::Msg(format!("Failed to create backup at {}: {}", backup_path.display(), e)))?;
    std::fs::write(&target_path, &output)
        .map_err(|e| SwapError::Msg(format!("Failed to write output to {}: {}", target_path.display(), e)))?;

    Ok(format!(
        "Swap complete: {} bytes written. Backup saved to: {}",
        output.len(),
        backup_path.display()
    ))
}

/// Restore a previously swapped file from its .bak backup.
/// Accepts either the original UPK path OR the .bak path itself.
pub fn restore_single(path: &str) -> Result<(), SwapError> {
    let (orig, bak) = if path.ends_with(".bak") {
        // Caller passed the .bak path — strip it to get the original
        let bak_p = PathBuf::from(path);
        let orig_name = bak_p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .trim_end_matches(".bak")
            .to_string();
        (bak_p.with_file_name(orig_name), bak_p)
    } else {
        // Caller passed the original UPK path — append .bak
        let p = PathBuf::from(path);
        let mut name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        name.push_str(".bak");
        let bak_p = p.with_file_name(name);
        (p, bak_p)
    };

    if !bak.exists() {
        return Err(SwapError::Msg(format!("no backup found for {}", orig.display())));
    }
    std::fs::copy(&bak, &orig)?;
    std::fs::remove_file(&bak)?;
    Ok(())
}

/// Restore all swapped files in `game_dir` by copying .upk.bak → .upk.
pub fn restore_all(game_dir: &str) -> Result<usize, SwapError> {
    let dir = Path::new(game_dir);
    let mut count = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.ends_with(".upk.bak") {
            // Restore: remove ".bak" to get original path
            let orig_name = name.trim_end_matches(".bak");
            let orig_path = dir.join(orig_name);
            std::fs::copy(&path, &orig_path)?;
            std::fs::remove_file(&path)?;
            count += 1;
        }
    }
    Ok(count)
}
