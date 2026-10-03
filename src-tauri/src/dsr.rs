/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 *
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use std::sync::Mutex;

pub const SESSION_ID_LEN: usize = 16;
pub const SEQ_LEN: usize = 4;
pub const IV_LEN: usize = 16;
pub const HMAC_LEN: usize = 32;
pub const KEY_LEN: usize = 32;
pub const HEADER_LEN: usize = SESSION_ID_LEN + SEQ_LEN + IV_LEN;
pub const BLOCK: usize = 16;

#[derive(Clone, Debug)]
pub struct DsrSession {
    pub reservation_id: String,
    pub message_type: String,
    pub server_address: String,
    pub ping_address: String,
    pub product_ids: Vec<i64>,
    pub key: Vec<u8>,
    pub iv: Vec<u8>,
    pub hmac_key: Vec<u8>,
    pub session_id: Vec<u8>,
    pub captured_at: String,
}

impl DsrSession {
    pub fn is_complete(&self) -> bool {
        self.key.len() == KEY_LEN && self.iv.len() == IV_LEN
    }

    pub fn summary(&self) -> String {
        let mut out = String::new();
        let push = |label: &str, value: &str, out: &mut String| {
            if !value.is_empty() {
                out.push_str(label);
                out.push_str(": ");
                out.push_str(value);
                out.push('\n');
            }
        };
        let key_hex = |bytes: &[u8]| -> String {
            if bytes.is_empty() {
                String::new()
            } else {
                format!("{} ({} bytes)", crate::applog::hex_encode(bytes), bytes.len())
            }
        };
        let product_ids = self
            .product_ids
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        push("MessageType", &self.message_type, &mut out);
        push("ReservationID", &self.reservation_id, &mut out);
        push("ServerAddress", &self.server_address, &mut out);
        push("PingAddress", &self.ping_address, &mut out);
        push("ProductIDs", &product_ids, &mut out);
        push("Key (AES-256, 32B)", &key_hex(&self.key), &mut out);
        push("IV (16B)", &key_hex(&self.iv), &mut out);
        push("HMACKey (32B)", &key_hex(&self.hmac_key), &mut out);
        push("SessionID (16B)", &key_hex(&self.session_id), &mut out);
        out
    }
}

static SESSIONS: Mutex<Vec<DsrSession>> = Mutex::new(Vec::new());
const MAX_SESSIONS: usize = 32;

