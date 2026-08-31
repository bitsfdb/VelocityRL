use std::io::{self, Cursor, Read, Seek, SeekFrom};

pub const PACKAGE_FILE_TAG: u32 = 0x9E2A83C1;

#[derive(Debug, Clone)]
pub struct FileSummary {
    pub tag: u32,
    pub file_version: u16,
    pub licensee_version: u16,
    pub total_header_size: i32,
    pub folder_name: String,
    pub package_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    pub depends_offset: i32,
}

#[derive(Debug, Clone)]
pub struct CompressionMeta {
    pub garbage_size: i32,

    pub compressed_chunks_offset: i32,
    pub last_block_size: i32,

    pub meta_file_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedChunk {
    pub uncompressed_offset: i64,
    pub uncompressed_size: i32,
    pub compressed_offset: i64,
    pub compressed_size: i32,
}

pub const CHUNK_ENTRY_FIELDS: usize = 24;

const CHUNK_ENTRY_STRIDES: [usize; 3] = [24, 32, 36];

#[derive(Debug, Clone)]
pub struct NameEntry {
    pub name: String,
    pub flags: u64,
}

fn read_i32(c: &mut Cursor<&[u8]>) -> io::Result<i32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn read_i64(c: &mut Cursor<&[u8]>) -> io::Result<i64> {
    let mut b = [0u8; 8];
    c.read_exact(&mut b)?;
    Ok(i64::from_le_bytes(b))
}

fn read_u16(c: &mut Cursor<&[u8]>) -> io::Result<u16> {
    let mut b = [0u8; 2];
    c.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32(c: &mut Cursor<&[u8]>) -> io::Result<u32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(c: &mut Cursor<&[u8]>) -> io::Result<u64> {
    let mut b = [0u8; 8];
    c.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

pub fn read_fstring(c: &mut Cursor<&[u8]>) -> io::Result<String> {
    const MAX_FSTRING_CHARS: i32 = 1_048_576;
    let len = read_i32(c)?;
    if len == 0 {
        return Ok(String::new());
    }
    if len > MAX_FSTRING_CHARS || len < -MAX_FSTRING_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("FString length {len} exceeds {MAX_FSTRING_CHARS}"),
        ));
    }
    if len > 0 {
        let mut buf = vec![0u8; len as usize];
        c.read_exact(&mut buf)?;
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
    } else {
        let char_count = (-len) as usize;
        let mut buf = vec![0u8; char_count * 2];
        c.read_exact(&mut buf)?;
        let words: Vec<u16> = buf.chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let end = words.iter().position(|&w| w == 0).unwrap_or(words.len());
        Ok(String::from_utf16_lossy(&words[..end]).to_owned())
    }
}

