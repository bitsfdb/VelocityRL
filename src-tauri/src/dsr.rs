/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 *
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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

pub fn start_udp_relay(session: DsrSession) -> Option<u16> {
    let target_addr: SocketAddr = session
        .server_address
        .parse()
        .ok()
        .or_else(|| {
            use std::net::ToSocketAddrs;
            session.server_address.to_socket_addrs().ok()?.next()
        })?;

    let client_socket = match UdpSocket::bind("127.0.0.1:0") {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log::error!("dsr_relay: failed to bind local UDP listener: {e}");
            return None;
        }
    };
    let local_port = client_socket.local_addr().ok()?.port();

    let server_socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log::error!("dsr_relay: failed to bind upstream UDP socket: {e}");
            return None;
        }
    };

    let client_addr_holder = Arc::new(Mutex::new(None::<SocketAddr>));
    let last_log_c2s = Arc::new(AtomicU64::new(0));
    let last_log_s2c = Arc::new(AtomicU64::new(0));
    let start_time = Instant::now();

    crate::applog::event(&format!(
        "dsr_relay: active on 127.0.0.1:{local_port} -> {target_addr}"
    ));

    // Thread 1: Client -> Server
    {
        let c_sock = client_socket.clone();
        let s_sock = server_socket.clone();
        let c_addr = client_addr_holder.clone();
        let sess = session.clone();
        let l_log = last_log_c2s.clone();
        std::thread::Builder::new()
            .name("velocity-udp-c2s".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    match c_sock.recv_from(&mut buf) {
                        Ok((n, from)) if n > 0 => {
                            if let Ok(mut g) = c_addr.lock() {
                                *g = Some(from);
                            }
                            let packet = &buf[..n];

                            // Rate limit traffic event logging to 2 per sec to prevent UI freezing
                            let now_ms = start_time.elapsed().as_millis() as u64;
                            let prev = l_log.load(Ordering::Relaxed);
                            if now_ms.saturating_sub(prev) >= 500 {
                                l_log.store(now_ms, Ordering::Relaxed);
                                let (summary, body_bytes) = match decrypt_datagram(packet, &sess) {
                                    Ok(pt) => {
                                        let text_opt = String::from_utf8(pt.clone()).ok();
                                        let content = if let Some(ref text) = text_opt {
                                            if text.chars().all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t') {
                                                text.clone()
                                            } else {
                                                format!(
                                                    "--- DECRYPTED PLAINTEXT ({} bytes) ---\nHex: {}\n\n--- RAW CIPHERTEXT ({} bytes) ---\nHex: {}",
                                                    pt.len(),
                                                    crate::applog::hex_encode(&pt),
                                                    packet.len(),
                                                    crate::applog::hex_encode(packet)
                                                )
                                            }
                                        } else {
                                            format!(
                                                "--- DECRYPTED PLAINTEXT ({} bytes) ---\nHex: {}\n\n--- RAW CIPHERTEXT ({} bytes) ---\nHex: {}",
                                                pt.len(),
                                                crate::applog::hex_encode(&pt),
                                                packet.len(),
                                                crate::applog::hex_encode(packet)
                                            )
                                        };
                                        (format!("DSR Decrypted ({}B -> {}B)", n, pt.len()), content.into_bytes())
                                    }
                                    Err(_) => (
                                        format!("DSR Raw UDP ({}B)", n),
                                        format!(
                                            "--- RAW DATAGRAM ({} bytes) ---\nHex: {}",
                                            packet.len(),
                                            crate::applog::hex_encode(packet)
                                        ).into_bytes(),
                                    ),
                                };
                                crate::applog::record_traffic_event(
                                    "UDP",
                                    "CLIENT->SRV",
                                    &format!("dsr:{}", target_addr.port()),
                                    Some(&target_addr.to_string()),
                                    false,
                                    None,
                                    None,
                                    &body_bytes,
                                    Some(&summary),
                                );
                            }

                            let _ = s_sock.send_to(packet, target_addr);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::debug!("dsr_relay c2s error: {e}");
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    }
                }
            })
            .ok()?;
    }

    // Thread 2: Server -> Client
    {
        let c_sock = client_socket.clone();
        let s_sock = server_socket.clone();
        let c_addr = client_addr_holder.clone();
        let sess = session.clone();
        let l_log = last_log_s2c.clone();
        std::thread::Builder::new()
            .name("velocity-udp-s2c".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
                loop {
                    match s_sock.recv_from(&mut buf) {
                        Ok((n, _from)) if n > 0 => {
                            let packet = &buf[..n];

                            let now_ms = start_time.elapsed().as_millis() as u64;
                            let prev = l_log.load(Ordering::Relaxed);
                            if now_ms.saturating_sub(prev) >= 500 {
                                l_log.store(now_ms, Ordering::Relaxed);
                                let (summary, body_bytes) = match decrypt_datagram(packet, &sess) {
                                    Ok(pt) => {
                                        let text_opt = String::from_utf8(pt.clone()).ok();
                                        let content = if let Some(ref text) = text_opt {
                                            if text.chars().all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t') {
                                                text.clone()
                                            } else {
                                                format!(
                                                    "--- DECRYPTED PLAINTEXT ({} bytes) ---\nHex: {}\n\n--- RAW CIPHERTEXT ({} bytes) ---\nHex: {}",
                                                    pt.len(),
                                                    crate::applog::hex_encode(&pt),
                                                    packet.len(),
                                                    crate::applog::hex_encode(packet)
                                                )
                                            }
                                        } else {
                                            format!(
                                                "--- DECRYPTED PLAINTEXT ({} bytes) ---\nHex: {}\n\n--- RAW CIPHERTEXT ({} bytes) ---\nHex: {}",
                                                pt.len(),
                                                crate::applog::hex_encode(&pt),
                                                packet.len(),
                                                crate::applog::hex_encode(packet)
                                            )
                                        };
                                        (format!("DSR Decrypted ({}B -> {}B)", n, pt.len()), content.into_bytes())
                                    }
                                    Err(_) => (
                                        format!("DSR Raw UDP ({}B)", n),
                                        format!(
                                            "--- RAW DATAGRAM ({} bytes) ---\nHex: {}",
                                            packet.len(),
                                            crate::applog::hex_encode(packet)
                                        ).into_bytes(),
                                    ),
                                };
                                crate::applog::record_traffic_event(
                                    "UDP",
                                    "SRV->CLIENT",
                                    &format!("dsr:{}", target_addr.port()),
                                    Some(&target_addr.to_string()),
                                    false,
                                    None,
                                    None,
                                    &body_bytes,
                                    Some(&summary),
                                );
                            }

                            let patched_opt = patch_udp_server_datagram(packet, &sess);
                            let packet_to_send = patched_opt.as_deref().unwrap_or(packet);

                            let target_client = {
                                c_addr.lock().ok().and_then(|g| *g)
                            };
                            if let Some(to) = target_client {
                                let _ = c_sock.send_to(packet_to_send, to);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::debug!("dsr_relay s2c error: {e}");
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    }
                }
            })
            .ok()?;
    }

    Some(local_port)
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
            DsrError::TooShort => "datagram shorter than the DSR header + one AES block",
            DsrError::BadCipherLen => "ciphertext length is not a positive multiple of 16",
            DsrError::HmacMismatch => "HMAC-SHA256 verification failed (tampered or wrong session key)",
            DsrError::BadPadding => "invalid PKCS#7 padding after AES-256-CBC decrypt",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for DsrError {}

