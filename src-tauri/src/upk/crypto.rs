use aes::Aes256;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::KeyInit;

/// Decrypt data using AES-256-ECB. Input must be a multiple of 16 bytes.
pub fn decrypt_ecb(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    use aes::cipher::BlockDecrypt;
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();
    for chunk in buf.chunks_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }
    buf
}

/// Encrypt data using AES-256-ECB. Pads to multiple of 16 if needed.
pub fn encrypt_ecb(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    use aes::cipher::BlockEncrypt;
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();
    let pad = (16 - buf.len() % 16) % 16;
    buf.extend(std::iter::repeat(0u8).take(pad));
    for chunk in buf.chunks_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    buf
}

/// Load all 32-byte AES keys from keys.txt content (one base64 per line).
pub fn load_keys(keys_txt: &str) -> Vec<[u8; 32]> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    keys_txt
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let bytes = engine.decode(line).ok()?;
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Some(arr)
            } else {
                None
            }
        })
        .collect()
}

/// Load package_name_lowercase → [u8;32] key mapping from keys_map.json content.
pub fn load_keys_map(json: &str) -> std::collections::HashMap<String, [u8; 32]> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let map: std::collections::HashMap<String, String> =
        serde_json::from_str(json).unwrap_or_default();
    map.into_iter()
        .filter_map(|(k, v)| {
            let bytes = engine.decode(&v).ok()?;
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Some((k, arr))
            } else {
                None
            }
        })
        .collect()
}

/// Try to find the correct AES-256-ECB key for an RL UPK encrypted block.
///
/// `encrypted_block`: the raw encrypted bytes starting at name_offset in the file.
/// `depends_offset`: the depends_offset from the file summary (used for verification).
/// `chunks_offset`: the compressed_chunks_offset (relative to start of encrypted_block).
/// `keys`: the list of candidate keys.
///
/// Returns the matching key if found.
pub fn find_valid_key(
    encrypted_block: &[u8],
    depends_offset: i32,
    chunks_offset: i32,
    keys: &[[u8; 32]],
) -> Option<[u8; 32]> {
    // Align chunks_offset to 16-byte AES block boundary
    let block_start = (chunks_offset as usize) & !15;
    let block_end = block_start + 32;
    if block_end > encrypted_block.len() {
        return None;
    }
    let probe = &encrypted_block[block_start..block_end];

    for &key in keys {
        let decrypted = decrypt_ecb(&key, probe);
        let inner = (chunks_offset as usize) % 16;
        if inner + 12 > decrypted.len() {
            continue;
        }
        let chunk_count = i32::from_le_bytes(
            decrypted[inner..inner + 4].try_into().unwrap(),
        );
        let unc_off = i64::from_le_bytes(
            decrypted[inner + 4..inner + 12].try_into().unwrap(),
        );
        if chunk_count >= 1 && chunk_count <= 8 && unc_off == depends_offset as i64 {
            return Some(key);
        }
    }
    None
}

/// Relaxed key check used for keys_map trusted entries.
/// Seekfree (_SF) packages may have unc_off ≠ depends_offset, so we only
/// require chunk_count to be in a sane range.
pub fn find_valid_key_relaxed(
    encrypted_block: &[u8],
    chunks_offset: i32,
    keys: &[[u8; 32]],
) -> Option<[u8; 32]> {
    let block_start = (chunks_offset as usize) & !15;
    let block_end = block_start + 32;
    if block_end > encrypted_block.len() {
        return None;
    }
    let probe = &encrypted_block[block_start..block_end];

    for &key in keys {
        let decrypted = decrypt_ecb(&key, probe);
        let inner = (chunks_offset as usize) % 16;
        if inner + 4 > decrypted.len() {
            continue;
        }
        let chunk_count = i32::from_le_bytes(
            decrypted[inner..inner + 4].try_into().unwrap(),
        );
        if chunk_count >= 1 && chunk_count <= 256 {
            return Some(key);
        }
    }
    None
}