pub fn parse_prefix(data: &[u8]) -> io::Result<(FileSummary, CompressionMeta)> {
    let mut c = Cursor::new(data);

    let tag = read_u32(&mut c)?;
    if tag != PACKAGE_FILE_TAG {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a valid UPK (bad tag)"));
    }
    let file_version = read_u16(&mut c)?;
    let licensee_version = read_u16(&mut c)?;
    let total_header_size = read_i32(&mut c)?;
    let folder_name = read_fstring(&mut c)?;
    let package_flags = read_u32(&mut c)?;
    let name_count = read_i32(&mut c)?;
    let name_offset = read_i32(&mut c)?;
    let export_count = read_i32(&mut c)?;
    let export_offset = read_i32(&mut c)?;
    let import_count = read_i32(&mut c)?;
    let import_offset = read_i32(&mut c)?;
    let depends_offset = read_i32(&mut c)?;
    let _import_export_guids_offset = read_i32(&mut c)?;
    let _import_guids_count = read_i32(&mut c)?;
    let _export_guids_count = read_i32(&mut c)?;
    let _thumbnail_table_offset = read_i32(&mut c)?;

    let mut _guid = [0u8; 16];
    c.read_exact(&mut _guid)?;

    let gen_count = read_i32(&mut c)?;
    if !(0..=16_384).contains(&gen_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("implausible generations count {gen_count}"),
        ));
    }
    for _ in 0..gen_count {
        let _ = read_i32(&mut c)?;
        let _ = read_i32(&mut c)?;
        let _ = read_i32(&mut c)?;
    }

    let _engine_version = read_u32(&mut c)?;
    let _cooker_version = read_u32(&mut c)?;

    let _compression_flags = read_u32(&mut c)?;

    let std_chunk_count = read_i32(&mut c)?;
    if !(0..=16_384).contains(&std_chunk_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("implausible standard chunk count {std_chunk_count}"),
        ));
    }

    for _ in 0..std_chunk_count {
        let _ = read_i64(&mut c)?;
        let _ = read_i32(&mut c)?;
        let _ = read_i64(&mut c)?;
        let _ = read_i32(&mut c)?;
    }

    let _ = read_u32(&mut c)?;

    let additional_count = read_i32(&mut c)?;
    if !(0..=16_384).contains(&additional_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("implausible AdditionalPackagesToCook count {additional_count}"),
        ));
    }
    for _ in 0..additional_count {
        let _ = read_fstring(&mut c)?;
    }

    let tex_alloc_count = read_i32(&mut c)?;
    if !(0..=16_384).contains(&tex_alloc_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("implausible TextureAllocations count {tex_alloc_count}"),
        ));
    }
    for _ in 0..tex_alloc_count {

        for _ in 0..5 { let _ = read_i32(&mut c)?; }
        let inner = read_i32(&mut c)?;
        for _ in 0..inner { let _ = read_i32(&mut c)?; }
    }

    let meta_file_offset = c.position() as usize;
    let garbage_size = read_i32(&mut c)?;
    let compressed_chunks_offset = read_i32(&mut c)?;
    let last_block_size = read_i32(&mut c)?;

    let summary = FileSummary {
        tag,
        file_version,
        licensee_version,
        total_header_size,
        folder_name,
        package_flags,
        name_count,
        name_offset,
        export_count,
        export_offset,
        import_count,
        import_offset,
        depends_offset,
    };
    let meta = CompressionMeta {
        garbage_size,
        compressed_chunks_offset,
        last_block_size,
        meta_file_offset,
    };
    Ok((summary, meta))
}

pub fn parse_chunks(decrypted_block: &[u8], chunks_offset: i32) -> io::Result<Vec<CompressedChunk>> {
    parse_chunks_with_stride(decrypted_block, chunks_offset).map(|(_, chunks)| chunks)
}

/// Like [`parse_chunks`], but also returns the on-disk entry stride (24 or 36).
pub fn parse_chunks_with_stride(
    decrypted_block: &[u8],
    chunks_offset: i32,
) -> io::Result<(usize, Vec<CompressedChunk>)> {
    if chunks_offset < 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "negative chunk table offset"));
    }
    let table_off = chunks_offset as usize;
    if table_off + 4 > decrypted_block.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "chunk table offset past end of decrypted block"));
    }
    let count_raw: [u8; 4] = decrypted_block[table_off..table_off + 4]
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "truncated chunk count"))?;
    let count = i32::from_le_bytes(count_raw);
    if count < 1 || count > 65_536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("implausible chunk count {count}"),
        ));
    }
    let count = count as usize;

    let mut last_err = String::new();
    for stride in CHUNK_ENTRY_STRIDES {
        match decode_chunk_table(decrypted_block, table_off, count, stride) {
            Ok(chunks) if chunk_table_plausible(&chunks) => return Ok((stride, chunks)),
            Ok(_) => {
                last_err = format!("stride {stride}: decoded {count} entries but values are not plausible");
            }
            Err(e) => last_err = format!("stride {stride}: {e}"),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("no valid chunk-table stride among {:?} ({last_err})", CHUNK_ENTRY_STRIDES),
    ))
}

fn decode_chunk_table(
    plain: &[u8],
    table_off: usize,
    count: usize,
    stride: usize,
) -> io::Result<Vec<CompressedChunk>> {
    let need = table_off
        .checked_add(4)
        .and_then(|o| o.checked_add(count.checked_mul(stride).unwrap_or(usize::MAX)))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "chunk table size overflow"))?;
    if need > plain.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("stride {stride} table needs {need} bytes, block is {}", plain.len()),
        ));
    }
    let mut chunks = Vec::with_capacity(count);
    for i in 0..count {
        let p = table_off + 4 + i * stride;
        chunks.push(read_chunk_fields(&plain[p..p + CHUNK_ENTRY_FIELDS]).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "truncated chunk entry")
        })?);
    }
    Ok(chunks)
}

