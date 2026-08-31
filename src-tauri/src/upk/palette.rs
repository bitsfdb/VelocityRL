//! Rich color palette unlock for Rocket League `TAGame.upk`.
//!
//! Garage pickers: stack vanilla **BlueTeamV3** (10×7) over **OrangeTeamV3**
//! (10×7) over Accent hues 0–9 (10×7) → HueCount=10, ValueCount=21.
//! Same width as the team grids so there is no empty black gutter. Accent’s
//! extra five hues stay on the stock Accent picker (untouched 15×7).
//! Stored **value-major** (`index = value * HueCount + hue`) — same layout as
//! stock Blue/Orange/Accent on disk and the garage fill order.
//! Written into Blue/Orange V1–V3 only. **Accent is never rewritten**.
//! Grown last-chunk payload is **appended at EOF** so trailing out-of-order
//! chunks (TAGame stores chunk 4 after the last table entry) keep their
//! `CompressedOffset`.

use crate::upk::{compression, crypto, parser};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const BACKUP_SUFFIX: &str = ".vrlpal.orig";

const COLOR_SET_CLASS: &str = "CarColorSet_TA";

const RICH_SET: &str = "Accent";
const SOURCE_BLUE: &str = "BlueTeamV3";
const SOURCE_ORANGE: &str = "OrangeTeamV3";
const SOURCE_BLUE_FALLBACK: &[&str] = &["BlueTeamV2", "BlueTeam"];
const SOURCE_ORANGE_FALLBACK: &[&str] = &["OrangeTeamV2", "OrangeTeam"];

const RICH_TARGETS: &[&str] = &[
    "BlueTeam",
    "BlueTeamV2",
    "BlueTeamV3",
    "OrangeTeam",
    "OrangeTeamV2",
    "OrangeTeamV3",
];

const STACK_SOURCES: &[&str] = &[SOURCE_BLUE, SOURCE_ORANGE, RICH_SET];

const TEAM_HUE_COUNT: i32 = 10;
const ACCENT_HUE_COUNT: i32 = 15;
const SRC_VALUES: i32 = 7;
const STACK_HUE_COUNT: i32 = TEAM_HUE_COUNT;
const STACK_VALUE_COUNT: i32 = SRC_VALUES * 3;
const STOCK_ACCENT_SERIAL: i32 = 10309;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaletteStatus {
    pub applied: bool,
    pub backup_present: bool,
    pub tagame_path: String,
    pub backup_path: String,
    pub fingerprint: String,
    pub message: String,
}

#[derive(Debug)]
pub enum PaletteError {
    Msg(String),
}

impl std::fmt::Display for PaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaletteError::Msg(s) => write!(f, "{s}"),
        }
    }
}

fn tagame_path(game_dir: &Path) -> PathBuf {
    game_dir.join("TAGame.upk")
}

fn backup_path(game_dir: &Path) -> PathBuf {
    let mut p = tagame_path(game_dir);
    let mut name = p
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    name.push_str(BACKUP_SUFFIX);
    p.set_file_name(name);
    p
}

pub fn resolve_cooked_dir(game_dir: &Path) -> Result<PathBuf, PaletteError> {
    if tagame_path(game_dir).exists() {
        return Ok(game_dir.to_path_buf());
    }
    let candidates = [
        game_dir.join("CookedPCConsole"),
        game_dir.join("TAGame").join("CookedPCConsole"),
    ];
    for c in candidates {
        if tagame_path(&c).exists() {
            return Ok(c);
        }
    }
    Err(PaletteError::Msg(format!(
        "TAGame.upk not found under {}. Point Settings at CookedPCConsole (…/TAGame/CookedPCConsole), not the game root.",
        game_dir.display()
    )))
}

pub fn fingerprint_bytes(data: &[u8], name_offset: usize, enc_len: usize) -> String {
    let end = (name_offset + enc_len).min(data.len());
    let slice = &data[name_offset..end];
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in slice {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}:{}", data.len())
}

pub fn file_fingerprint(path: &Path) -> Result<String, PaletteError> {
    let data = std::fs::read(path).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (summary, meta) = parser::parse_prefix(&data)
        .map_err(|e| PaletteError::Msg(format!("parse TAGame: {e}")))?;
    let name_offset = summary.name_offset as usize;
    let enc_size = (summary.total_header_size - meta.garbage_size - summary.name_offset) as usize;
    let enc_aligned = (enc_size + 15) & !15;
    Ok(fingerprint_bytes(&data, name_offset, enc_aligned))
}

fn parse_names_in_block(plain: &[u8], name_count: i32) -> Result<Vec<String>, PaletteError> {
    let mut c = std::io::Cursor::new(plain);
    let mut names = Vec::with_capacity(name_count.max(0) as usize);
    for _ in 0..name_count.max(0) {
        let name = parser::read_fstring(&mut c).map_err(|e| PaletteError::Msg(e.to_string()))?;
        let mut flags = [0u8; 8];
        c.read_exact(&mut flags)
            .map_err(|e| PaletteError::Msg(e.to_string()))?;
        names.push(name);
    }
    Ok(names)
}

struct RawExport {
    pos: usize,
    class_index: i32,
    name_idx: i32,
    serial_size: i32,
    serial_offset: i64,
}

fn walk_exports(plain: &[u8], export_rel: usize, depends_rel: usize) -> Vec<RawExport> {
    let mut out = Vec::new();
    let mut pos = export_rel;
    while pos + 72 <= depends_rel && pos + 72 <= plain.len() && out.len() < 200_000 {
        let i32_at = |a: usize| i32::from_le_bytes(plain[pos + a..pos + a + 4].try_into().unwrap());
        let noc = i32_at(48);
        out.push(RawExport {
            pos,
            class_index: i32_at(0),
            name_idx: i32_at(12),
            serial_size: i32_at(32),
            serial_offset: i64::from_le_bytes(plain[pos + 36..pos + 44].try_into().unwrap()),
        });
        pos += 72 + (noc.max(0) as usize) * 4;
    }
    out
}

fn parse_import_names(plain: &[u8], summary: &parser::FileSummary, names: &[String]) -> Vec<String> {
    let base = (summary.import_offset - summary.name_offset) as usize;
    (0..summary.import_count.max(0) as usize)
        .map_while(|i| {
            let p = base + i * 28;
            let raw = plain.get(p + 20..p + 24)?;
            let idx = i32::from_le_bytes(raw.try_into().unwrap());
            Some(name_at(names, idx))
        })
        .collect()
}

fn name_at(names: &[String], idx: i32) -> String {
    names.get(idx.max(0) as usize).cloned().unwrap_or_default()
}

fn color_sets(
    plain: &[u8],
    summary: &parser::FileSummary,
) -> Result<HashMap<String, (usize, i32, i64)>, PaletteError> {
    let names = parse_names_in_block(plain, summary.name_count)?;
    let imports = parse_import_names(plain, summary, &names);
    let export_rel = (summary.export_offset - summary.name_offset) as usize;
    let depends_rel = (summary.depends_offset - summary.name_offset) as usize;
    let exports = walk_exports(plain, export_rel, depends_rel);

    let class_name = |e: &RawExport| -> String {
        if e.class_index > 0 {
            exports
                .get((e.class_index - 1) as usize)
                .map(|c| name_at(&names, c.name_idx))
                .unwrap_or_default()
        } else if e.class_index < 0 {
            imports
                .get((-e.class_index - 1) as usize)
                .cloned()
                .unwrap_or_default()
        } else {
            String::new()
        }
    };

    let mut out = HashMap::new();
    for e in &exports {
        if class_name(e) != COLOR_SET_CLASS {
            continue;
        }
        out.entry(name_at(&names, e.name_idx))
            .or_insert((e.pos, e.serial_size, e.serial_offset));
    }
    Ok(out)
}