pub fn decrypt_datagram(datagram: &[u8], session: &DsrSession) -> Result<Vec<u8>, DsrError> {
    if datagram.len() < SESSION_ID_LEN + BLOCK {
        return Err(DsrError::TooShort);
    }

    // Layout 1: With HMAC-SHA256 trailer (Priority: if HMAC matches, layout is authenticated)
    if !session.hmac_key.is_empty() && datagram.len() >= HEADER_LEN + BLOCK + HMAC_LEN {
        let ciphertext_end = datagram.len() - HMAC_LEN;
        let ct = &datagram[HEADER_LEN..ciphertext_end];
        let mac = &datagram[ciphertext_end..];
        if ct.len() % BLOCK == 0 {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;

            // Check standard HMAC over Header (36B) + Ciphertext
            let mut matched = false;
            if let Ok(mut mac_ctx) = Hmac::<Sha256>::new_from_slice(&session.hmac_key) {
                mac_ctx.update(&datagram[..ciphertext_end]);
                if mac_ctx.finalize().into_bytes().as_slice() == mac {
                    matched = true;
                }
            }

            // Check HMAC over Seq+IV+Ciphertext (excluding SessionID)
            if !matched {
                if let Ok(mut mac_ctx) = Hmac::<Sha256>::new_from_slice(&session.hmac_key) {
                    mac_ctx.update(&datagram[SESSION_ID_LEN..ciphertext_end]);
                    if mac_ctx.finalize().into_bytes().as_slice() == mac {
                        matched = true;
                    }
                }
            }

            if matched {
                let iv = &datagram[SESSION_ID_LEN + SEQ_LEN..HEADER_LEN];
                if let Ok(res) = aes256_cbc_decrypt(&session.key, iv, ct) {
                    return Ok(res);
                }
            }
        }
    }

    // Layout 2: SessionID(16) + Seq(4) + Per-packet IV(16) + Ciphertext (Header=36)
    if datagram.len() >= HEADER_LEN + BLOCK {
        let iv = &datagram[SESSION_ID_LEN + SEQ_LEN..HEADER_LEN];
        let ct = &datagram[HEADER_LEN..];
        if ct.len() % BLOCK == 0 {
            if let Ok(res) = aes256_cbc_decrypt(&session.key, iv, ct) {
                return Ok(res);
            }
        }
    }

    // Layout 3: SessionID(16) + Per-packet IV(16) + Ciphertext(multiple of 16)
    if datagram.len() >= SESSION_ID_LEN + IV_LEN + BLOCK {
        let iv = &datagram[SESSION_ID_LEN..SESSION_ID_LEN + IV_LEN];
        let ct = &datagram[SESSION_ID_LEN + IV_LEN..];
        if ct.len() % BLOCK == 0 {
            if let Ok(res) = aes256_cbc_decrypt(&session.key, iv, ct) {
                return Ok(res);
            }
        }
    }

    // Layout 4: SessionID(16) + Ciphertext(multiple of 16) using session.iv from reservation
    if session.iv.len() == IV_LEN && datagram.len() >= SESSION_ID_LEN + BLOCK {
        let ct = &datagram[SESSION_ID_LEN..];
        if ct.len() % BLOCK == 0 {
            if let Ok(res) = aes256_cbc_decrypt(&session.key, &session.iv, ct) {
                return Ok(res);
            }
        }
    }

    // Layout 5: Entire datagram is AES-256-CBC with session.iv (if len % 16 == 0)
    if session.iv.len() == IV_LEN && datagram.len() % BLOCK == 0 {
        if let Ok(res) = aes256_cbc_decrypt(&session.key, &session.iv, datagram) {
            return Ok(res);
        }
    }

    Err(DsrError::BadCipherLen)
}