fn read_chunk_fields(entry: &[u8]) -> Option<CompressedChunk> {
    if entry.len() < CHUNK_ENTRY_FIELDS {
        return None;
    }
    Some(CompressedChunk {
        uncompressed_offset: i64::from_le_bytes(entry.get(0..8)?.try_into().ok()?),
        uncompressed_size: i32::from_le_bytes(entry.get(8..12)?.try_into().ok()?),
        compressed_offset: i64::from_le_bytes(entry.get(12..20)?.try_into().ok()?),
        compressed_size: i32::from_le_bytes(entry.get(20..24)?.try_into().ok()?),
    })
}

fn chunk_table_plausible(chunks: &[CompressedChunk]) -> bool {
    if chunks.is_empty() {
        return false;
    }
    for c in chunks {
        if c.uncompressed_offset < 0 || c.compressed_offset < 0 {
            return false;
        }
        if c.uncompressed_size <= 0 || c.compressed_size <= 0 {
            return false;
        }

        if (c.compressed_size as i64) > (c.uncompressed_size as i64) + 4096 {
            return false;
        }
    }
    chunks.windows(2).all(|w| {
        w[1].uncompressed_offset > w[0].uncompressed_offset
            && w[1].uncompressed_offset - w[0].uncompressed_offset == w[0].uncompressed_size as i64
    })
}

pub fn parse_name_table(data: &[u8], name_offset: i32, name_count: i32) -> io::Result<Vec<NameEntry>> {
    let mut c = Cursor::new(data);
    c.seek(SeekFrom::Start(name_offset as u64))?;
    let mut names = Vec::with_capacity(name_count.max(0) as usize);
    for _ in 0..name_count.max(0) {
        let name = read_fstring(&mut c)?;
        let flags = read_u64(&mut c)?;
        names.push(NameEntry { name, flags });
    }
    Ok(names)
}

pub struct SummaryOffsets {
    pub total_header_size_offset: usize,
    pub name_count_offset: usize,
    pub name_offset_offset: usize,
    pub export_count_offset: usize,
    pub export_offset_offset: usize,
    pub import_count_offset: usize,
    pub import_offset_offset: usize,
    pub depends_offset_offset: usize,
}