fn decrypt_tagame(
    data: &[u8],
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<(parser::FileSummary, parser::CompressionMeta, Vec<u8>, [u8; 32], usize), PaletteError>
{
    let (summary, meta) = parser::parse_prefix(data)
        .map_err(|e| PaletteError::Msg(format!("parse: {e}")))?;
    let name_offset = summary.name_offset as usize;
    let enc_size = (summary.total_header_size - meta.garbage_size - summary.name_offset) as usize;
    let enc_aligned = (enc_size + 15) & !15;
    if name_offset + enc_aligned > data.len() {
        return Err(PaletteError::Msg("encrypted block OOB".into()));
    }
    let enc_block = &data[name_offset..name_offset + enc_aligned];
    let all_keys = crypto::load_keys(keys_txt);
    let keys_map = crypto::load_keys_map(keys_map_json);
    let map_key = keys_map
        .get("tagame")
        .copied()
        .or_else(|| keys_map.get("TAGame").copied());
    let key = map_key
        .and_then(|k| {
            crypto::find_valid_key_relaxed(enc_block, meta.compressed_chunks_offset, &[k])
        })
        .or_else(|| {
            crypto::find_valid_key(
                enc_block,
                summary.depends_offset,
                meta.compressed_chunks_offset,
                &all_keys,
            )
        })
        .ok_or_else(|| {
            PaletteError::Msg(
                "Can't decrypt TAGame.upk (AES key not in bundled keys). Repair game files or update the app."
                    .into(),
            )
        })?;
    Ok((
        summary,
        meta,
        crypto::decrypt_ecb(&key, enc_block),
        key,
        enc_aligned,
    ))
}

fn missing_sets_error(sets: &HashMap<String, (usize, i32, i64)>) -> PaletteError {
    let mut found: Vec<&str> = sets.keys().map(|s| s.as_str()).collect();
    found.sort_unstable();
    let found_s = if found.is_empty() {
        "(none)".into()
    } else {
        found.join(", ")
    };
    PaletteError::Msg(format!(
        "Color sets not found in TAGame.upk (need {RICH_SET} + the BlueTeam/OrangeTeam sets). Found: {found_s}"
    ))
}

fn is_broken_alias(sets: &HashMap<String, (usize, i32, i64)>) -> bool {
    let Some(&(_, _, rich_off)) = sets.get(RICH_SET) else {
        return false;
    };
    RICH_TARGETS.iter().any(|t| {
        sets.get(*t)
            .map(|&(_, _, off)| off == rich_off)
            .unwrap_or(false)
    })
}

/// Previous Accent-only design: team exports are unique copies of *stock-sized*
/// Accent (10309). Accent itself stays 10309.
fn is_accent_copy(sets: &HashMap<String, (usize, i32, i64)>) -> bool {
    let Some(&(_, rich_size, rich_off)) = sets.get(RICH_SET) else {
        return false;
    };
    if rich_size != STOCK_ACCENT_SERIAL {
        return false;
    }
    let mut matched = 0usize;
    let mut seen = HashSet::new();
    seen.insert(rich_off);
    for target in RICH_TARGETS {
        let Some(&(_, size, off)) = sets.get(*target) else {
            continue;
        };
        if size != rich_size || off == rich_off || !seen.insert(off) {
            return false;
        }
        matched += 1;
    }
    matched > 0
}

/// 10×21 stack: team exports unique and larger than stock Accent; Accent stays stock.
fn is_combined(sets: &HashMap<String, (usize, i32, i64)>) -> bool {
    let Some(&(_, rich_size, rich_off)) = sets.get(RICH_SET) else {
        return false;
    };
    if rich_size != STOCK_ACCENT_SERIAL {
        return false;
    }
    let mut matched = 0usize;
    let mut seen = HashSet::new();
    seen.insert(rich_off);
    for target in RICH_TARGETS {
        let Some(&(_, size, off)) = sets.get(*target) else {
            continue;
        };
        if size <= STOCK_ACCENT_SERIAL || off == rich_off || !seen.insert(off) {
            return false;
        }
        matched += 1;
    }
    matched == RICH_TARGETS.len()
}

/// Older remap: several Blue*/Orange* exports share one serial offset (e.g. all → V3).
fn teams_share_serial(sets: &HashMap<String, (usize, i32, i64)>) -> bool {
    let mut seen = HashSet::new();
    for target in RICH_TARGETS {
        let Some(&(_, _, off)) = sets.get(*target) else {
            continue;
        };
        if !seen.insert(off) {
            return true;
        }
    }
    false
}

fn is_remapped(sets: &HashMap<String, (usize, i32, i64)>) -> bool {
    is_combined(sets) || teams_share_serial(sets) || is_accent_copy(sets) || is_broken_alias(sets)
}

/// True when `needle` swatches appear as a contiguous 16-byte-aligned block in `haystack`.
#[allow(dead_code)]
fn colors_payload_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() || needle.len() % 16 != 0 {
        return false;
    }
    let mut off = 0usize;
    while off + needle.len() <= haystack.len() {
        if haystack[off..off + needle.len()] == *needle {
            return true;
        }
        off += 16;
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveKind {
    BrokenAlias,
    AccentCopy,
    /// Team exports hold the 10×21 Blue/Orange/Accent-head stack; Accent is stock 15×7.
    Applied,
    /// Larger remaps that are not the current 10×21 stack.
    StaleRemap,
    /// Stock / restored structure (not the rich remap).
    Vanilla,
}

/// Inspect live export sizes + swatch payloads. Never uses backup presence as a proxy.
fn classify_live_sets(
    file: &[u8],
    plain: &[u8],
    summary: &parser::FileSummary,
    meta: &parser::CompressionMeta,
    sets: &HashMap<String, (usize, i32, i64)>,
) -> LiveKind {
    if is_broken_alias(sets) {
        return LiveKind::BrokenAlias;
    }
    if is_accent_copy(sets) {
        return LiveKind::AccentCopy;
    }
    if teams_share_serial(sets) {
        return LiveKind::StaleRemap;
    }

    match classify_via_swatches(file, plain, summary, meta, sets) {
        Ok(kind) => kind,
        // Size-only "combined" is not Applied — classify via swatches.
        Err(_) if is_combined(sets) => LiveKind::StaleRemap,
        Err(_) => LiveKind::Vanilla,
    }
}

fn classify_via_swatches(
    file: &[u8],
    plain: &[u8],
    summary: &parser::FileSummary,
    meta: &parser::CompressionMeta,
    sets: &HashMap<String, (usize, i32, i64)>,
) -> Result<LiveKind, PaletteError> {
    let names = parse_names_in_block(plain, summary.name_count)?;
    let chunks = parser::parse_chunks(plain, meta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("chunk table: {e}")))?;

    let Some(&(_, accent_size, accent_off)) = sets.get(RICH_SET) else {
        return Err(missing_sets_error(sets));
    };
    if accent_size <= 0 {
        return Err(PaletteError::Msg("Accent serial size invalid".into()));
    }
    let accent_bytes = read_serial_bytes(file, &chunks, accent_off, accent_size as usize)?;
    let accent = extract_swatches(&accent_bytes, &names, RICH_SET)?;

    let blue_name = pick_source_name(sets, SOURCE_BLUE, SOURCE_BLUE_FALLBACK)?;
    let orange_name = pick_source_name(sets, SOURCE_ORANGE, SOURCE_ORANGE_FALLBACK)?;
    let &(_, blue_size, blue_off) = sets
        .get(&blue_name)
        .ok_or_else(|| PaletteError::Msg("blue source missing".into()))?;
    let &(_, orange_size, orange_off) = sets
        .get(&orange_name)
        .ok_or_else(|| PaletteError::Msg("orange source missing".into()))?;
    let blue = extract_swatches(
        &read_serial_bytes(file, &chunks, blue_off, blue_size as usize)?,
        &names,
        &blue_name,
    )?;
    let orange = extract_swatches(
        &read_serial_bytes(file, &chunks, orange_off, orange_size as usize)?,
        &names,
        &orange_name,
    )?;

    let teams_same = blue.colors_payload == orange.colors_payload;
    if teams_same
        && is_full_stack(&blue)
        && accent.hue_count == ACCENT_HUE_COUNT
        && accent.value_count == SRC_VALUES
    {
        return Ok(LiveKind::Applied);
    }
    if is_combined(sets)
        || blue.value_count != SRC_VALUES
        || accent.hue_count != ACCENT_HUE_COUNT
        || accent.value_count != SRC_VALUES
    {
        return Ok(LiveKind::StaleRemap);
    }

    Ok(LiveKind::Vanilla)
}


/// Chunk table indices whose compressed payload starts at or after `payload_end`.
/// TAGame stores chunk 4 *after* the last table entry; splicing a larger last
/// payload used to move those bytes without updating this offset.
fn later_compressed_chunks(chunks: &[parser::CompressedChunk], payload_end: i64) -> Vec<usize> {
    chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.compressed_offset >= payload_end)
        .map(|(i, _)| i)
        .collect()
}

fn write_chunk_entry(
    plain: &mut [u8],
    table_off: usize,
    idx: usize,
    stride: usize,
    uncompressed_size: i32,
    compressed_offset: i64,
    compressed_size: i32,
) -> Result<(), PaletteError> {
    let stride_off = idx
        .checked_mul(stride)
        .ok_or_else(|| PaletteError::Msg("chunk table entry overflow".into()))?;
    let entry_off = table_off
        .checked_add(4)
        .and_then(|o| o.checked_add(stride_off))
        .ok_or_else(|| PaletteError::Msg("chunk table entry overflow".into()))?;
    if entry_off + 24 > plain.len() {
        return Err(PaletteError::Msg("chunk table entry OOB".into()));
    }
    plain[entry_off + 8..entry_off + 12].copy_from_slice(&uncompressed_size.to_le_bytes());
    plain[entry_off + 12..entry_off + 20].copy_from_slice(&compressed_offset.to_le_bytes());
    plain[entry_off + 20..entry_off + 24].copy_from_slice(&compressed_size.to_le_bytes());
    Ok(())
}

fn validate_chunk_layout(
    file: &[u8],
    chunks: &[parser::CompressedChunk],
) -> Result<(), PaletteError> {
    let mut ranges: Vec<(i64, i64, usize)> = Vec::with_capacity(chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        if c.compressed_offset < 0 || c.uncompressed_offset < 0 {
            return Err(PaletteError::Msg(format!("chunk {i} has a negative offset")));
        }
        if c.compressed_size <= 0 || c.uncompressed_size <= 0 {
            return Err(PaletteError::Msg(format!("chunk {i} has a non-positive size")));
        }
        let start = c.compressed_offset as usize;
        let end = start
            .checked_add(c.compressed_size as usize)
            .ok_or_else(|| PaletteError::Msg(format!("chunk {i} size overflow")))?;
        if end > file.len() {
            return Err(PaletteError::Msg(format!(
                "chunk {i} payload OOB: offset {} size {} file {}",
                c.compressed_offset,
                c.compressed_size,
                file.len()
            )));
        }
        if start + 4 > file.len() {
            return Err(PaletteError::Msg(format!("chunk {i} missing magic")));
        }
        let magic = u32::from_le_bytes(file[start..start + 4].try_into().unwrap());
        if magic != parser::PACKAGE_FILE_TAG {
            return Err(PaletteError::Msg(format!(
                "chunk {i} missing compression magic at offset {}",
                c.compressed_offset
            )));
        }
        ranges.push((c.compressed_offset, c.compressed_offset + c.compressed_size as i64, i));
    }
    ranges.sort_by_key(|r| r.0);
    for w in ranges.windows(2) {
        if w[1].0 < w[0].1 {
            return Err(PaletteError::Msg(format!(
                "chunk {} compressed payload overlaps chunk {}",
                w[1].2, w[0].2
            )));
        }
    }
    Ok(())
}

fn validate_applied(
    file: &[u8],
    keys_txt: &str,
    keys_map_json: &str,
    expected: &HashMap<String, Vec<u8>>,
) -> Result<(), PaletteError> {
    let (summary, meta, plain, _, _) = decrypt_tagame(file, keys_txt, keys_map_json)?;
    let chunks = parser::parse_chunks(&plain, meta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("validate chunk table: {e}")))?;
    validate_chunk_layout(file, &chunks)?;

    let last = chunks
        .last()
        .ok_or_else(|| PaletteError::Msg("validate: no chunks".into()))?;
    let start = last.compressed_offset as usize;
    let end = start + last.compressed_size as usize;
    let last_uncomp = compression::decompress_chunk(&file[start..end])
        .map_err(|e| PaletteError::Msg(format!("validate decompress last: {e}")))?;
    if last_uncomp.len() != last.uncompressed_size as usize {
        return Err(PaletteError::Msg(format!(
            "last chunk serial size mismatch: decompressed {} expected {}",
            last_uncomp.len(),
            last.uncompressed_size
        )));
    }

    let sets = color_sets(&plain, &summary)?;
    if is_broken_alias(&sets) {
        return Err(PaletteError::Msg(
            "apply produced a shared-Accent remap; refusing to write".into(),
        ));
    }
    if !is_combined(&sets) {
        return Err(PaletteError::Msg(
            "apply did not produce unique combined palettes".into(),
        ));
    }
    for (target, expected_bytes) in expected {
        let Some(&(_, size, off)) = sets.get(target.as_str()) else {
            return Err(PaletteError::Msg(format!("{target} missing after apply")));
        };
        if size as usize != expected_bytes.len() {
            return Err(PaletteError::Msg(format!(
                "{target} serial size {size} != written {}",
                expected_bytes.len()
            )));
        }
        let copy = read_serial_bytes(file, &chunks, off, size as usize)?;
        if copy != *expected_bytes {
            return Err(PaletteError::Msg(format!(
                "{target} serial bytes do not match written combined palette"
            )));
        }
    }
    Ok(())
}

