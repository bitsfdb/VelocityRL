use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::io::{self, Cursor, Read, Write};

const PACKAGE_FILE_TAG: u32 = 0x9E2A83C1;
const DEFAULT_BLOCK_SIZE: usize = 0x20000; // 128 KiB

fn read_i32(c: &mut Cursor<&[u8]>) -> io::Result<i32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

/// Decompress a single RL chunk payload into uncompressed bytes.
pub fn decompress_chunk(payload: &[u8]) -> io::Result<Vec<u8>> {
    let mut c = Cursor::new(payload);

    let tag = {
        let mut b = [0u8; 4];
        c.read_exact(&mut b)?;
        u32::from_le_bytes(b)
    };
    if tag != PACKAGE_FILE_TAG {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("bad chunk magic: 0x{:08X}", tag)));
    }
    let _block_size = read_i32(&mut c)?;
    let _total_comp = read_i32(&mut c)?;
    let total_uncomp = read_i32(&mut c)?;

    // Read block headers until sum of uncompressed sizes == total_uncomp
    let mut blocks: Vec<(i32, i32)> = Vec::new();
    let mut sum_uncomp = 0i32;
    while sum_uncomp < total_uncomp {
        let comp = read_i32(&mut c)?;
        let uncomp = read_i32(&mut c)?;
        blocks.push((comp, uncomp));
        sum_uncomp = sum_uncomp.saturating_add(uncomp);
    }

    // Decompress each block
    let mut out = Vec::with_capacity(total_uncomp as usize);
    for (comp_size, _uncomp_size) in &blocks {
        let start = c.position() as usize;
        let end = start + *comp_size as usize;
        if end > payload.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "block data overrun"));
        }
        let block_data = &payload[start..end];
        let mut decoder = ZlibDecoder::new(block_data);
        let mut block_out = Vec::new();
        decoder.read_to_end(&mut block_out)?;
        out.extend_from_slice(&block_out);
        c.set_position(end as u64);
    }
    Ok(out)
}

/// Compress uncompressed bytes into an RL chunk payload.
pub fn compress_chunk(data: &[u8]) -> io::Result<Vec<u8>> {
    // Split into DEFAULT_BLOCK_SIZE blocks and compress each
    let mut compressed_blocks: Vec<Vec<u8>> = Vec::new();
    let mut orig_block_sizes: Vec<usize> = Vec::new();

    for chunk in data.chunks(DEFAULT_BLOCK_SIZE) {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(chunk)?;
        let compressed = enc.finish()?;
        orig_block_sizes.push(chunk.len());
        compressed_blocks.push(compressed);
    }

    let total_comp: i32 = compressed_blocks.iter().map(|b| b.len() as i32).sum();
    let total_uncomp = data.len() as i32;

    let mut out = Vec::new();
    out.extend_from_slice(&PACKAGE_FILE_TAG.to_le_bytes());
    out.extend_from_slice(&(DEFAULT_BLOCK_SIZE as i32).to_le_bytes());
    out.extend_from_slice(&total_comp.to_le_bytes());
    out.extend_from_slice(&total_uncomp.to_le_bytes());
    // Block headers
    for (compressed, orig_size) in compressed_blocks.iter().zip(orig_block_sizes.iter()) {
        out.extend_from_slice(&(compressed.len() as i32).to_le_bytes());
        out.extend_from_slice(&(*orig_size as i32).to_le_bytes());
    }
    // Block data
    for compressed in &compressed_blocks {
        out.extend_from_slice(compressed);
    }
    Ok(out)
}