pub fn find_summary_offsets(data: &[u8]) -> io::Result<SummaryOffsets> {
    let mut c = Cursor::new(data);
    let mut b4 = [0u8; 4];
    let mut b2 = [0u8; 2];

    c.read_exact(&mut b4)?;
    if u32::from_le_bytes(b4) != PACKAGE_FILE_TAG {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad tag"));
    }
    c.read_exact(&mut b2)?;
    c.read_exact(&mut b2)?;

    let total_header_size_offset = c.position() as usize;
    c.read_exact(&mut b4)?;

    c.read_exact(&mut b4)?;
    let fstr_len = i32::from_le_bytes(b4);
    if fstr_len > 0 {
        c.seek(SeekFrom::Current(fstr_len as i64))?;
    } else if fstr_len < 0 {
        c.seek(SeekFrom::Current((-fstr_len * 2) as i64))?;
    }

    c.read_exact(&mut b4)?;
    let name_count_offset = c.position() as usize;
    c.read_exact(&mut b4)?;
    let name_offset_offset = c.position() as usize;
    c.read_exact(&mut b4)?;
    let export_count_offset = c.position() as usize;
    c.read_exact(&mut b4)?;
    let export_offset_offset = c.position() as usize;
    c.read_exact(&mut b4)?;
    let import_count_offset = c.position() as usize;
    c.read_exact(&mut b4)?;
    let import_offset_offset = c.position() as usize;
    c.read_exact(&mut b4)?;
    let depends_offset_offset = c.position() as usize;

    Ok(SummaryOffsets {
        total_header_size_offset,
        name_count_offset,
        name_offset_offset,
        export_count_offset,
        export_offset_offset,
        import_count_offset,
        import_offset_offset,
        depends_offset_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    fn tagame_36_prefix() -> Vec<u8> {
        let mut b = hex(
            "
            03 00 00 00
            7C BD 9C 00 00 00 00 00 C4 FC 1F 00 65 93 A4 00 00 00 00 00 0C 05 08 00
            00 00 00 00 00 00 00 00 00 00 00 00
            40 BA BC 00 00 00 00 00 EB FF 1F 00 71 98 AC 00 00 00 00 00 C5 75 08 00
            00 00 00 00 00 00 00 00 00 00 00 00
            2B BA DC 00 00 00 00 00 9E FF 1F 00 36 0E B5 00 00 00 00 00 E6 8C 08 00
            00 00 00 00 00 00 00 00 00 00 00 00
            ",
        );
        assert_eq!(b.len(), 4 + 3 * 36);

        b.extend_from_slice(&[0xAA, 0xBB]);
        b
    }

    fn aftershock_24_table() -> Vec<u8> {
        hex(
            "
            04 00 00 00
            DB 07 01 00 00 00 00 00 1E 66 00 00 85 47 01 00 00 00 00 00 97 0B 00 00
            F9 6D 01 00 00 00 00 00 3A 7F 38 00 1C 53 01 00 00 00 00 00 3D 93 11 00
            33 ED 39 00 00 00 00 00 9C 3E 01 00 59 E6 12 00 00 00 00 00 99 DB 00 00
            CF 2B 3B 00 00 00 00 00 C5 00 00 00 F2 C1 13 00 00 00 00 00 7A 00 00 00
            ",
        )
    }

    #[test]
    fn parse_chunks_tagame_uses_36_byte_stride() {
        let buf = tagame_36_prefix();
        let (stride, chunks) =
            parse_chunks_with_stride(&buf, 0).expect("TAGame 36-byte table should parse");
        assert_eq!(stride, 36);
        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks[0],
            CompressedChunk {
                uncompressed_offset: 10_272_124,
                uncompressed_size: 2_096_324,
                compressed_offset: 10_785_637,
                compressed_size: 525_580,
            }
        );
        assert_eq!(chunks[1].uncompressed_offset, 12_368_448);
        assert_eq!(chunks[1].uncompressed_size, 2_097_131);
        assert_eq!(chunks[1].compressed_offset, 11_311_217);
        assert_eq!(chunks[1].compressed_size, 554_437);
        assert_eq!(chunks[2].uncompressed_offset, 14_465_579);
        assert_eq!(chunks[2].uncompressed_size, 2_097_054);
        assert_eq!(chunks[2].compressed_offset, 11_865_654);
        assert_eq!(chunks[2].compressed_size, 560_358);

        assert_eq!(
            chunks[1].uncompressed_offset - chunks[0].uncompressed_offset,
            chunks[0].uncompressed_size as i64
        );
        assert_eq!(
            chunks[2].uncompressed_offset - chunks[1].uncompressed_offset,
            chunks[1].uncompressed_size as i64
        );
    }

    #[test]
    fn parse_chunks_sf_uses_24_byte_stride() {
        let buf = aftershock_24_table();
        assert_eq!(buf.len(), 4 + 4 * 24);
        let (stride, chunks) =
            parse_chunks_with_stride(&buf, 0).expect("_SF 24-byte table should parse");
        assert_eq!(stride, 24);
        assert_eq!(chunks.len(), 4);
        assert_eq!(
            chunks[0],
            CompressedChunk {
                uncompressed_offset: 67_547,
                uncompressed_size: 26_142,
                compressed_offset: 83_845,
                compressed_size: 2_967,
            }
        );
        assert_eq!(chunks[1].uncompressed_offset, 93_689);
        assert_eq!(chunks[1].uncompressed_size, 3_702_586);
        assert_eq!(chunks[2].uncompressed_offset, 3_796_275);
        assert_eq!(chunks[3].uncompressed_offset, 3_877_839);
        assert_eq!(chunks[3].uncompressed_size, 197);
        assert_eq!(chunks[3].compressed_size, 122);
        for w in chunks.windows(2) {
            assert_eq!(
                w[1].uncompressed_offset - w[0].uncompressed_offset,
                w[0].uncompressed_size as i64
            );
        }
    }

    #[test]
    fn parse_chunks_24_byte_misparse_of_tagame_is_rejected() {
        let buf = tagame_36_prefix();
        let wrong = decode_chunk_table(&buf, 0, 3, 24).unwrap();
        assert!(!chunk_table_plausible(&wrong), "24-byte stride on TAGame bytes must not look plausible");
        assert_eq!(wrong[1].uncompressed_offset, 0);
        assert_eq!(wrong[1].uncompressed_size, 0);
    }

    #[test]
    fn parse_chunks_rejects_bad_count_and_truncated_table() {
        assert!(parse_chunks(&[0, 0, 0, 0], 0).is_err());
        assert!(parse_chunks(&[0xff, 0xff, 0xff, 0x7f], 0).is_err());
        let tiny = hex("03 00 00 00 01 00 00 00");
        let err = parse_chunks(&tiny, 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