pub fn aes256_cbc_decrypt(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, DsrError> {
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

    // If valid PKCS#7 padding, strip it; otherwise return raw plaintext
    if let Some(&pad_byte) = out.last() {
        let pad = pad_byte as usize;
        let n = out.len();
        if pad > 0 && pad <= BLOCK && pad <= n && out[n - pad..].iter().all(|&b| b as usize == pad) {
            out.truncate(n - pad);
        }
    }
    Ok(out)
}

pub fn cbc_encrypt(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Vec<u8> {
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

pub fn encrypt_datagram(
    plaintext: &[u8],
    session: &DsrSession,
    seq: u32,
    iv: &[u8],
    include_hmac: bool,
) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let ciphertext = cbc_encrypt(&session.key, iv, plaintext);
    let mut datagram = Vec::with_capacity(HEADER_LEN + ciphertext.len() + if include_hmac { HMAC_LEN } else { 0 });
    datagram.extend_from_slice(&session.session_id);
    datagram.extend_from_slice(&seq.to_le_bytes());
    datagram.extend_from_slice(iv);
    datagram.extend_from_slice(&ciphertext);

    if include_hmac && !session.hmac_key.is_empty() {
        if let Ok(mut ctx) = Hmac::<Sha256>::new_from_slice(&session.hmac_key) {
            ctx.update(&datagram);
            datagram.extend_from_slice(&ctx.finalize().into_bytes());
        }
    }
    datagram
}

pub fn reencrypt_patched_datagram(
    orig_datagram: &[u8],
    new_plaintext: &[u8],
    session: &DsrSession,
) -> Option<Vec<u8>> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // Layout 1: With HMAC-SHA256 trailer
    if !session.hmac_key.is_empty() && orig_datagram.len() >= HEADER_LEN + BLOCK + HMAC_LEN {
        let seq = u32::from_le_bytes(
            orig_datagram[SESSION_ID_LEN..SESSION_ID_LEN + SEQ_LEN]
                .try_into()
                .ok()?,
        );
        let iv = &orig_datagram[SESSION_ID_LEN + SEQ_LEN..HEADER_LEN];
        let new_ct = cbc_encrypt(&session.key, iv, new_plaintext);

        let mut new_datagram = Vec::with_capacity(HEADER_LEN + new_ct.len() + HMAC_LEN);
        new_datagram.extend_from_slice(&orig_datagram[..HEADER_LEN]);
        new_datagram.extend_from_slice(&new_ct);

        if let Ok(mut mac_ctx) = Hmac::<Sha256>::new_from_slice(&session.hmac_key) {
            mac_ctx.update(&new_datagram[..HEADER_LEN + new_ct.len()]);
            new_datagram.extend_from_slice(&mac_ctx.finalize().into_bytes());
            return Some(new_datagram);
        }
    }

    // Layout 2: Header (36B) + Ciphertext without HMAC
    if orig_datagram.len() >= HEADER_LEN + BLOCK {
        let seq = u32::from_le_bytes(
            orig_datagram[SESSION_ID_LEN..SESSION_ID_LEN + SEQ_LEN]
                .try_into()
                .ok()?,
        );
        let iv = &orig_datagram[SESSION_ID_LEN + SEQ_LEN..HEADER_LEN];
        let new_ct = cbc_encrypt(&session.key, iv, new_plaintext);
        let mut new_datagram = Vec::with_capacity(HEADER_LEN + new_ct.len());
        new_datagram.extend_from_slice(&orig_datagram[..HEADER_LEN]);
        new_datagram.extend_from_slice(&new_ct);
        return Some(new_datagram);
    }

    None
}

pub fn patch_udp_server_datagram(
    packet: &[u8],
    sess: &DsrSession,
) -> Option<Vec<u8>> {
    let mut pt = decrypt_datagram(packet, sess).ok()?;

    let spoof_cfg = crate::psynet::load_active_spoof_from_disk()?;
    let inv = spoof_cfg.inventory_spoof.as_ref()?;
    if !inv.enabled || inv.items.is_empty() {
        return None;
    }

    let mut changed = false;

    for item in &inv.items {
        if item.product_id <= 0 {
            continue;
        }
        let target_pid = item.product_id as u32;
        let target_bytes = target_pid.to_le_bytes();

        let norm_slot = item.slot.to_lowercase().replace([' ', '_', '-'], "");
        let (default_pids, is_body): (&[u32], bool) = if norm_slot.contains("wheel") {
            (&[27, 28, 1580, 376, 1948], false)
        } else if norm_slot.contains("boost") || norm_slot.contains("rocket_trail") {
            (&[64, 63, 33, 3763], false)
        } else if norm_slot.contains("trail") {
            (&[1907], false)
        } else if norm_slot.contains("explosion") || norm_slot.contains("goal") {
            (&[7726], false)
        } else if norm_slot.contains("body") {
            (&[23], true)
        } else {
            (&[], false)
        };

        let mut to_replace = default_pids.to_vec();
        for &sess_pid in &sess.product_ids {
            let sess_u32 = sess_pid as u32;
            if sess_u32 > 0 && sess_u32 != target_pid && !to_replace.contains(&sess_u32) {
                if default_pids.contains(&sess_u32) {
                    to_replace.push(sess_u32);
                }
            }
        }

        for def_pid in to_replace {
            if def_pid == target_pid {
                continue;
            }
            if is_body && target_pid == 23 {
                continue;
            }
            let def_bytes = def_pid.to_le_bytes();
            let mut offset = 0;
            while offset + 4 <= pt.len() {
                if pt[offset..offset + 4] == def_bytes {
                    pt[offset..offset + 4].copy_from_slice(&target_bytes);
                    changed = true;
                    offset += 4;
                } else {
                    offset += 1;
                }
            }
        }
    }

    if !changed {
        return None;
    }

    reencrypt_patched_datagram(packet, &pt, sess)
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

    #[test]
    fn decrypts_a_roundtripped_datagram_with_hmac() {
        let session = sample_session();
        let datagram = encrypt_datagram(b"hello rocket league", &session, 7, &[0x77; IV_LEN], true);
        let plaintext = decrypt_datagram(&datagram, &session).unwrap();
        assert_eq!(plaintext, b"hello rocket league");
    }

    #[test]
    fn decrypts_a_roundtripped_datagram_no_hmac() {
        let mut session = sample_session();
        session.hmac_key.clear();
        let datagram = encrypt_datagram(b"hello rocket league raw", &session, 7, &[0x77; IV_LEN], false);
        let plaintext = decrypt_datagram(&datagram, &session).unwrap();
        assert_eq!(plaintext, b"hello rocket league raw");
    }

    #[test]
    fn rejects_tampered_hmac() {
        let session = sample_session();
        let mut datagram = encrypt_datagram(b"payload", &session, 1, &[0x55; IV_LEN], true);
        let last = datagram.len() - 1;
        datagram[last] ^= 0xff;
        // Tampered HMAC will fall back to attempting no-hmac on bad ciphertext length or failing HMAC
        assert!(decrypt_datagram(&datagram, &session).is_err());
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
