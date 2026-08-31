use aes::Aes256;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::KeyInit;

fn i32_le(buf: &[u8], off: usize) -> Option<i32> {
    buf.get(off..off + 4)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(i32::from_le_bytes)
}

fn i64_le(buf: &[u8], off: usize) -> Option<i64> {
    buf.get(off..off + 8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(i64::from_le_bytes)
}

pub fn decrypt_ecb(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    use aes::cipher::BlockDecrypt;
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();
    for chunk in buf.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block(block);
    }
    buf
}

pub fn encrypt_ecb(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    use aes::cipher::BlockEncrypt;
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();
    let pad = (16 - buf.len() % 16) % 16;
    buf.extend(std::iter::repeat(0u8).take(pad));
    for chunk in buf.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.encrypt_block(block);
    }
    buf
}

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

pub fn find_valid_key(
    encrypted_block: &[u8],
    depends_offset: i32,
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
        let Some(chunk_count) = i32_le(&decrypted, inner) else {
            continue;
        };
        let Some(unc_off) = i64_le(&decrypted, inner + 4) else {
            continue;
        };

        if chunk_count >= 1 && chunk_count <= 65_536 && unc_off == depends_offset as i64 {
            return Some(key);
        }
    }
    None
}

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
        let Some(chunk_count) = i32_le(&decrypted, inner) else {
            continue;
        };
        if chunk_count >= 1 && chunk_count <= 256 {
            return Some(key);
        }
    }
    None
}
