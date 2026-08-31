use crate::upk::{
    compression::compress_chunk,
    crypto::encrypt_ecb,
    parser::{CompressedChunk, CompressionMeta, FileSummary, find_summary_offsets, parse_prefix},
};

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

    let name_offset = summary.name_offset as usize;
    let enc_block_size = (summary.total_header_size - meta.garbage_size - summary.name_offset) as usize;
    let enc_block_size_aligned = (enc_block_size + 15) & !15;
    if name_offset + enc_block_size_aligned > original_file.len() {
        return Err("original encrypted block OOB".into());
    }
    let enc_data = &original_file[name_offset..name_offset + enc_block_size_aligned];
    let original_plain = crate::upk::crypto::decrypt_ecb(donor_key, enc_data);

    let (mod_sum, _) = parse_prefix(modified_decrypted)
        .map_err(|e| format!("re-parse modified prefix: {}", e))?;

    let modified_depends = mod_sum.depends_offset as i64;
    let orig_first_uoff = original_chunks[0].uncompressed_offset;
    let chunk_delta = modified_depends - orig_first_uoff;

    let chunks_table_offset = meta.compressed_chunks_offset as usize;

    let chunk_table_len = 4 + original_chunks.len() * crate::upk::parser::CHUNK_ENTRY_FIELDS;
    let required_plain_len = chunks_table_offset + chunk_table_len;
    let encrypted_plain_len = (required_plain_len + 15) & !15;
    let new_total_header_size = summary.name_offset + encrypted_plain_len as i32 + meta.garbage_size;

    let mut rebuilt_chunks: Vec<CompressedChunk> = Vec::new();
    let mut rebuilt_payloads: Vec<Vec<u8>> = Vec::new();
    let mut current_compressed_offset = new_total_header_size as i64;

    for (i, orig_chunk) in original_chunks.iter().enumerate() {
        let start = (orig_chunk.uncompressed_offset + chunk_delta) as usize;
        let end = if i + 1 < original_chunks.len() {
            (original_chunks[i + 1].uncompressed_offset + chunk_delta) as usize
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

    let mut header_plain = vec![0u8; encrypted_plain_len];

    let copy_len = original_plain.len().min(encrypted_plain_len);
    header_plain[..copy_len].copy_from_slice(&original_plain[..copy_len]);

    let tables_len = (mod_sum.depends_offset as usize).saturating_sub(name_offset);
    let tables_copy = tables_len.min(chunks_table_offset);
    if name_offset + tables_copy <= modified_decrypted.len() {
        header_plain[..tables_copy]
            .copy_from_slice(&modified_decrypted[name_offset..name_offset + tables_copy]);
    }

    let chunk_table_bytes = serialize_chunk_table(&rebuilt_chunks);
    let ct_end = chunks_table_offset + chunk_table_bytes.len();
    if ct_end <= header_plain.len() {
        header_plain[chunks_table_offset..ct_end].copy_from_slice(&chunk_table_bytes);
    }

    let encrypted_header = encrypt_ecb(output_key, &header_plain);

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

    if meta.meta_file_offset + 8 <= prefix.len() {
        patch_i32(&mut prefix, meta.meta_file_offset + 4, meta.compressed_chunks_offset);
        if let Some(last) = rebuilt_chunks.last() {
            patch_i32(&mut prefix, meta.meta_file_offset + 8, last.uncompressed_size);
        }
    }

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