pub fn parse_reservation(body: &[u8]) -> Option<DsrSession> {
    use base64::Engine;
    let outer: serde_json::Value = serde_json::from_slice(body).ok()?;
    let reservation_id = outer
        .get("ReservationID")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message_type = outer
        .get("MessageType")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let payload: serde_json::Value = match outer.get("MessagePayload") {
        Some(serde_json::Value::String(s)) => serde_json::from_str(s).ok()?,
        Some(v) if v.is_object() => v.clone(),
        _ => return None,
    };

    let field = |name: &str| -> String {
        payload
            .get(name)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let keys = payload.get("Keys");
    let decode = |name: &str| -> Vec<u8> {
        keys.and_then(|k| k.get(name))
            .and_then(|v| v.as_str())
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
            .unwrap_or_default()
    };

    Some(DsrSession {
        reservation_id,
        message_type,
        server_address: field("ServerAddress"),
        ping_address: field("PingAddress"),
        product_ids: payload
            .get("ProductIDs")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
            .unwrap_or_default(),
        key: decode("Key"),
        iv: decode("IV"),
        hmac_key: decode("HMACKey"),
        session_id: decode("SessionID"),
        captured_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

pub fn register(session: DsrSession) -> bool {
    let complete = session.is_complete();
    if let Ok(mut guard) = SESSIONS.lock() {
        if !session.reservation_id.is_empty() {
            if let Some(pos) = guard
                .iter()
                .position(|s| s.reservation_id == session.reservation_id)
            {
                guard[pos] = session;
                return complete;
            }
        }
        guard.push(session);
        if guard.len() > MAX_SESSIONS {
            let excess = guard.len() - MAX_SESSIONS;
            guard.drain(0..excess);
        }
    }
    complete
}

pub fn sessions() -> Vec<DsrSession> {
    SESSIONS.lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn latest() -> Option<DsrSession> {
    SESSIONS.lock().ok().and_then(|g| g.last().cloned())
}

pub fn session_for(address: &str) -> Option<DsrSession> {
    let host = address.split(':').next().unwrap_or(address);
    SESSIONS.lock().ok().and_then(|g| {
        g.iter()
            .rev()
            .find(|s| {
                s.server_address == address
                    || s.ping_address == address
                    || s.server_address.starts_with(host)
                    || s.ping_address.starts_with(host)
            })
            .cloned()
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum DsrError {
    TooShort,
    BadCipherLen,
    HmacMismatch,
    BadPadding,
}

impl std::fmt::Display for DsrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            DsrError::TooShort => "datagram shorter than the DSR header + one AES block + HMAC",
            DsrError::BadCipherLen => "ciphertext length is not a positive multiple of 16",
            DsrError::HmacMismatch => "HMAC-SHA256 verification failed (tampered or wrong session key)",
            DsrError::BadPadding => "invalid PKCS#7 padding after AES-256-CBC decrypt",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for DsrError {}

pub fn decrypt_datagram(datagram: &[u8], session: &DsrSession) -> Result<Vec<u8>, DsrError> {
    if datagram.len() < HEADER_LEN + BLOCK + HMAC_LEN {
        return Err(DsrError::TooShort);
    }
    let session_id = &datagram[..SESSION_ID_LEN];
    let seq = &datagram[SESSION_ID_LEN..SESSION_ID_LEN + SEQ_LEN];
    let iv = &datagram[SESSION_ID_LEN + SEQ_LEN..HEADER_LEN];
    let ciphertext_end = datagram.len() - HMAC_LEN;
    let ciphertext = &datagram[HEADER_LEN..ciphertext_end];
    let mac = &datagram[ciphertext_end..];

    if ciphertext.is_empty() || ciphertext.len() % BLOCK != 0 {
        return Err(DsrError::BadCipherLen);
    }

    if !session.hmac_key.is_empty() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac_ctx = Hmac::<Sha256>::new_from_slice(&session.hmac_key)
            .map_err(|_| DsrError::HmacMismatch)?;
        mac_ctx.update(session_id);
        mac_ctx.update(seq);
        mac_ctx.update(iv);
        mac_ctx.update(ciphertext);
        let expected = mac_ctx.finalize().into_bytes();
        let mut diff = 0u8;
        for (a, b) in expected.iter().zip(mac.iter()) {
            diff |= a ^ b;
        }
        if diff != 0 || expected.len() != mac.len() {
            return Err(DsrError::HmacMismatch);
        }
    }

    aes256_cbc_decrypt(&session.key, iv, ciphertext)
}

fn aes256_cbc_decrypt(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, DsrError> {
    use aes::Aes256;
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockDecrypt, KeyInit};

    if key.len() != KEY_LEN || iv.len() != IV_LEN || ciphertext.is_empty() || ciphertext.len() % BLOCK != 0 {
        return Err(DsrError::BadCipherLen);
    }

    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut prev = [0u8; BLOCK];
    prev.copy_from_slice(iv);

    let mut out = Vec::with_capacity(ciphertext.len());
    for chunk in ciphertext.chunks_exact(BLOCK) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        for i in 0..BLOCK {
            out.push(block[i] ^ prev[i]);
        }
        prev.copy_from_slice(chunk);
    }

    let pad = *out.last().ok_or(DsrError::BadPadding)? as usize;
    if pad == 0 || pad > BLOCK || pad > out.len() {
        return Err(DsrError::BadPadding);
    }
    let n = out.len();
    if out[n - pad..].iter().any(|&b| b as usize != pad) {
        return Err(DsrError::BadPadding);
    }
    out.truncate(n - pad);
    Ok(out)
}

#[derive(Clone, serde::Serialize)]
pub struct DsrSessionView {
    pub reservation_id: String,
    pub message_type: String,
    pub server_address: String,
    pub ping_address: String,
    pub product_ids: Vec<i64>,
    pub key_hex: String,
    pub iv_hex: String,
    pub hmac_key_hex: String,
    pub session_id_hex: String,
    pub complete: bool,
    pub captured_at: String,
}

impl DsrSession {
    pub fn to_view(&self) -> DsrSessionView {
        DsrSessionView {
            reservation_id: self.reservation_id.clone(),
            message_type: self.message_type.clone(),
            server_address: self.server_address.clone(),
            ping_address: self.ping_address.clone(),
            product_ids: self.product_ids.clone(),
            key_hex: crate::applog::hex_encode(&self.key),
            iv_hex: crate::applog::hex_encode(&self.iv),
            hmac_key_hex: crate::applog::hex_encode(&self.hmac_key),
            session_id_hex: crate::applog::hex_encode(&self.session_id),
            complete: self.is_complete(),
            captured_at: self.captured_at.clone(),
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct DsrPlaintext {
    pub reservation_id: String,
    pub server_address: String,
    pub len: usize,
    pub hex: String,
    pub text: Option<String>,
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut hi: Option<u8> = None;
    for ch in input.chars() {
        if ch.is_ascii_hexdigit() {
            let v = ch.to_digit(16)? as u8;
            match hi.take() {
                Some(h) => out.push((h << 4) | v),
                None => hi = Some(v),
            }
        } else if ch == ' ' || ch == ':' || ch == '-' || ch == '\n' || ch == '\r' || ch == '\t' {
            continue;
        } else {
            return None;
        }
    }
    if hi.is_some() {
        return None;
    }
    Some(out)
}

#[tauri::command]
pub fn get_dsr_sessions() -> Vec<DsrSessionView> {
    sessions().iter().map(|s| s.to_view()).collect()
}

#[tauri::command]
pub fn decrypt_dsr_datagram(hex: String, address: Option<String>) -> Result<DsrPlaintext, String> {
    let datagram = decode_hex(&hex).ok_or_else(|| "invalid hex datagram".to_string())?;
    let session = match address.as_deref() {
        Some(addr) if !addr.is_empty() => session_for(addr)
            .or_else(latest)
            .ok_or_else(|| format!("no DSR session known for {addr}"))?,
        _ => latest().ok_or_else(|| "no DSR session captured yet".to_string())?,
    };

    if !session.is_complete() {
        return Err("DSR session is missing Key/IV — cannot decrypt yet".to_string());
    }

    let plaintext = decrypt_datagram(&datagram, &session).map_err(|e| e.to_string())?;
    let text = String::from_utf8(plaintext.clone()).ok();
    Ok(DsrPlaintext {
        reservation_id: session.reservation_id,
        server_address: session.server_address,
        len: plaintext.len(),
        hex: crate::applog::hex_encode(&plaintext),
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> DsrSession {
        DsrSession {
            reservation_id: "efeab65b9b96d4b222e70c874cfa6d1f".into(),
            message_type: "ReservationsReadyMessage_X".into(),
            server_address: "18.156.221.11:9093".into(),
            ping_address: "18.156.221.11:15093".into(),
            product_ids: vec![4284, 4293, 376],
            key: vec![0x11; KEY_LEN],
            iv: vec![0x22; IV_LEN],
            hmac_key: vec![0x33; KEY_LEN],
            session_id: vec![0x44; SESSION_ID_LEN],
            captured_at: "2026-10-03 00:00:00".into(),
        }
    }

    fn cbc_encrypt(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Vec<u8> {
        use aes::Aes256;
        use aes::cipher::generic_array::GenericArray;
        use aes::cipher::{BlockEncrypt, KeyInit};

        let cipher = Aes256::new(GenericArray::from_slice(key));
        let mut data = plaintext.to_vec();
        let pad = BLOCK - data.len() % BLOCK;
        data.extend(std::iter::repeat(pad as u8).take(pad));

        let mut prev = [0u8; BLOCK];
        prev.copy_from_slice(iv);
        for chunk in data.chunks_exact_mut(BLOCK) {
            for i in 0..BLOCK {
                chunk[i] ^= prev[i];
            }
            let block = GenericArray::from_mut_slice(chunk);
            cipher.encrypt_block(block);
            prev.copy_from_slice(chunk);
        }
        data
    }

    fn build_datagram(session: &DsrSession, seq: u32, plaintext: &[u8], iv: &[u8]) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let ciphertext = cbc_encrypt(&session.key, iv, plaintext);
        let mut datagram = Vec::new();
        datagram.extend_from_slice(&session.session_id);
        datagram.extend_from_slice(&seq.to_le_bytes());
        datagram.extend_from_slice(iv);
        datagram.extend_from_slice(&ciphertext);

        let mut ctx = Hmac::<Sha256>::new_from_slice(&session.hmac_key).unwrap();
        ctx.update(&datagram);
        datagram.extend_from_slice(&ctx.finalize().into_bytes());
        datagram
    }

    #[test]
    fn decrypts_a_roundtripped_datagram() {
        let session = sample_session();
        let datagram = build_datagram(&session, 7, b"hello rocket league", &[0x77; IV_LEN]);
        let plaintext = decrypt_datagram(&datagram, &session).unwrap();
        assert_eq!(plaintext, b"hello rocket league");
    }

    #[test]
    fn rejects_tampered_hmac() {
        let session = sample_session();
        let mut datagram = build_datagram(&session, 1, b"payload", &[0x55; IV_LEN]);
        let last = datagram.len() - 1;
        datagram[last] ^= 0xff;
        assert_eq!(decrypt_datagram(&datagram, &session), Err(DsrError::HmacMismatch));
    }

    #[test]
    fn rejects_short_datagram() {
        let session = sample_session();
        assert_eq!(decrypt_datagram(&[0u8; 12], &session), Err(DsrError::TooShort));
    }

    #[test]
    fn rejects_bad_padding() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let session = sample_session();
        let iv = [0x09u8; IV_LEN];
        let mut datagram = Vec::new();
        datagram.extend_from_slice(&session.session_id);
        datagram.extend_from_slice(&1u32.to_le_bytes());
        datagram.extend_from_slice(&iv);
        datagram.extend_from_slice(&[0u8; BLOCK]);
        let mut ctx = Hmac::<Sha256>::new_from_slice(&session.hmac_key).unwrap();
        ctx.update(&datagram);
        datagram.extend_from_slice(&ctx.finalize().into_bytes());
        assert_eq!(decrypt_datagram(&datagram, &session), Err(DsrError::BadPadding));
    }

    #[test]
    fn parses_reservation_and_registers_session() {
        use base64::Engine;
        let enc = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let inner = format!(
            "{{\"ServerAddress\":\"18.156.221.11:9093\",\"PingAddress\":\"18.156.221.11:15093\",\"ProductIDs\":[4284,4293],\"Keys\":{{\"Key\":\"{}\",\"IV\":\"{}\",\"HMACKey\":\"{}\",\"SessionID\":\"{}\"}}}}",
            enc(&[0xAA; KEY_LEN]),
            enc(&[0xBB; IV_LEN]),
            enc(&[0xCC; KEY_LEN]),
            enc(&[0xDD; SESSION_ID_LEN]),
        );
        let outer = format!(
            "{{\"ReservationID\":\"efeab65b\",\"MessageType\":\"ReservationsReadyMessage_X\",\"MessagePayload\":{}}}",
            serde_json::to_string(&inner).unwrap()
        );

        let session = parse_reservation(outer.as_bytes()).expect("reservation must parse");
        assert_eq!(session.server_address, "18.156.221.11:9093");
        assert_eq!(session.ping_address, "18.156.221.11:15093");
        assert_eq!(session.product_ids, vec![4284, 4293]);
        assert_eq!(session.key, vec![0xAA; KEY_LEN]);
        assert_eq!(session.iv, vec![0xBB; IV_LEN]);
        assert_eq!(session.hmac_key, vec![0xCC; KEY_LEN]);
        assert_eq!(session.session_id, vec![0xDD; SESSION_ID_LEN]);
        assert!(session.is_complete());

        assert!(register(session.clone()));
        assert_eq!(latest().unwrap().reservation_id, "efeab65b");
        assert_eq!(
            session_for("18.156.221.11:9093").unwrap().reservation_id,
            "efeab65b"
        );
    }
}
