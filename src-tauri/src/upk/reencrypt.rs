use crate::upk::{
    compression::compress_chunk,
    crypto::encrypt_ecb,
    parser::{CompressedChunk, CompressionMeta, FileSummary, find_summary_offsets, parse_prefix},
};

/// Serialize the RL INT64 chunk table: count (i32) + N × (i64, i32, i64, i32).
fn serialize_chunk_table(chunks: &[CompressedChunk]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(chunks.len() as i32).to_le_bytes());
    for c in chunks {
        out.extend_from_slice(&c.uncompressed_offset.to_le_bytes());
        out.extend_from_slice(&c.uncompressed_size.to_le_bytes());
        out.extend_from_slice(&c.compressed_offset.to_le_bytes());
        out.extend_from_slice(&c.compressed_size.to_le_bytes());
    }
    out
}

fn patch_i32(data: &mut [u8], offset: usize, value: i32) {
    if offset + 4 <= data.len() {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// Re-encrypt a modified decrypted RL UPK package.
///
/// Parameters:
/// - `original_file`: raw bytes of the original (donor) encrypted file
/// - `modified_decrypted`: full decrypted+decompressed bytes after name-table rename
/// - `summary`: FileSummary parsed from the original file
/// - `meta`: CompressionMeta parsed from the original file
/// - `original_chunks`: INT64 chunk list parsed from the original encrypted block
/// - `donor_key`: AES key used to decrypt the donor file
/// - `output_key`: AES key to use for encrypting the output (target's key, or donor's)
///
/// Returns the final encrypted output bytes ready to write to disk.
pub fn reencrypt(
    original_file: &[u8],
    modified_decrypted: &[u8],
    summary: &FileSummary,
    meta: &CompressionMeta,
    original_chunks: &[CompressedChunk],
    donor_key: &[u8; 32],
    output_key: &[u8; 32],
) -> Result<Vec<u8>, String> {
    if original_chunks.is_empty() {
        return Err("no compressed chunks in donor file".into());
    }

    // ── 1. Decrypt original encrypted block (for header structure) ──────────────
    let name_offset = summary.name_offset as usize;
    let enc_block_size = (summary.total_header_size - meta.garbage_size - summary.name_offset) as usize;
    let enc_block_size_aligned = (enc_block_size + 15) & !15;
    if name_offset + enc_block_size_aligned > original_file.len() {
        return Err("original encrypted block OOB".into());
    }
    let enc_data = &original_file[name_offset..name_offset + enc_block_size_aligned];
    let original_plain = crate::upk::crypto::decrypt_ecb(donor_key, enc_data);

    // ── 2. Re-parse modified buffer FIRST to get updated offsets ────────────────
    // apply_name_pairs may have rebuilt the name table (shrinking/growing it),
    // which shifts depends_offset. We must use the NEW value so chunk_shift and
    // tables_copy are both correct.
    let (mod_sum, _) = parse_prefix(modified_decrypted)
        .map_err(|e| format!("re-parse modified prefix: {}", e))?;

    let modified_depends = mod_sum.depends_offset as i64;
    let orig_first_uoff = original_chunks[0].uncompressed_offset;
    let chunk_shift = modified_depends - orig_first_uoff;

    // ── 3. Determine encrypted header size ──────────────────────────────────────
    let chunks_table_offset = meta.compressed_chunks_offset as usize;
    // Each INT64 chunk entry = i64 + i32 + i64 + i32 = 24 bytes; + 4 byte count
    let chunk_table_len = 4 + original_chunks.len() * 24;
    let required_plain_len = chunks_table_offset + chunk_table_len;
    let encrypted_plain_len = (required_plain_len + 15) & !15;
    let new_total_header_size = summary.name_offset + encrypted_plain_len as i32 + meta.garbage_size;

    // ── 4. Compress body chunks ──────────────────────────────────────────────────
    let mut rebuilt_chunks: Vec<CompressedChunk> = Vec::new();
    let mut rebuilt_payloads: Vec<Vec<u8>> = Vec::new();
    let mut current_compressed_offset = new_total_header_size as i64;

    for (i, orig_chunk) in original_chunks.iter().enumerate() {
        let start = (orig_chunk.uncompressed_offset + chunk_shift) as usize;
        let end = if i + 1 < original_chunks.len() {
            (original_chunks[i + 1].uncompressed_offset + chunk_shift) as usize
        } else {
            modified_decrypted.len()
        };
        if start > modified_decrypted.len() || end > modified_decrypted.len() || end < start {
            return Err(format!("chunk {}: body slice [{},{}] out of range (total={})",
                i, start, end, modified_decrypted.len()));
        }
        let payload = compress_chunk(&modified_decrypted[start..end])
            .map_err(|e| format!("compress chunk {}: {}", i, e))?;
        let comp_size = payload.len() as i32;
        rebuilt_chunks.push(CompressedChunk {
            uncompressed_offset: start as i64,
            uncompressed_size: (end - start) as i32,
            compressed_offset: current_compressed_offset,
            compressed_size: comp_size,
        });
        current_compressed_offset += comp_size as i64;
        rebuilt_payloads.push(payload);
    }

    // ── 5. Build header_plain ────────────────────────────────────────────────────
    let mut header_plain = vec![0u8; encrypted_plain_len];
    // Copy original decrypted header as base
    let copy_len = original_plain.len().min(encrypted_plain_len);
    header_plain[..copy_len].copy_from_slice(&original_plain[..copy_len]);

    // Overwrite with modified name/import/export tables
    // Use the NEW depends_offset (after name table rebuild) so we don't copy
    // body bytes into the header or miss the last few table bytes.
    let tables_len = (mod_sum.depends_offset as usize).saturating_sub(name_offset);
    let tables_copy = tables_len.min(chunks_table_offset);
    if name_offset + tables_copy <= modified_decrypted.len() {
        header_plain[..tables_copy]
            .copy_from_slice(&modified_decrypted[name_offset..name_offset + tables_copy]);
    }

    // Write chunk table at chunks_table_offset
    let chunk_table_bytes = serialize_chunk_table(&rebuilt_chunks);
    let ct_end = chunks_table_offset + chunk_table_bytes.len();
    if ct_end <= header_plain.len() {
        header_plain[chunks_table_offset..ct_end].copy_from_slice(&chunk_table_bytes);
    }

    // ── 6. AES-encrypt the header ────────────────────────────────────────────────
    let encrypted_header = encrypt_ecb(output_key, &header_plain);

    // ── 7. Patch the unencrypted prefix ──────────────────────────────────────────
    let mut prefix = original_file[..name_offset].to_vec();
    let offsets = find_summary_offsets(&prefix)
        .map_err(|e| format!("find_summary_offsets: {}", e))?;

    patch_i32(&mut prefix, offsets.total_header_size_offset, new_total_header_size);
    patch_i32(&mut prefix, offsets.name_count_offset, mod_sum.name_count);
    patch_i32(&mut prefix, offsets.name_offset_offset, mod_sum.name_offset);
    patch_i32(&mut prefix, offsets.export_count_offset, mod_sum.export_count);
    patch_i32(&mut prefix, offsets.export_offset_offset, mod_sum.export_offset);
    patch_i32(&mut prefix, offsets.import_count_offset, mod_sum.import_count);
    patch_i32(&mut prefix, offsets.import_offset_offset, mod_sum.import_offset);
    patch_i32(&mut prefix, offsets.depends_offset_offset, mod_sum.depends_offset);

    // Patch meta: compressed_chunks_offset and last_block_size
    if meta.meta_file_offset + 8 <= prefix.len() {
        patch_i32(&mut prefix, meta.meta_file_offset + 4, meta.compressed_chunks_offset);
        if let Some(last) = rebuilt_chunks.last() {
            patch_i32(&mut prefix, meta.meta_file_offset + 8, last.uncompressed_size);
        }
    }

    // ── 8. Assemble output ───────────────────────────────────────────────────────
    // Layout: prefix | encrypted_header | gap | compressed_payloads
    // The gap must be exactly garbage_size bytes. The encrypted block size is
    // aligned up to 16 bytes which may overshoot the real end of encrypted data —
    // use the Python fallback: if computed gap != garbage_size, take the last
    // garbage_size bytes before the first chunk's compressed_offset instead.
    let orig_gap_start = name_offset + enc_data.len();
    let orig_gap_end = original_chunks[0].compressed_offset as usize;
    let garbage_size = meta.garbage_size as usize;
    let gap_bytes: &[u8] = if orig_gap_end <= original_file.len() {
        let candidate = if orig_gap_end > orig_gap_start {
            &original_file[orig_gap_start..orig_gap_end]
        } else {
            &[]
        };
        if candidate.len() != garbage_size && orig_gap_end >= garbage_size {
            // Fallback: use last garbage_size bytes before first chunk
            &original_file[orig_gap_end - garbage_size..orig_gap_end]
        } else {
            candidate
        }
    } else {
        &[]
    };

    let mut output = Vec::new();
    output.extend_from_slice(&prefix);
    output.extend_from_slice(&encrypted_header);
    output.extend_from_slice(gap_bytes);
    for payload in &rebuilt_payloads {
        output.extend_from_slice(payload);
    }
    Ok(output)
}
