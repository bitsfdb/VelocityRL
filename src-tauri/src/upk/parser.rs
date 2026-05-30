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
    /// Offset of the INT64 chunk table within the decrypted encrypted block.
    pub compressed_chunks_offset: i32,
    pub last_block_size: i32,
    /// Absolute byte position of the garbage_size field in the raw file bytes (for patching).
    pub meta_file_offset: usize,
}

/// A compressed chunk entry using the RL INT64 format (stored inside encrypted block).
#[derive(Debug, Clone)]
pub struct CompressedChunk {
    pub uncompressed_offset: i64,
    pub uncompressed_size: i32,
    pub compressed_offset: i64,
    pub compressed_size: i32,
}

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

/// Read an FString: i32 length (LE), then bytes. Positive length = ANSI (includes null). Negative = UTF-16LE.
pub fn read_fstring(c: &mut Cursor<&[u8]>) -> io::Result<String> {
    let len = read_i32(c)?;
    if len == 0 {
        return Ok(String::new());
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

/// Parse the unencrypted file prefix to extract FileSummary and CompressionMeta.
/// The CompressionMeta fields follow the standard UE3 summary in RL's custom format.
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

    // GUID: 16 bytes
    let mut _guid = [0u8; 16];
    c.read_exact(&mut _guid)?;

    // Generations TArray: i32 count, then count × (i32, i32, i32)
    let gen_count = read_i32(&mut c)?;
    for _ in 0..gen_count {
        let _ = read_i32(&mut c)?;
        let _ = read_i32(&mut c)?;
        let _ = read_i32(&mut c)?;
    }

    // engine_version (u32), cooker_version (u32)
    let _engine_version = read_u32(&mut c)?;
    let _cooker_version = read_u32(&mut c)?;

    // compression_flags (u32)
    let _compression_flags = read_u32(&mut c)?;

    // Standard UE3 compressed_chunks TArray (always 0 count in RL — empty)
    let std_chunk_count = read_i32(&mut c)?;
    // Skip any entries just in case
    for _ in 0..std_chunk_count {
        let _ = read_i64(&mut c)?; // uncompressed_offset
        let _ = read_i32(&mut c)?; // uncompressed_size
        let _ = read_i64(&mut c)?; // compressed_offset
        let _ = read_i32(&mut c)?; // compressed_size
    }

    // PackageSource (u32)
    let _ = read_u32(&mut c)?;

    // AdditionalPackagesToCook TArray<FString>
    let additional_count = read_i32(&mut c)?;
    for _ in 0..additional_count {
        let _ = read_fstring(&mut c)?;
    }

    // TextureAllocations: TArray of structs, always 0 count in cooked RL packages
    let tex_alloc_count = read_i32(&mut c)?;
    for _ in 0..tex_alloc_count {
        // Each entry: 5 × i32, then TArray<i32>
        for _ in 0..5 { let _ = read_i32(&mut c)?; }
        let inner = read_i32(&mut c)?;
        for _ in 0..inner { let _ = read_i32(&mut c)?; }
    }

    // RL-specific metadata immediately follows
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

/// Parse the INT64 chunk table from the decrypted encrypted block.
/// `chunks_offset` is relative to the start of the decrypted block.
pub fn parse_chunks(decrypted_block: &[u8], chunks_offset: i32) -> io::Result<Vec<CompressedChunk>> {
    let mut c = Cursor::new(decrypted_block);
    c.seek(SeekFrom::Start(chunks_offset as u64))?;
    let count = read_i32(&mut c)?;
    let mut chunks = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count.max(0) {
        chunks.push(CompressedChunk {
            uncompressed_offset: read_i64(&mut c)?,
            uncompressed_size: read_i32(&mut c)?,
            compressed_offset: read_i64(&mut c)?,
            compressed_size: read_i32(&mut c)?,
        });
    }
    Ok(chunks)
}

/// Parse the name table from decrypted data.
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

/// Locate the byte offsets of key summary fields within the raw prefix bytes.
/// Used during re-encryption to patch the prefix in place.
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

    c.read_exact(&mut b4)?; // tag
    if u32::from_le_bytes(b4) != PACKAGE_FILE_TAG {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad tag"));
    }
    c.read_exact(&mut b2)?; // file_version
    c.read_exact(&mut b2)?; // licensee_version

    let total_header_size_offset = c.position() as usize;
    c.read_exact(&mut b4)?; // total_header_size

    // Skip folder_name fstring
    c.read_exact(&mut b4)?;
    let fstr_len = i32::from_le_bytes(b4);
    if fstr_len > 0 {
        c.seek(SeekFrom::Current(fstr_len as i64))?;
    } else if fstr_len < 0 {
        c.seek(SeekFrom::Current((-fstr_len * 2) as i64))?;
    }

    c.read_exact(&mut b4)?; // package_flags
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