fn replace_file(from: &Path, to: &Path) -> Result<(), PaletteError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        fn wide(p: &Path) -> Vec<u16> {
            p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
        }
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let src = wide(from);
        let dst = wide(to);
        let ok = unsafe {
            MoveFileExW(
                src.as_ptr(),
                dst.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            return Err(PaletteError::Msg(format!(
                "atomic replace failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to).map_err(|e| PaletteError::Msg(format!("atomic replace failed: {e}")))
    }
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), PaletteError> {
    let mut name = path
        .file_name()
        .unwrap_or_default()
        .to_os_string();
    name.push(".vrlpal.tmp");
    let tmp = path.with_file_name(name);
    std::fs::write(&tmp, data).map_err(|e| PaletteError::Msg(format!("temp write failed: {e}")))?;
    match replace_file(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn refuse_if_game_running(action: &str) -> Result<(), PaletteError> {
    if let Some(who) = crate::psynet::rocket_league_lock_holder() {
        return Err(PaletteError::Msg(format!(
            "Rocket League is running ({who}). Close it, then {action}."
        )));
    }
    Ok(())
}

fn hex_head(data: &[u8], n: usize) -> String {
    data.iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fname_in_serial(serial: &[u8], pos: usize, names: &[String]) -> Option<(String, i32, i32)> {
    if pos + 8 > serial.len() {
        return None;
    }
    let idx = i32::from_le_bytes(serial[pos..pos + 4].try_into().ok()?);
    let num = i32::from_le_bytes(serial[pos + 4..pos + 8].try_into().ok()?);
    Some((name_at(names, idx), idx, num))
}

fn dump_one_serial(name: &str, size: i32, off: i64, serial: &[u8], names: &[String]) -> String {
    let mut s = format!("{name} serial_size={size} offset={off} bytes={}\n", serial.len());
    s.push_str(&format!("  head: {}\n", hex_head(serial, 96.min(serial.len()))));
    if serial.len() > 16 {
        s.push_str(&format!(
            "  tail: {}\n",
            hex_head(&serial[serial.len().saturating_sub(16)..], 16)
        ));
    }
    match parse_export_serial(serial, names) {
        Ok(parsed) => {
            let rebuilt = write_export_serial(&parsed);
            s.push_str(&format!(
                "  roundtrip_ok={} rebuilt_len={}\n",
                rebuilt == serial,
                rebuilt.len()
            ));
        }
        Err(e) => s.push_str(&format!("  parse_err: {e}\n")),
    }
    for start in [0usize, 4] {
        s.push_str(&format!("  walk from {start}:\n"));
        match walk_props(serial, start, names) {
            Ok(lines) => s.push_str(&lines),
            Err(e) => s.push_str(&format!("    err: {e}\n")),
        }
    }
    s
}

fn walk_props(serial: &[u8], mut pos: usize, names: &[String]) -> Result<String, PaletteError> {
    let mut out = String::new();
    for step in 0..64 {
        if pos + 8 > serial.len() {
            out.push_str(&format!("    eof at {pos} after {step} props\n"));
            break;
        }
        let (pname, pidx, pnum) =
            fname_in_serial(serial, pos, names).ok_or_else(|| PaletteError::Msg("fname".into()))?;
        pos += 8;
        if pname == "None" {
            out.push_str(&format!(
                "    None idx={pidx} num={pnum} pos={pos} remain={}\n",
                serial.len() - pos
            ));
            break;
        }
        if pos + 16 > serial.len() {
            return Err(PaletteError::Msg(format!(
                "truncated tag for {pname} at {pos}"
            )));
        }
        let (tname, tidx, tnum) =
            fname_in_serial(serial, pos, names).ok_or_else(|| PaletteError::Msg("type fname".into()))?;
        pos += 8;
        let sz = i32::from_le_bytes(serial[pos..pos + 4].try_into().unwrap());
        let arr = i32::from_le_bytes(serial[pos + 4..pos + 8].try_into().unwrap());
        pos += 8;
        out.push_str(&format!(
            "    {pname} ({pidx}/{pnum}) {tname} ({tidx}/{tnum}) size={sz} arr={arr} body@{pos}\n"
        ));
        if tname == "StructProperty" || tname == "ByteProperty" {
            if pos + 8 > serial.len() {
                return Err(PaletteError::Msg(format!("{tname} missing inner name")));
            }
            let (inner, iidx, inum) = fname_in_serial(serial, pos, names).unwrap();
            pos += 8;
            out.push_str(&format!("      inner {inner} ({iidx}/{inum})\n"));
        }
        if tname == "BoolProperty" {
            if pos >= serial.len() {
                return Err(PaletteError::Msg("bool OOB".into()));
            }
            out.push_str(&format!("      bool={}\n", serial[pos]));
            pos += 1;
            continue;
        }
        if sz < 0 {
            return Err(PaletteError::Msg(format!("negative size {sz}")));
        }
        let n = sz as usize;
        if pos + n > serial.len() {
            return Err(PaletteError::Msg(format!(
                "{pname} size {n} OOB at {pos}/{}",
                serial.len()
            )));
        }
        let body = &serial[pos..pos + n];
        if tname == "IntProperty" && n == 4 {
            let v = i32::from_le_bytes(body.try_into().unwrap());
            out.push_str(&format!("      int={v}\n"));
        } else if tname == "FloatProperty" && n == 4 {
            let v = f32::from_le_bytes(body.try_into().unwrap());
            out.push_str(&format!("      float={v}\n"));
        } else if tname == "ArrayProperty" && n >= 4 {
            let count = i32::from_le_bytes(body[..4].try_into().unwrap());
            let rest = n - 4;
            out.push_str(&format!(
                "      count={count} payload={rest} /16={} /4={} /20={} /24={}\n",
                rest as f32 / 16.0,
                rest as f32 / 4.0,
                rest as f32 / 20.0,
                rest as f32 / 24.0
            ));
            if rest >= 16 {
                out.push_str(&format!("      first16: {}\n", hex_head(&body[4..20.min(body.len())], 16)));
            }
        } else {
            out.push_str(&format!("      raw: {}\n", hex_head(body, 32.min(body.len()))));
        }
        pos += n;
    }
    Ok(out)
}

/// Inspect `CarColorSet_TA` serials in a TAGame.upk (live or backup).
pub fn dump_color_sets(
    upk_path: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<String, PaletteError> {
    let data = std::fs::read(upk_path).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (summary, meta, plain, _, _) = decrypt_tagame(&data, keys_txt, keys_map_json)?;
    let names = parse_names_in_block(&plain, summary.name_count)?;
    let sets = color_sets(&plain, &summary)?;
    let chunks = parser::parse_chunks(&plain, meta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("chunk table: {e}")))?;
    let mut names_of_interest: Vec<String> = sets.keys().cloned().collect();
    names_of_interest.sort();
    let mut all_names: Vec<String> = sets.keys().cloned().collect();
    all_names.sort();
    let mut out = format!(
        "file {} {} bytes, {} color sets: {}\n",
        upk_path.display(),
        data.len(),
        sets.len(),
        all_names.join(", ")
    );
    out.push_str("COUNTS:\n");
    for n in &names_of_interest {
        let Some(&(_, size, off)) = sets.get(n) else {
            continue;
        };
        match read_serial_bytes(&data, &chunks, off, size.max(0) as usize) {
            Ok(serial) => match extract_swatches(&serial, &names, n) {
                Ok(sw) => out.push_str(&format!(
                    "  {n}: HueCount={} ValueCount={} Colors={}\n",
                    sw.hue_count, sw.value_count, sw.color_count
                )),
                Err(e) => out.push_str(&format!("  {n}: (unparsed {e})\n")),
            },
            Err(e) => out.push_str(&format!("  {n}: size={size} off={off} READ ERR {e}\n")),
        }
    }
    out.push('\n');
    for n in &names_of_interest {
        let Some(&( _pos, size, off)) = sets.get(n) else {
            continue;
        };
        match read_serial_bytes(&data, &chunks, off, size.max(0) as usize) {
            Ok(serial) => out.push_str(&dump_one_serial(n, size, off, &serial, &names)),
            Err(e) => out.push_str(&format!("{n} size={size} off={off} READ ERR {e}\n")),
        }
    }
    Ok(out)
}

fn read_serial_bytes(
    file: &[u8],
    chunks: &[parser::CompressedChunk],
    off: i64,
    size: usize,
) -> Result<Vec<u8>, PaletteError> {
    for c in chunks {
        let end = c.uncompressed_offset + c.uncompressed_size as i64;
        if off < c.uncompressed_offset || off + size as i64 > end {
            continue;
        }
        let start = c.compressed_offset as usize;
        let cend = start + c.compressed_size as usize;
        if cend > file.len() {
            return Err(PaletteError::Msg("chunk payload OOB".into()));
        }
        let buf = compression::decompress_chunk(&file[start..cend])
            .map_err(|e| PaletteError::Msg(format!("decompress: {e}")))?;
        let local = (off - c.uncompressed_offset) as usize;
        if local + size > buf.len() {
            return Err(PaletteError::Msg("serial slice OOB in chunk".into()));
        }
        return Ok(buf[local..local + size].to_vec());
    }
    Err(PaletteError::Msg(format!(
        "serial offset {off} size {size} not in any compressed chunk"
    )))
}

#[derive(Clone)]
struct TaggedProp {
    name_idx: i32,
    name_num: i32,
    type_idx: i32,
    type_num: i32,
    size: i32,
    array_index: i32,
    inner: Option<(i32, i32)>,
    body: Vec<u8>,
    bool_value: Option<u8>,
}

#[derive(Clone)]
struct ExportSerial {
    net_index: i32,
    props: Vec<TaggedProp>,
    none_idx: i32,
    none_num: i32,
}

fn type_name_of(p: &TaggedProp, names: &[String]) -> String {
    name_at(names, p.type_idx)
}

fn prop_name_of(p: &TaggedProp, names: &[String]) -> String {
    name_at(names, p.name_idx)
}

fn parse_export_serial(serial: &[u8], names: &[String]) -> Result<ExportSerial, PaletteError> {
    if serial.len() < 12 {
        return Err(PaletteError::Msg("color set serial too short".into()));
    }
    let net_index = i32::from_le_bytes(serial[0..4].try_into().unwrap());
    let mut pos = 4usize;
    let mut props = Vec::new();
    let mut none_idx = -1i32;
    let mut none_num = 0i32;
    for _ in 0..64 {
        let (pname, pidx, pnum) = fname_in_serial(serial, pos, names)
            .ok_or_else(|| PaletteError::Msg("truncated property name".into()))?;
        pos += 8;
        if pname == "None" {
            none_idx = pidx;
            none_num = pnum;
            break;
        }
        if pos + 16 > serial.len() {
            return Err(PaletteError::Msg(format!("truncated tag for {pname}")));
        }
        let (tname, tidx, tnum) = fname_in_serial(serial, pos, names)
            .ok_or_else(|| PaletteError::Msg("truncated property type".into()))?;
        pos += 8;
        let size = i32::from_le_bytes(serial[pos..pos + 4].try_into().unwrap());
        let array_index = i32::from_le_bytes(serial[pos + 4..pos + 8].try_into().unwrap());
        pos += 8;
        let mut inner = None;
        if tname == "StructProperty" || tname == "ByteProperty" {
            let (_in, iidx, inum) = fname_in_serial(serial, pos, names)
                .ok_or_else(|| PaletteError::Msg(format!("{tname} missing inner name")))?;
            pos += 8;
            inner = Some((iidx, inum));
        }
        let bool_value;
        let body;
        if tname == "BoolProperty" {
            if pos >= serial.len() {
                return Err(PaletteError::Msg("bool OOB".into()));
            }
            bool_value = Some(serial[pos]);
            pos += 1;
            body = Vec::new();
        } else {
            if size < 0 {
                return Err(PaletteError::Msg(format!("{pname} negative size")));
            }
            let n = size as usize;
            if pos + n > serial.len() {
                return Err(PaletteError::Msg(format!("{pname} body OOB")));
            }
            body = serial[pos..pos + n].to_vec();
            pos += n;
            bool_value = None;
        }
        props.push(TaggedProp {
            name_idx: pidx,
            name_num: pnum,
            type_idx: tidx,
            type_num: tnum,
            size,
            array_index,
            inner,
            body,
            bool_value,
        });
    }
    if none_idx < 0 {
        return Err(PaletteError::Msg("color set serial missing None terminator".into()));
    }
    if pos != serial.len() {
        return Err(PaletteError::Msg(format!(
            "color set serial trailing bytes: pos={pos} len={}",
            serial.len()
        )));
    }
    Ok(ExportSerial {
        net_index,
        props,
        none_idx,
        none_num,
    })
}

fn push_fname(buf: &mut Vec<u8>, idx: i32, num: i32) {
    buf.extend_from_slice(&idx.to_le_bytes());
    buf.extend_from_slice(&num.to_le_bytes());
}

fn write_export_serial(ser: &ExportSerial) -> Vec<u8> {
    let mut out = Vec::with_capacity(ser.props.len() * 64);
    out.extend_from_slice(&ser.net_index.to_le_bytes());
    for p in &ser.props {
        push_fname(&mut out, p.name_idx, p.name_num);
        push_fname(&mut out, p.type_idx, p.type_num);
        out.extend_from_slice(&p.size.to_le_bytes());
        out.extend_from_slice(&p.array_index.to_le_bytes());
        if let Some((iidx, inum)) = p.inner {
            push_fname(&mut out, iidx, inum);
        }
        if let Some(b) = p.bool_value {
            out.push(b);
        } else {
            out.extend_from_slice(&p.body);
        }
    }
    push_fname(&mut out, ser.none_idx, ser.none_num);
    out
}

fn find_prop_mut<'a>(
    ser: &'a mut ExportSerial,
    names: &[String],
    want: &str,
) -> Result<&'a mut TaggedProp, PaletteError> {
    ser.props
        .iter_mut()
        .find(|p| prop_name_of(p, names) == want)
        .ok_or_else(|| PaletteError::Msg(format!("property {want} not found in color set")))
}

fn read_int_prop(p: &TaggedProp, names: &[String]) -> Result<i32, PaletteError> {
    if type_name_of(p, names) != "IntProperty" || p.body.len() != 4 {
        return Err(PaletteError::Msg(format!(
            "{} is not IntProperty",
            prop_name_of(p, names)
        )));
    }
    Ok(i32::from_le_bytes(p.body[..4].try_into().unwrap()))
}

fn set_int_prop(p: &mut TaggedProp, value: i32) {
    p.size = 4;
    p.body = value.to_le_bytes().to_vec();
    p.bool_value = None;
}

fn read_array_prop<'a>(
    p: &'a TaggedProp,
    names: &[String],
) -> Result<(i32, &'a [u8]), PaletteError> {
    if type_name_of(p, names) != "ArrayProperty" || p.body.len() < 4 {
        return Err(PaletteError::Msg(format!(
            "{} is not ArrayProperty",
            prop_name_of(p, names)
        )));
    }
    let count = i32::from_le_bytes(p.body[..4].try_into().unwrap());
    Ok((count, &p.body[4..]))
}

fn set_array_prop(p: &mut TaggedProp, count: i32, payload: &[u8]) -> Result<(), PaletteError> {
    let mut body = Vec::with_capacity(4 + payload.len());
    body.extend_from_slice(&count.to_le_bytes());
    body.extend_from_slice(payload);
    p.size = i32::try_from(body.len())
        .map_err(|_| PaletteError::Msg("array property size exceeds i32".into()))?;
    p.body = body;
    p.bool_value = None;
    Ok(())
}

struct SwatchSource {
    color_count: usize,
    colors_payload: Vec<u8>,
    debug_payload: Vec<u8>,
    debug_elem: usize,
    hue_count: i32,
    value_count: i32,
}

fn extract_swatches(
    serial: &[u8],
    names: &[String],
    label: &str,
) -> Result<SwatchSource, PaletteError> {
    let ser = parse_export_serial(serial, names)?;
    let colors_p = ser
        .props
        .iter()
        .find(|p| prop_name_of(p, names) == "Colors")
        .ok_or_else(|| PaletteError::Msg(format!("{label}: missing Colors")))?;
    let debug_p = ser
        .props
        .iter()
        .find(|p| prop_name_of(p, names) == "DebugColors")
        .ok_or_else(|| PaletteError::Msg(format!("{label}: missing DebugColors")))?;
    let hue_p = ser
        .props
        .iter()
        .find(|p| prop_name_of(p, names) == "HueCount")
        .ok_or_else(|| PaletteError::Msg(format!("{label}: missing HueCount")))?;
    let value_p = ser
        .props
        .iter()
        .find(|p| prop_name_of(p, names) == "ValueCount")
        .ok_or_else(|| PaletteError::Msg(format!("{label}: missing ValueCount")))?;

    let (color_count, colors_payload): (i32, &[u8]) = read_array_prop(colors_p, names)?;
    let (debug_count, debug_payload): (i32, &[u8]) = read_array_prop(debug_p, names)?;
    let hue_count = read_int_prop(hue_p, names)?;
    let value_count = read_int_prop(value_p, names)?;

    if color_count <= 0 || debug_count <= 0 {
        return Err(PaletteError::Msg(format!("{label}: empty color arrays")));
    }
    if color_count != debug_count {
        return Err(PaletteError::Msg(format!(
            "{label}: Colors ({color_count}) != DebugColors ({debug_count})"
        )));
    }
    if colors_payload.len() != color_count as usize * 16 {
        return Err(PaletteError::Msg(format!(
            "{label}: Colors payload {} != {}*16",
            colors_payload.len(),
            color_count
        )));
    }
    if debug_payload.is_empty() || debug_payload.len() % debug_count as usize != 0 {
        return Err(PaletteError::Msg(format!(
            "{label}: DebugColors payload {} not divisible by {debug_count}",
            debug_payload.len()
        )));
    }
    let debug_elem = debug_payload.len() / debug_count as usize;
    if hue_count > 0
        && value_count > 0
        && (hue_count as i64) * (value_count as i64) != color_count as i64
    {
        return Err(PaletteError::Msg(format!(
            "{label}: HueCount ({hue_count}) * ValueCount ({value_count}) != Colors ({color_count})"
        )));
    }
    Ok(SwatchSource {
        color_count: color_count as usize,
        colors_payload: colors_payload.to_vec(),
        debug_payload: debug_payload.to_vec(),
        debug_elem,
        hue_count,
        value_count,
    })
}

fn pick_source_name(
    sets: &HashMap<String, (usize, i32, i64)>,
    primary: &str,
    fallbacks: &[&str],
) -> Result<String, PaletteError> {
    if sets.contains_key(primary) {
        return Ok(primary.to_string());
    }
    for f in fallbacks {
        if sets.contains_key(*f) {
            return Ok((*f).to_string());
        }
    }
    Err(PaletteError::Msg(format!(
        "Missing primary color set {primary} (and fallbacks)"
    )))
}

struct CombinedSwatches {
    hue_count: i32,
    value_count: i32,
    color_count: i32,
    colors_payload: Vec<u8>,
    debug_payload: Vec<u8>,
}

fn source_hues(s: &SwatchSource) -> Result<i32, PaletteError> {
    if s.value_count <= 0 || s.hue_count <= 0 {
        return Err(PaletteError::Msg("HueCount/ValueCount invalid".into()));
    }
    if (s.hue_count as i64) * (s.value_count as i64) != s.color_count as i64 {
        return Err(PaletteError::Msg(format!(
            "HueCount ({}) * ValueCount ({}) != Colors ({})",
            s.hue_count, s.value_count, s.color_count
        )));
    }
    Ok(s.hue_count)
}

fn value_major_off(hue: i32, value: i32, hue_count: i32, elem: usize) -> Result<usize, PaletteError> {
    let i = (value as usize)
        .checked_mul(hue_count as usize)
        .and_then(|n| n.checked_add(hue as usize))
        .ok_or_else(|| PaletteError::Msg("value-major index overflow".into()))?;
    i.checked_mul(elem)
        .ok_or_else(|| PaletteError::Msg("value-major offset overflow".into()))
}

fn extend_cell(out: &mut Vec<u8>, src: &[u8], off: usize, elem: usize) -> Result<(), PaletteError> {
    let end = off
        .checked_add(elem)
        .ok_or_else(|| PaletteError::Msg("cell range overflow".into()))?;
    if end > src.len() {
        return Err(PaletteError::Msg("cell OOB in source palette".into()));
    }
    out.extend_from_slice(&src[off..end]);
    Ok(())
}

/// Copy one swatch from a value-major source (stock / stacked) into a value-major build.
fn push_from_value_major(
    out: &mut Vec<u8>,
    src: &[u8],
    src_hues: i32,
    src_values: i32,
    hue: i32,
    value: i32,
    elem: usize,
) -> Result<(), PaletteError> {
    if hue < 0 || hue >= src_hues || value < 0 || value >= src_values {
        return Err(PaletteError::Msg("source cell OOB".into()));
    }
    let off = value_major_off(hue, value, src_hues, elem)?;
    extend_cell(out, src, off, elem)
}

/// Pull a value-band (optionally fewer hues) keeping value-major order.
fn extract_value_major_band(
    payload: &[u8],
    src_hues: i32,
    src_values: i32,
    value_start: i32,
    band_values: i32,
    elem: usize,
    take_hues: i32,
) -> Result<Vec<u8>, PaletteError> {
    if take_hues <= 0 || take_hues > src_hues || value_start < 0 || band_values <= 0 {
        return Err(PaletteError::Msg("invalid value band".into()));
    }
    let band_end = value_start
        .checked_add(band_values)
        .ok_or_else(|| PaletteError::Msg("value band overflow".into()))?;
    if band_end > src_values {
        return Err(PaletteError::Msg("value band OOB".into()));
    }
    let mut out = Vec::with_capacity(take_hues as usize * band_values as usize * elem);
    for v in 0..band_values {
        for h in 0..take_hues {
            let off = value_major_off(h, value_start + v, src_hues, elem)?;
            extend_cell(&mut out, payload, off, elem)?;
        }
    }
    Ok(out)
}

/// True when `team` is the 10×21 BlueV3 + OrangeV3 + Accent-head stack.
fn is_full_stack(team: &SwatchSource) -> bool {
    if team.hue_count != STACK_HUE_COUNT || team.value_count != STACK_VALUE_COUNT {
        return false;
    }
    (team.hue_count as i64) * (team.value_count as i64) == team.color_count as i64
}

/// True when live stack bands match vanilla Blue / Orange / Accent hues 0–9.
fn is_three_band_stack(
    team: &SwatchSource,
    blue: &SwatchSource,
    orange: &SwatchSource,
    accent: &SwatchSource,
) -> bool {
    if !is_full_stack(team) {
        return false;
    }
    if blue.hue_count != TEAM_HUE_COUNT
        || orange.hue_count != TEAM_HUE_COUNT
        || accent.hue_count != ACCENT_HUE_COUNT
        || blue.value_count != SRC_VALUES
        || orange.value_count != SRC_VALUES
        || accent.value_count != SRC_VALUES
    {
        return false;
    }
    let top = match extract_value_major_band(
        &team.colors_payload,
        team.hue_count,
        team.value_count,
        0,
        SRC_VALUES,
        16,
        TEAM_HUE_COUNT,
    ) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mid = match extract_value_major_band(
        &team.colors_payload,
        team.hue_count,
        team.value_count,
        SRC_VALUES,
        SRC_VALUES,
        16,
        TEAM_HUE_COUNT,
    ) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let bot = match extract_value_major_band(
        &team.colors_payload,
        team.hue_count,
        team.value_count,
        SRC_VALUES * 2,
        SRC_VALUES,
        16,
        TEAM_HUE_COUNT,
    ) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let accent_head = match extract_value_major_band(
        &accent.colors_payload,
        accent.hue_count,
        accent.value_count,
        0,
        SRC_VALUES,
        16,
        TEAM_HUE_COUNT,
    ) {
        Ok(b) => b,
        Err(_) => return false,
    };
    top == blue.colors_payload && mid == orange.colors_payload && bot == accent_head
}

/// BlueV3 over OrangeV3 over Accent hues 0–9: 10×21, no empty cells.
fn combine_swatches(blocks: &[&SwatchSource]) -> Result<CombinedSwatches, PaletteError> {
    if blocks.len() != 3 {
        return Err(PaletteError::Msg(format!(
            "stack requires BlueTeamV3 + OrangeTeamV3 + Accent (got {} blocks)",
            blocks.len()
        )));
    }
    let blue = blocks[0];
    let orange = blocks[1];
    let accent = blocks[2];
    let debug_elem = blue.debug_elem;
    if orange.debug_elem != debug_elem || accent.debug_elem != debug_elem {
        return Err(PaletteError::Msg("DebugColors element size mismatch".into()));
    }
    if blue.hue_count != TEAM_HUE_COUNT || blue.value_count != SRC_VALUES {
        return Err(PaletteError::Msg(format!(
            "BlueTeamV3 must be {TEAM_HUE_COUNT}×{SRC_VALUES} (got {}×{})",
            blue.hue_count, blue.value_count
        )));
    }
    if orange.hue_count != TEAM_HUE_COUNT || orange.value_count != SRC_VALUES {
        return Err(PaletteError::Msg(format!(
            "OrangeTeamV3 must be {TEAM_HUE_COUNT}×{SRC_VALUES} (got {}×{})",
            orange.hue_count, orange.value_count
        )));
    }
    if accent.hue_count != ACCENT_HUE_COUNT || accent.value_count != SRC_VALUES {
        return Err(PaletteError::Msg(format!(
            "Accent must be {ACCENT_HUE_COUNT}×{SRC_VALUES} (got {}×{})",
            accent.hue_count, accent.value_count
        )));
    }
    let _ = source_hues(blue)?;
    let _ = source_hues(orange)?;
    let _ = source_hues(accent)?;

    let hue_count = STACK_HUE_COUNT;
    let value_count = STACK_VALUE_COUNT;
    let mut colors_payload = Vec::with_capacity((hue_count * value_count) as usize * 16);
    let mut debug_payload = Vec::with_capacity((hue_count * value_count) as usize * debug_elem);
    // Row-major: Colors[value * HueCount + hue] — matches garage left-to-right fill.
    for v in 0..value_count {
        let band = v / SRC_VALUES;
        let local_v = v % SRC_VALUES;
        for h in 0..hue_count {
            match band {
                0 => {
                    push_from_value_major(
                        &mut colors_payload,
                        &blue.colors_payload,
                        TEAM_HUE_COUNT,
                        SRC_VALUES,
                        h,
                        local_v,
                        16,
                    )?;
                    push_from_value_major(
                        &mut debug_payload,
                        &blue.debug_payload,
                        TEAM_HUE_COUNT,
                        SRC_VALUES,
                        h,
                        local_v,
                        debug_elem,
                    )?;
                }
                1 => {
                    push_from_value_major(
                        &mut colors_payload,
                        &orange.colors_payload,
                        TEAM_HUE_COUNT,
                        SRC_VALUES,
                        h,
                        local_v,
                        16,
                    )?;
                    push_from_value_major(
                        &mut debug_payload,
                        &orange.debug_payload,
                        TEAM_HUE_COUNT,
                        SRC_VALUES,
                        h,
                        local_v,
                        debug_elem,
                    )?;
                }
                2 => {
                    push_from_value_major(
                        &mut colors_payload,
                        &accent.colors_payload,
                        ACCENT_HUE_COUNT,
                        SRC_VALUES,
                        h,
                        local_v,
                        16,
                    )?;
                    push_from_value_major(
                        &mut debug_payload,
                        &accent.debug_payload,
                        ACCENT_HUE_COUNT,
                        SRC_VALUES,
                        h,
                        local_v,
                        debug_elem,
                    )?;
                }
                _ => {
                    return Err(PaletteError::Msg("stack value out of three bands".into()));
                }
            }
        }
    }

    let color_count_i = hue_count
        .checked_mul(value_count)
        .ok_or_else(|| PaletteError::Msg("HueCount * ValueCount overflow".into()))?;
    if colors_payload.len() != color_count_i as usize * 16 {
        return Err(PaletteError::Msg(format!(
            "stacked Colors len {} != {}*16",
            colors_payload.len(),
            color_count_i
        )));
    }
    if debug_payload.len() != color_count_i as usize * debug_elem {
        return Err(PaletteError::Msg(format!(
            "stacked DebugColors len {} != {}*{debug_elem}",
            debug_payload.len(),
            color_count_i
        )));
    }
    log::info!(
        "palette: stacked {}×{} ({} swatches = BlueV3 over OrangeV3 over Accent hues 0–9; Accent export untouched)",
        hue_count,
        value_count,
        color_count_i
    );
    Ok(CombinedSwatches {
        hue_count,
        value_count,
        color_count: color_count_i,
        colors_payload,
        debug_payload,
    })
}

fn rebuild_with_combined(
    template: &[u8],
    names: &[String],
    combined: &CombinedSwatches,
) -> Result<Vec<u8>, PaletteError> {
    let mut ser = parse_export_serial(template, names)?;
    {
        let p = find_prop_mut(&mut ser, names, "HueCount")?;
        set_int_prop(p, combined.hue_count);
    }
    {
        let p = find_prop_mut(&mut ser, names, "ValueCount")?;
        set_int_prop(p, combined.value_count);
    }
    {
        let p = find_prop_mut(&mut ser, names, "Colors")?;
        set_array_prop(p, combined.color_count, &combined.colors_payload)?;
    }
    {
        let p = find_prop_mut(&mut ser, names, "DebugColors")?;
        set_array_prop(p, combined.color_count, &combined.debug_payload)?;
    }
    if let Ok(p) = find_prop_mut(&mut ser, names, "DefaultId") {
        let cur = read_int_prop(p, names).unwrap_or(0);
        if cur < 0 || cur >= combined.color_count {
            set_int_prop(p, 0);
        }
    }
    let out = write_export_serial(&ser);
    // Round-trip sanity: rebuilt must parse and keep counts.
    let check = extract_swatches(&out, names, "rebuilt")?;
    if check.color_count != combined.color_count as usize
        || check.value_count != combined.value_count
        || check.hue_count != combined.hue_count
    {
        return Err(PaletteError::Msg(
            "rebuilt color set failed self-check".into(),
        ));
    }
    Ok(out)
}

fn load_swatch(
    file: &[u8],
    chunks: &[parser::CompressedChunk],
    sets: &HashMap<String, (usize, i32, i64)>,
    names: &[String],
    name: &str,
) -> Result<SwatchSource, PaletteError> {
    let &(_, size, off) = sets
        .get(name)
        .ok_or_else(|| PaletteError::Msg(format!("missing color set {name}")))?;
    if size <= 0 {
        return Err(PaletteError::Msg(format!("{name}: serial size invalid")));
    }
    let bytes = read_serial_bytes(file, chunks, off, size as usize)?;
    extract_swatches(&bytes, names, name)
}

fn team_sources_already_combined(blocks: &[&SwatchSource]) -> bool {
    if blocks.len() < 2 {
        return false;
    }
    // Identical payloads ⇒ the stacked remap was already written into each source export.
    blocks
        .windows(2)
        .all(|w| w[0].colors_payload == w[1].colors_payload)
}

fn apply_combined_copies(
    file: &mut Vec<u8>,
    plain: &mut [u8],
    summary: &parser::FileSummary,
    meta: &parser::CompressionMeta,
    key: &[u8; 32],
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<(usize, CombinedSwatches), PaletteError> {
    let names = parse_names_in_block(plain, summary.name_count)?;
    let sets = color_sets(plain, summary)?;
    let Some(&(_, rich_size, rich_off)) = sets.get(RICH_SET) else {
        return Err(missing_sets_error(&sets));
    };
    if rich_size <= 0 {
        return Err(PaletteError::Msg("Accent serial size invalid".into()));
    }
    if is_broken_alias(&sets) {
        return Err(PaletteError::Msg(
            "Broken shared-Accent remap still present; restore the palette backup first.".into(),
        ));
    }
    if is_accent_copy(&sets) {
        return Err(PaletteError::Msg(
            "Old Accent-only remap still present; restore the palette backup first, then Apply."
                .into(),
        ));
    }

    let (stride, chunks) = parser::parse_chunks_with_stride(plain, meta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("chunk table: {e}")))?;
    if chunks.is_empty() {
        return Err(PaletteError::Msg("no compressed chunks".into()));
    }
    validate_chunk_layout(file, &chunks)?;
    let accent_bytes = read_serial_bytes(file, &chunks, rich_off, rich_size as usize)?;
    let accent = extract_swatches(&accent_bytes, &names, RICH_SET)?;
    if accent.hue_count != ACCENT_HUE_COUNT || accent.value_count != SRC_VALUES {
        return Err(PaletteError::Msg(format!(
            "Accent must be stock {ACCENT_HUE_COUNT}×{SRC_VALUES} (got {}×{})",
            accent.hue_count, accent.value_count
        )));
    }

    let sample_name = pick_source_name(&sets, SOURCE_BLUE, SOURCE_BLUE_FALLBACK)?;
    let sample = load_swatch(file, &chunks, &sets, &names, &sample_name)?;
    let already_combined = is_combined(&sets);
    if already_combined && is_full_stack(&sample) {
        return Ok((
            0,
            CombinedSwatches {
                hue_count: sample.hue_count,
                value_count: sample.value_count,
                color_count: i32::try_from(sample.color_count).unwrap_or(0),
                colors_payload: Vec::new(),
                debug_payload: Vec::new(),
            },
        ));
    }
    if already_combined {
        return Err(PaletteError::Msg(
            "Live TAGame.upk already has a different stacked palette. Restore vanilla .vrlpal.orig, then Apply."
                .into(),
        ));
    }

    for name in STACK_SOURCES {
        if !sets.contains_key(*name) {
            return Err(missing_sets_error(&sets));
        }
    }
    let owned: Vec<SwatchSource> = STACK_SOURCES
        .iter()
        .map(|name| load_swatch(file, &chunks, &sets, &names, name))
        .collect::<Result<_, _>>()?;
    let refs: Vec<&SwatchSource> = owned.iter().collect();
    if team_sources_already_combined(&refs) {
        return Err(PaletteError::Msg(
            "Source color sets are already combined (not stock vanilla). Restore .vrlpal.orig, then Apply."
                .into(),
        ));
    }
    let combined = combine_swatches(&refs)?;
    let stacked_check = SwatchSource {
        color_count: combined.color_count as usize,
        colors_payload: combined.colors_payload.clone(),
        debug_payload: combined.debug_payload.clone(),
        debug_elem: owned[0].debug_elem,
        hue_count: combined.hue_count,
        value_count: combined.value_count,
    };
    if !is_three_band_stack(&stacked_check, &owned[0], &owned[1], &owned[2]) {
        return Err(PaletteError::Msg(
            "stacked BlueV3+OrangeV3+Accent did not produce the expected 10×21 grid".into(),
        ));
    }

    let last_idx = chunks.len() - 1;
    let last = chunks[last_idx].clone();
    let last_payload_start = last.compressed_offset as usize;
    let last_payload_end = last_payload_start + last.compressed_size as usize;
    if last_payload_end > file.len() {
        return Err(PaletteError::Msg("last chunk payload OOB".into()));
    }

    let mut last_uncomp = compression::decompress_chunk(&file[last_payload_start..last_payload_end])
        .map_err(|e| PaletteError::Msg(format!("decompress last chunk: {e}")))?;
    if last_uncomp.len() != last.uncompressed_size as usize {
        return Err(PaletteError::Msg(format!(
            "last chunk decompressed {} bytes, table says {}",
            last_uncomp.len(),
            last.uncompressed_size
        )));
    }

    let mut patched = 0usize;
    let mut append_plan: Vec<(usize, i32, i64)> = Vec::new();
    let mut expected: HashMap<String, Vec<u8>> = HashMap::new();

    for target in RICH_TARGETS {
        let Some(&(pos, size, off)) = sets.get(*target) else {
            continue;
        };
        if off == rich_off {
            return Err(PaletteError::Msg(format!(
                "{target} still aliases Accent serial; restore backup first"
            )));
        }
        let template = read_serial_bytes(file, &chunks, off, size as usize)?;
        let new_serial = rebuild_with_combined(&template, &names, &combined)?;
        let new_size = i32::try_from(new_serial.len())
            .map_err(|_| PaletteError::Msg(format!("{target}: combined serial exceeds i32")))?;
        if new_size <= size && template == new_serial {
            expected.insert((*target).to_string(), new_serial);
            continue;
        }
        // Prefer EOF-append whenever serial grows or content changes.
        if new_size > size || template != new_serial {
            let new_off = last.uncompressed_offset + last_uncomp.len() as i64;
            last_uncomp.extend_from_slice(&new_serial);
            append_plan.push((pos, new_size, new_off));
            expected.insert((*target).to_string(), new_serial);
            patched += 1;
        } else {
            expected.insert((*target).to_string(), new_serial);
        }
    }
    expected.insert(RICH_SET.to_string(), accent_bytes);

    if patched == 0 {
        if is_combined(&color_sets(plain, summary)?) {
            return Ok((0, combined));
        }
        return Err(missing_sets_error(&sets));
    }

    let new_usize = i32::try_from(last_uncomp.len())
        .map_err(|_| PaletteError::Msg(
            "combined palette too large: last chunk uncompressed size exceeds i32".into(),
        ))?;
    let new_payload = compression::compress_chunk(&last_uncomp)
        .map_err(|e| PaletteError::Msg(format!("compress last chunk: {e}")))?;
    let new_csize = i32::try_from(new_payload.len())
        .map_err(|_| PaletteError::Msg(
            "combined palette too large: last chunk compressed size exceeds i32".into(),
        ))?;

    for &(pos, new_size, new_off) in &append_plan {
        if pos + 44 > plain.len() {
            return Err(PaletteError::Msg("export entry OOB".into()));
        }
        plain[pos + 32..pos + 36].copy_from_slice(&new_size.to_le_bytes());
        plain[pos + 36..pos + 44].copy_from_slice(&new_off.to_le_bytes());
    }

    let name_offset = summary.name_offset as usize;
    let enc_size = (summary.total_header_size - meta.garbage_size - summary.name_offset) as usize;
    let enc_aligned = (enc_size + 15) & !15;
    if plain.len() != enc_aligned {
        return Err(PaletteError::Msg(format!(
            "decrypted block size mismatch ({} vs {})",
            plain.len(),
            enc_aligned
        )));
    }

    // Append grown last-chunk payload at true EOF so trailing out-of-order
    // chunks (TAGame chunk 4) keep their original CompressedOffset.
    let mut output = file.clone();
    let new_coff = i64::try_from(output.len())
        .map_err(|_| PaletteError::Msg("file offset exceeds i64".into()))?;
    output.extend_from_slice(&new_payload);

    write_chunk_entry(
        plain,
        meta.compressed_chunks_offset as usize,
        last_idx,
        stride,
        new_usize,
        new_coff,
        new_csize,
    )?;

    let new_enc = crypto::encrypt_ecb(key, plain);
    if name_offset + enc_aligned > output.len() {
        return Err(PaletteError::Msg("encrypted header OOB after chunk rewrite".into()));
    }
    output[name_offset..name_offset + enc_aligned].copy_from_slice(&new_enc);

    validate_applied(
        &output,
        keys_txt,
        keys_map_json,
        &expected,
    )?;

    let later = later_compressed_chunks(
        &chunks,
        last.compressed_offset + last.compressed_size as i64,
    );
    if !later.is_empty() {
        log::info!(
            "palette: {} chunk(s) stored after last table entry {:?}; grew payload appended at EOF",
            later.len(),
            later
        );
    }

    *file = output;
    Ok((patched, combined))
}

fn make_status(
    game_dir: &Path,
    applied: bool,
    fingerprint: String,
    message: String,
) -> PaletteStatus {
    let tagame = tagame_path(game_dir);
    let backup = backup_path(game_dir);
    PaletteStatus {
        applied,
        backup_present: backup.exists(),
        tagame_path: tagame.to_string_lossy().into_owned(),
        backup_path: backup.to_string_lossy().into_owned(),
        fingerprint,
        message,
    }
}

fn sets_from_file(
    path: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<HashMap<String, (usize, i32, i64)>, PaletteError> {
    let data = std::fs::read(path).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (summary, _meta, plain, _, _) = decrypt_tagame(&data, keys_txt, keys_map_json)?;
    color_sets(&plain, &summary)
}

pub fn is_remapped_in_file(
    game_dir: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<bool, PaletteError> {
    let cooked = resolve_cooked_dir(game_dir)?;
    let tagame = tagame_path(&cooked);
    if !tagame.exists() {
        return Ok(false);
    }
    let sets = sets_from_file(&tagame, keys_txt, keys_map_json)?;
    Ok(is_remapped(&sets))
}

fn is_broken_in_file(
    path: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<bool, PaletteError> {
    let sets = sets_from_file(path, keys_txt, keys_map_json)?;
    Ok(is_broken_alias(&sets))
}

fn is_combined_in_file(
    path: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<bool, PaletteError> {
    let sets = sets_from_file(path, keys_txt, keys_map_json)?;
    Ok(is_combined(&sets))
}

fn is_accent_copy_in_file(
    path: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<bool, PaletteError> {
    let sets = sets_from_file(path, keys_txt, keys_map_json)?;
    Ok(is_accent_copy(&sets))
}

fn is_current_stack_in_file(
    path: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<bool, PaletteError> {
    let data = std::fs::read(path).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (summary, meta, plain, _, _) = decrypt_tagame(&data, keys_txt, keys_map_json)?;
    let sets = color_sets(&plain, &summary)?;
    Ok(classify_live_sets(&data, &plain, &summary, &meta, &sets) == LiveKind::Applied)
}

/// True when live team exports are the 10×21 stack from stock BlueV3 + OrangeV3 + Accent,
/// and Accent still matches stock.
fn is_current_stack_matching_stock(
    live: &Path,
    stock: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<bool, PaletteError> {
    let stock_data = std::fs::read(stock).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (ssum, smeta, splain, _, _) = decrypt_tagame(&stock_data, keys_txt, keys_map_json)?;
    let snames = parse_names_in_block(&splain, ssum.name_count)?;
    let ssets = color_sets(&splain, &ssum)?;
    if is_broken_alias(&ssets)
        || is_accent_copy(&ssets)
        || is_combined(&ssets)
        || teams_share_serial(&ssets)
    {
        return Ok(false);
    }
    let schunks = parser::parse_chunks(&splain, smeta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("stock chunk table: {e}")))?;
    let blue = load_swatch(&stock_data, &schunks, &ssets, &snames, SOURCE_BLUE)?;
    let orange = load_swatch(&stock_data, &schunks, &ssets, &snames, SOURCE_ORANGE)?;
    let accent_stock = load_swatch(&stock_data, &schunks, &ssets, &snames, RICH_SET)?;
    if accent_stock.hue_count != ACCENT_HUE_COUNT || accent_stock.value_count != SRC_VALUES {
        return Ok(false);
    }
    if !is_current_stack_in_file(live, keys_txt, keys_map_json).unwrap_or(false) {
        return Ok(false);
    }
    let combined = combine_swatches(&[&blue, &orange, &accent_stock])?;

    let live_data = std::fs::read(live).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (lsum, lmeta, lplain, _, _) = decrypt_tagame(&live_data, keys_txt, keys_map_json)?;
    let lnames = parse_names_in_block(&lplain, lsum.name_count)?;
    let lsets = color_sets(&lplain, &lsum)?;
    let lchunks = parser::parse_chunks(&lplain, lmeta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("live chunk table: {e}")))?;
    let accent_live = load_swatch(&live_data, &lchunks, &lsets, &lnames, RICH_SET)?;
    if accent_live.colors_payload != accent_stock.colors_payload
        || accent_live.hue_count != accent_stock.hue_count
        || accent_live.value_count != accent_stock.value_count
    {
        return Ok(false);
    }
    for target in RICH_TARGETS {
        let team = load_swatch(&live_data, &lchunks, &lsets, &lnames, target)?;
        if team.hue_count != combined.hue_count
            || team.value_count != combined.value_count
            || team.color_count != combined.color_count as usize
            || team.colors_payload != combined.colors_payload
            || team.debug_payload != combined.debug_payload
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn is_vanilla_stock_file(path: &Path, keys: &str, keymap: &str) -> bool {
    if file_layout_ok(path, keys, keymap).is_err() {
        return false;
    }
    let Ok(sets) = sets_from_file(path, keys, keymap) else {
        return false;
    };
    !is_broken_alias(&sets)
        && !is_accent_copy(&sets)
        && !is_combined(&sets)
        && !teams_share_serial(&sets)
}

pub fn status(game_dir: &Path, expected_patched_fp: Option<&str>) -> PaletteStatus {
    let cooked = match resolve_cooked_dir(game_dir) {
        Ok(p) => p,
        Err(e) => {
            return make_status(game_dir, false, String::new(), e.to_string());
        }
    };
    let tagame = tagame_path(&cooked);
    let fp = file_fingerprint(&tagame).unwrap_or_default();
    let keys = include_str!("../../../python/keys.txt");
    let keymap = include_str!("../../../python/keys_map.json");
    // Integrity fingerprint is only for wipe detection elsewhere — never for applied/vanilla.
    // Backup presence is also never used as a proxy for applied.
    let _ = expected_patched_fp;

    let data = match std::fs::read(&tagame) {
        Ok(d) => d,
        Err(e) => return make_status(&cooked, false, fp, e.to_string()),
    };
    let (summary, meta, plain, _, _) = match decrypt_tagame(&data, keys, keymap) {
        Ok(v) => v,
        Err(e) => return make_status(&cooked, false, fp, e.to_string()),
    };
    let sets = match color_sets(&plain, &summary) {
        Ok(s) => s,
        Err(e) => return make_status(&cooked, false, fp, e.to_string()),
    };

    let mut kind = classify_live_sets(&data, &plain, &summary, &meta, &sets);
    let backup = backup_path(&cooked);
    if backup.exists() && is_vanilla_stock_file(&backup, keys, keymap) {
        match is_current_stack_matching_stock(&tagame, &backup, keys, keymap) {
            Ok(true) => kind = LiveKind::Applied,
            Ok(false) if kind == LiveKind::Applied => kind = LiveKind::StaleRemap,
            _ => {}
        }
    }

    match kind {
        LiveKind::BrokenAlias => make_status(
            &cooked,
            false,
            fp,
            "Broken palette remap (shared Accent serial) — click Restore before launching RL."
                .into(),
        ),
        LiveKind::AccentCopy => make_status(
            &cooked,
            false,
            fp,
            "Old Accent-only remap — Restore, then Apply for Blue+Orange V3 stack.".into(),
        ),
        LiveKind::StaleRemap => make_status(
            &cooked,
            false,
            fp,
            "Old palette remap — Restore (or Apply will restore .vrlpal.orig first)."
                .into(),
        ),
        LiveKind::Applied => make_status(
            &cooked,
            true,
            fp,
            "Rich palette on (10×21: Blue V3 over Orange V3 over Accent). Accent picker untouched.".into(),
        ),
        LiveKind::Vanilla => make_status(&cooked, false, fp, "Rich palette off".into()),
    }
}

fn restore_from(src: &Path, tagame: &Path, keys: &str, keymap: &str) -> Result<(), PaletteError> {
    if !src.exists() {
        return Err(PaletteError::Msg(format!(
            "Backup not found: {}",
            src.display()
        )));
    }
    if is_broken_in_file(src, keys, keymap).unwrap_or(false) {
        return Err(PaletteError::Msg(format!(
            "Backup {} also has the broken shared-Accent remap. Use Steam/Epic Verify.",
            src.display()
        )));
    }
    if is_accent_copy_in_file(src, keys, keymap).unwrap_or(false) {
        return Err(PaletteError::Msg(format!(
            "Backup {} has the old Accent-only remap (no stock Blue/Orange swatches). Use Steam/Epic Verify.",
            src.display()
        )));
    }
    file_layout_ok(src, keys, keymap).map_err(|e| {
        PaletteError::Msg(format!(
            "Backup {} failed layout check ({e}). Use Steam/Epic Verify.",
            src.display()
        ))
    })?;
    let bytes = std::fs::read(src).map_err(|e| PaletteError::Msg(format!("read backup: {e}")))?;
    atomic_write(tagame, &bytes)
}

fn file_layout_ok(path: &Path, keys: &str, keymap: &str) -> Result<(), PaletteError> {
    let data = std::fs::read(path).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (_summary, meta, plain, _, _) = decrypt_tagame(&data, keys, keymap)?;
    let chunks = parser::parse_chunks(&plain, meta.compressed_chunks_offset)
        .map_err(|e| PaletteError::Msg(format!("chunk table: {e}")))?;
    validate_chunk_layout(&data, &chunks)
}

pub fn apply(
    game_dir: &Path,
    keys_txt: &str,
    keys_map_json: &str,
) -> Result<PaletteStatus, PaletteError> {
    refuse_if_game_running("Apply")?;
    let cooked = resolve_cooked_dir(game_dir)?;
    let tagame = tagame_path(&cooked);
    let backup = backup_path(&cooked);
    if !tagame.exists() {
        return Err(PaletteError::Msg(format!(
            "TAGame.upk not found in {}. Use the CookedPCConsole folder.",
            cooked.display()
        )));
    }

    let live_broken = is_broken_in_file(&tagame, keys_txt, keys_map_json).unwrap_or(false);
    let live_accent_copy =
        is_accent_copy_in_file(&tagame, keys_txt, keys_map_json).unwrap_or(false);
    let live_layout_bad = file_layout_ok(&tagame, keys_txt, keys_map_json).is_err();
    let live_combined = is_combined_in_file(&tagame, keys_txt, keys_map_json).unwrap_or(false);
    let live_shared = sets_from_file(&tagame, keys_txt, keys_map_json)
        .map(|s| teams_share_serial(&s))
        .unwrap_or(false);
    let backup_usable = backup.exists() && is_vanilla_stock_file(&backup, keys_txt, keys_map_json);
    // Rebuild from stock BlueV3 + OrangeV3 + Accent whenever live is not that 10×21 stack.
    let matches_stock = backup_usable
        && is_current_stack_matching_stock(&tagame, &backup, keys_txt, keys_map_json)
            .unwrap_or(false);
    if !matches_stock {
        if backup_usable {
            restore_from(&backup, &tagame, keys_txt, keys_map_json)?;
        } else if live_broken
            || live_accent_copy
            || live_layout_bad
            || live_combined
            || live_shared
        {
            return Err(PaletteError::Msg(
                "TAGame.upk is damaged or already remapped, and no usable stock backup (.vrlpal.orig). Verify game files in Steam/Epic."
                    .into(),
            ));
        }
    }

    let mut created_backup = false;
    if !backup.exists() {
        if is_broken_in_file(&tagame, keys_txt, keys_map_json).unwrap_or(false)
            || is_accent_copy_in_file(&tagame, keys_txt, keys_map_json).unwrap_or(false)
        {
            return Err(PaletteError::Msg(
                "Refusing to back up a broken or Accent-only remapped TAGame.upk.".into(),
            ));
        }
        file_layout_ok(&tagame, keys_txt, keys_map_json)?;
        std::fs::copy(&tagame, &backup)
            .map_err(|e| PaletteError::Msg(format!("backup failed: {e}")))?;
        created_backup = true;
    }

    let mut data = std::fs::read(&tagame).map_err(|e| PaletteError::Msg(e.to_string()))?;
    let (summary, meta, mut plain, key, enc_aligned) =
        decrypt_tagame(&data, keys_txt, keys_map_json)?;

    let mut backup_note = String::new();
    if created_backup && is_combined(&color_sets(&plain, &summary)?) {
        log::warn!(
            "palette backup created from an already-combined TAGame.upk; restore will not return to stock colors"
        );
        backup_note = " Warning: backup was taken from an already-patched palette, not stock. Restore will not return to vanilla.".into();
    }

    let (patched, combined) = apply_combined_copies(
        &mut data,
        &mut plain,
        &summary,
        &meta,
        &key,
        keys_txt,
        keys_map_json,
    )?;
    let name_offset = summary.name_offset as usize;

    if patched == 0 {
        let fp = fingerprint_bytes(&data, name_offset, enc_aligned);
        return Ok(make_status(
            &cooked,
            true,
            fp,
            format!(
                "Rich palette already on (10×21: Blue V3 over Orange V3 over Accent). Restart Rocket League if colors look wrong.{backup_note}"
            ),
        ));
    }

    atomic_write(&tagame, &data)?;
    let fp = fingerprint_bytes(&data, name_offset, enc_aligned);
    Ok(make_status(
        &cooked,
        true,
        fp,
        format!(
            "Rich palette on ({patched} sets, {}×{} / {} swatches: Blue V3 over Orange V3 over Accent). Accent picker untouched. Restart Rocket League.{backup_note}",
            combined.hue_count,
            combined.value_count,
            combined.color_count
        ),
    ))
}

pub fn restore(game_dir: &Path) -> Result<PaletteStatus, PaletteError> {
    refuse_if_game_running("Restore")?;
    let cooked = resolve_cooked_dir(game_dir)?;
    let tagame = tagame_path(&cooked);
    let backup = backup_path(&cooked);
    let keys = include_str!("../../../python/keys.txt");
    let keymap = include_str!("../../../python/keys_map.json");

    if backup.exists() {
        restore_from(&backup, &tagame, keys, keymap)?;
    } else {
        return Err(PaletteError::Msg(
            "No palette backup (.vrlpal.orig)".into(),
        ));
    }

    if is_broken_in_file(&tagame, keys, keymap).unwrap_or(false) {
        return Err(PaletteError::Msg(
            "Restore finished but TAGame.upk still has a shared-Accent remap. Verify game files.".into(),
        ));
    }
    // Re-inspect live TAGame — do not assume off just because a backup file exists.
    let mut st = status(&cooked, None);
    if st.applied {
        st.message = "Restore wrote the backup, but TAGame.upk still looks patched. Verify game files.".into();
    }
    Ok(st)
}

pub fn repair_wiped_palette(game_dir: &Path, expected_patched_fp: Option<&str>) -> bool {
    let Some(exp) = expected_patched_fp.filter(|s| !s.is_empty()) else {
        return false;
    };
    let Ok(cooked) = resolve_cooked_dir(game_dir) else {
        return false;
    };
    let tagame = tagame_path(&cooked);
    let backup = backup_path(&cooked);
    if !backup.exists() || !tagame.exists() {
        return false;
    }
    let Ok(live) = file_fingerprint(&tagame) else {
        return false;
    };
    let Ok(bak) = file_fingerprint(&backup) else {
        return false;
    };
    live == bak && live != exp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_unique(team_size: i32, base_off: i64) -> HashMap<String, (usize, i32, i64)> {
        let mut s = HashMap::new();
        s.insert(RICH_SET.into(), (0, 10309, base_off));
        for (i, name) in RICH_TARGETS.iter().enumerate() {
            s.insert(
                (*name).into(),
                (0, team_size, base_off + 100_000 + (i as i64) * 20_000),
            );
        }
        s
    }

    #[test]
    fn stock_per_team_serials_are_not_remapped() {
        let mut s = HashMap::new();
        s.insert("Accent".into(), (0, 10309, 17_901_839));
        s.insert("BlueTeam".into(), (0, 1946, 17_912_148));
        s.insert("BlueTeamV2".into(), (0, 2916, 17_914_094));
        s.insert("BlueTeamV3".into(), (0, 6990, 17_917_010));
        s.insert("OrangeTeam".into(), (0, 1946, 17_936_886));
        s.insert("OrangeTeamV2".into(), (0, 2916, 17_938_832));
        s.insert("OrangeTeamV3".into(), (0, 6990, 17_941_748));
        assert!(!is_combined(&s));
        assert!(!is_accent_copy(&s));
        assert!(!is_broken_alias(&s));
    }

    #[test]
    fn shared_accent_offset_is_broken_not_applied() {
        let mut s = HashMap::new();
        s.insert("Accent".into(), (0, 10309, 17_901_839));
        for name in RICH_TARGETS {
            s.insert((*name).into(), (0, 10309, 17_901_839));
        }
        assert!(is_broken_alias(&s));
        assert!(!is_combined(&s));
        assert!(!is_accent_copy(&s));
    }

    #[test]
    fn unique_accent_copies_are_old_design_not_combined() {
        let s = sample_unique(10309, 17_901_839);
        assert!(is_accent_copy(&s));
        assert!(!is_combined(&s));
        assert!(!is_broken_alias(&s));
    }

    #[test]
    fn combined_larger_than_stock_accent_is_remapped() {
        let s = sample_unique(24_000, 17_901_839);
        assert!(is_combined(&s));
        assert!(!is_accent_copy(&s));
        assert!(is_remapped(&s));
    }

    #[test]
    fn combined_payload_embeds_accent_in_bottom_band() {
        let accent: Vec<u8> = (0..105u8).flat_map(|i| [i; 16]).collect();
        let blue: Vec<u8> = (0..70u8).flat_map(|i| [i.wrapping_add(40); 16]).collect();
        let orange: Vec<u8> = (0..70u8).flat_map(|i| [i.wrapping_add(80); 16]).collect();
        let mut stacked = Vec::new();
        stacked.extend_from_slice(&blue);
        stacked.extend_from_slice(&orange);
        assert!(!colors_payload_contains(&stacked, &accent));
    }

    fn mk(n: usize, vc: i32) -> SwatchSource {
        SwatchSource {
            color_count: n,
            colors_payload: vec![0u8; n * 16],
            debug_payload: vec![0u8; n * 81],
            debug_elem: 81,
            hue_count: (n as i32) / vc,
            value_count: vc,
        }
    }

    fn mk_ident(hues: i32, values: i32, tag: u8) -> SwatchSource {
        let n = (hues * values) as usize;
        let mut colors = Vec::with_capacity(n * 16);
        let mut debug = Vec::with_capacity(n * 81);
        for v in 0..values {
            for h in 0..hues {
                let mut c = [0u8; 16];
                c[0] = tag;
                c[1] = h as u8;
                c[2] = v as u8;
                colors.extend_from_slice(&c);
                let mut d = vec![0u8; 81];
                d[0] = tag;
                d[1] = h as u8;
                d[2] = v as u8;
                debug.extend_from_slice(&d);
            }
        }
        SwatchSource {
            color_count: n,
            colors_payload: colors,
            debug_payload: debug,
            debug_elem: 81,
            hue_count: hues,
            value_count: values,
        }
    }

    fn swatch_at(payload: &[u8], hue: i32, value: i32, hue_count: i32) -> [u8; 16] {
        let i = (value * hue_count + hue) as usize;
        payload[i * 16..i * 16 + 16].try_into().unwrap()
    }

    #[test]
    fn combine_counts_three_band() {
        let blue_v3 = mk(70, 7);
        let orange_v3 = mk(70, 7);
        let accent = mk(105, 7);
        let c = combine_swatches(&[&blue_v3, &orange_v3, &accent]).unwrap();
        assert_eq!(c.color_count, 210);
        assert_eq!(c.hue_count, 10);
        assert_eq!(c.value_count, 21);
        assert_eq!(c.colors_payload.len(), 210 * 16);
        assert_eq!(c.debug_payload.len(), 210 * 81);
        assert!(combine_swatches(&[&blue_v3, &orange_v3]).is_err());
    }

    #[test]
    fn stacked_layout_is_three_packed_bands() {
        let blue = mk_ident(10, 7, 1);
        let orange = mk_ident(10, 7, 2);
        let accent = mk_ident(15, 7, 3);
        let c = combine_swatches(&[&blue, &orange, &accent]).unwrap();
        assert_eq!(c.hue_count, 10);
        assert_eq!(c.value_count, 21);
        assert_eq!(c.color_count, 210);
        assert_eq!(&swatch_at(&c.colors_payload, 0, 0, 10)[..3], &[1, 0, 0]);
        assert_eq!(&swatch_at(&c.colors_payload, 0, 6, 10)[..3], &[1, 0, 6]);
        assert_eq!(&swatch_at(&c.colors_payload, 0, 7, 10)[..3], &[2, 0, 0]);
        assert_eq!(&swatch_at(&c.colors_payload, 0, 13, 10)[..3], &[2, 0, 6]);
        assert_eq!(&swatch_at(&c.colors_payload, 0, 14, 10)[..3], &[3, 0, 0]);
        assert_eq!(&swatch_at(&c.colors_payload, 0, 20, 10)[..3], &[3, 0, 6]);
        assert_eq!(&swatch_at(&c.colors_payload, 9, 0, 10)[..3], &[1, 9, 0]);
        assert_eq!(&swatch_at(&c.colors_payload, 9, 14, 10)[..3], &[3, 9, 0]);
        for h in 0..10 {
            let i = h as usize;
            assert_eq!(&c.colors_payload[i * 16..i * 16 + 3], &[1, h as u8, 0]);
        }
        let stacked = SwatchSource {
            color_count: c.color_count as usize,
            colors_payload: c.colors_payload.clone(),
            debug_payload: c.debug_payload.clone(),
            debug_elem: 81,
            hue_count: c.hue_count,
            value_count: c.value_count,
        };
        assert!(is_full_stack(&stacked));
        assert!(is_three_band_stack(&stacked, &blue, &orange, &accent));
    }

    #[test]
    fn fifteen_by_thirty_five_path_is_deleted() {
        let b1 = mk_ident(6, 3, 1);
        let b2 = mk_ident(7, 4, 2);
        let b3 = mk_ident(10, 7, 3);
        let o1 = mk_ident(6, 3, 4);
        let o2 = mk_ident(7, 4, 5);
        let o3 = mk_ident(10, 7, 6);
        let accent = mk_ident(15, 7, 7);
        assert!(combine_swatches(&[&b1, &b2, &b3, &o1, &o2, &o3, &accent]).is_err());
    }

    #[test]
    fn two_block_path_is_deleted() {
        let blue = mk_ident(10, 7, 1);
        let orange = mk_ident(10, 7, 2);
        let accent = mk_ident(15, 7, 3);
        assert!(combine_swatches(&[&blue, &orange]).is_err());
        let c = combine_swatches(&[&blue, &orange, &accent]).unwrap();
        assert_eq!(c.hue_count, 10);
        assert_eq!(c.value_count, 21);
    }

    #[test]
    fn shared_team_serial_is_stale_remap() {
        let mut s = HashMap::new();
        s.insert("Accent".into(), (0, 10309, 17_901_839));
        // All blue exports alias one V3 serial; oranges alias another.
        for name in ["BlueTeam", "BlueTeamV2", "BlueTeamV3"] {
            s.insert(name.into(), (0, 6990, 17_917_010));
        }
        for name in ["OrangeTeam", "OrangeTeamV2", "OrangeTeamV3"] {
            s.insert(name.into(), (0, 6990, 17_941_748));
        }
        assert!(teams_share_serial(&s));
        assert!(!is_combined(&s));
        assert!(is_remapped(&s));
    }

    #[test]
    fn tagged_prop_roundtrip_preserves_bytes() {
        // NetIndex -1, IntProperty HueCount=10, None
        let mut names = vec![String::new(); 5];
        names[0] = "None".into();
        names[1] = "HueCount".into();
        names[2] = "IntProperty".into();
        let mut serial = Vec::new();
        serial.extend_from_slice(&(-1i32).to_le_bytes());
        push_fname(&mut serial, 1, 0);
        push_fname(&mut serial, 2, 0);
        serial.extend_from_slice(&4i32.to_le_bytes());
        serial.extend_from_slice(&0i32.to_le_bytes());
        serial.extend_from_slice(&10i32.to_le_bytes());
        push_fname(&mut serial, 0, 0);
        let parsed = parse_export_serial(&serial, &names).unwrap();
        assert_eq!(write_export_serial(&parsed), serial);
    }

    #[test]
    fn later_chunks_after_last_table_entry_are_detected() {
        let chunks = vec![
            parser::CompressedChunk {
                uncompressed_offset: 0,
                uncompressed_size: 100,
                compressed_offset: 1000,
                compressed_size: 50,
            },
            parser::CompressedChunk {
                uncompressed_offset: 100,
                uncompressed_size: 100,
                compressed_offset: 2000,
                compressed_size: 50,
            },
            parser::CompressedChunk {
                uncompressed_offset: 50,
                uncompressed_size: 100,
                compressed_offset: 3000,
                compressed_size: 50,
            },
        ];
        // Last table entry ends at 2050; chunk 0 at 1000 is before; chunk 2 at 3000 is after.
        assert_eq!(later_compressed_chunks(&chunks, 2050), vec![2]);
    }

    #[test]
    fn chunk_layout_rejects_oob_and_overlap() {
        let file = vec![0u8; 100];
        let oob = [parser::CompressedChunk {
            uncompressed_offset: 0,
            uncompressed_size: 10,
            compressed_offset: 90,
            compressed_size: 20,
        }];
        assert!(validate_chunk_layout(&file, &oob).is_err());

        let mut tagged = vec![0u8; 80];
        tagged[10..14].copy_from_slice(&parser::PACKAGE_FILE_TAG.to_le_bytes());
        tagged[20..24].copy_from_slice(&parser::PACKAGE_FILE_TAG.to_le_bytes());
        let overlap = [
            parser::CompressedChunk {
                uncompressed_offset: 0,
                uncompressed_size: 10,
                compressed_offset: 10,
                compressed_size: 20,
            },
            parser::CompressedChunk {
                uncompressed_offset: 10,
                uncompressed_size: 10,
                compressed_offset: 25,
                compressed_size: 20,
            },
        ];
        assert!(validate_chunk_layout(&tagged, &overlap).is_err());
    }

    /// Decrypts stock fixture, Apply, proves HueCount=10 / ValueCount=21 and
    /// bands match BlueV3, OrangeV3, Accent hues 0–9. Accent export untouched.
    #[test]
    fn apply_stock_fixture_yields_exact_10x21_bands() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let orig = manifest.join("target/pal_test/TAGame.upk.vrlpal.orig");
        if !orig.exists() {
            eprintln!("skip apply_stock_fixture: missing {}", orig.display());
            return;
        }
        let keys = include_str!("../../../python/keys.txt");
        let keymap = include_str!("../../../python/keys_map.json");

        let mut data = std::fs::read(&orig).unwrap();
        let (sum, meta, mut plain, key, _) = decrypt_tagame(&data, keys, keymap).unwrap();
        let names = parse_names_in_block(&plain, sum.name_count).unwrap();
        let sets = color_sets(&plain, &sum).unwrap();
        let chunks = parser::parse_chunks(&plain, meta.compressed_chunks_offset).unwrap();
        let blue_stock = load_swatch(&data, &chunks, &sets, &names, SOURCE_BLUE).unwrap();
        let orange_stock = load_swatch(&data, &chunks, &sets, &names, SOURCE_ORANGE).unwrap();
        let accent_stock = load_swatch(&data, &chunks, &sets, &names, RICH_SET).unwrap();
        assert_eq!((blue_stock.hue_count, blue_stock.value_count), (10, 7));
        assert_eq!((orange_stock.hue_count, orange_stock.value_count), (10, 7));
        assert_eq!((accent_stock.hue_count, accent_stock.value_count), (15, 7));

        let (patched, combined) =
            apply_combined_copies(&mut data, &mut plain, &sum, &meta, &key, keys, keymap)
                .expect("apply_combined_copies");
        assert!(patched > 0, "expected team exports to be rewritten");
        assert_eq!(combined.hue_count, 10);
        assert_eq!(combined.value_count, 21);
        assert_eq!(combined.color_count, 210);

        let (sum2, meta2, plain2, _, _) = decrypt_tagame(&data, keys, keymap).unwrap();
        let names2 = parse_names_in_block(&plain2, sum2.name_count).unwrap();
        let sets2 = color_sets(&plain2, &sum2).unwrap();
        let chunks2 = parser::parse_chunks(&plain2, meta2.compressed_chunks_offset).unwrap();

        let accent_live = load_swatch(&data, &chunks2, &sets2, &names2, RICH_SET).unwrap();
        assert_eq!(accent_live.colors_payload, accent_stock.colors_payload);
        assert_eq!((accent_live.hue_count, accent_live.value_count), (15, 7));

        for target in RICH_TARGETS {
            let team = load_swatch(&data, &chunks2, &sets2, &names2, target).unwrap();
            assert_eq!(team.hue_count, 10, "{target} HueCount");
            assert_eq!(team.value_count, 21, "{target} ValueCount");
            assert_eq!(team.color_count, 210, "{target} Colors");
            assert!(
                is_three_band_stack(&team, &blue_stock, &orange_stock, &accent_stock),
                "{target}: BlueV3 / OrangeV3 / Accent head"
            );
        }

        assert_eq!(
            classify_live_sets(&data, &plain2, &sum2, &meta2, &sets2),
            LiveKind::Applied
        );

        let out = manifest.join("target/pal_verify_10x21");
        let _ = std::fs::create_dir_all(&out);
        std::fs::write(out.join("TAGame.upk"), &data).unwrap();
        eprintln!(
            "PROOF: HueCount=10 ValueCount=21 Colors=210; Accent untouched 15×7; wrote {}",
            out.join("TAGame.upk").display()
        );
    }
}
