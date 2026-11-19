/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use aes::Aes256;
use cbc::{Decryptor, Encryptor};
use cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};

type Aes256CbcDec = Decryptor<Aes256>;
type Aes256CbcEnc = Encryptor<Aes256>;

/// How many recently seen reservation keys to keep trying per datagram.
const MAX_CRYPTO_CANDIDATES: usize = 4;

/// Modern Rocket League UDP Match Security Session Keys
#[derive(Clone, Debug)]
pub struct ReservationCrypto {
    pub aes_key: [u8; 32],
    pub aes_iv: [u8; 16],
    /// Present in the reservation payload; the current match protocol does not
    /// append a MAC to each datagram, so this is retained for diagnostics only.
    pub hmac_key: [u8; 32],
}

impl ReservationCrypto {
    pub fn new(aes_key: [u8; 32], aes_iv: [u8; 16], hmac_key: [u8; 32]) -> Self {
        Self {
            aes_key,
            aes_iv,
            hmac_key,
        }
    }
}

/// Shared state for the UDP Match Proxy.
struct ProxyState {
    /// Local UDP port Rocket League connects to (e.g. 127.0.0.1:local_port).
    local_port: AtomicU16,
    /// Real game server address parsed from AddReservationMessage.
    target_server: RwLock<Option<SocketAddr>>,
    /// Learned endpoint of the local Rocket League client.
    client_endpoint: RwLock<Option<SocketAddr>>,
    /// Candidate AES-256-CBC session keys parsed from reservations. PsyNet can
    /// re-issue a reservation with a fresh key that the match never uses, so we
    /// keep a few and use whichever one actually decrypts the datagram.
    active_crypto: RwLock<Vec<ReservationCrypto>>,
    /// Whether the proxy is active.
    running: AtomicBool,
}

static PROXY_STATE: once_cell_or_lazy::LazyState = once_cell_or_lazy::LazyState::new();
static PING_SPOOF: std::sync::RwLock<Option<crate::psynet::PingSpoofPayload>> =
    std::sync::RwLock::new(None);

pub fn set_ping_spoof(cfg: Option<crate::psynet::PingSpoofPayload>) {
    if let Ok(mut lock) = PING_SPOOF.write() {
        *lock = cfg;
    }
}

pub fn get_ping_spoof() -> Option<crate::psynet::PingSpoofPayload> {
    if let Ok(lock) = PING_SPOOF.read() {
        if lock.is_some() {
            return lock.clone();
        }
    }
    if let Some(disk) = crate::psynet::load_active_spoof_from_disk() {
        if let Some(ps) = disk.ping_spoof {
            set_ping_spoof(Some(ps.clone()));
            return Some(ps);
        }
    }
    None
}

/// One captured match datagram. `payload` is decrypted plaintext when
/// `decrypted` is true, otherwise the raw bytes that crossed the wire.
#[derive(Clone, Debug)]
pub struct UdpCaptureEntry {
    pub timestamp: String,
    pub direction: String,
    pub endpoint: String,
    pub encrypted: bool,
    pub decrypted: bool,
    pub payload: Vec<u8>,
}

const MAX_UDP_CAPTURE_PACKETS: usize = 4096;
const MAX_UDP_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

static UDP_CAPTURE: std::sync::Mutex<Option<std::collections::VecDeque<UdpCaptureEntry>>> =
    std::sync::Mutex::new(None);
static UDP_CAPTURE_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub fn record_udp_capture(
    direction: &str,
    endpoint: &str,
    encrypted: bool,
    decrypted: bool,
    payload: &[u8],
) {
    let entry = UdpCaptureEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        direction: direction.to_string(),
        endpoint: endpoint.to_string(),
        encrypted,
        decrypted,
        payload: payload.to_vec(),
    };

    if let Ok(mut lock) = UDP_CAPTURE.lock() {
        let deque = lock.get_or_insert_with(|| std::collections::VecDeque::with_capacity(1024));
        let mut total = UDP_CAPTURE_BYTES.load(Ordering::Relaxed);
        while !deque.is_empty()
            && (deque.len() >= MAX_UDP_CAPTURE_PACKETS
                || total.saturating_add(payload.len()) > MAX_UDP_CAPTURE_BYTES)
        {
            if let Some(old) = deque.pop_front() {
                total = total.saturating_sub(old.payload.len());
            }
        }
        total = total.saturating_add(payload.len());
        deque.push_back(entry);
        UDP_CAPTURE_BYTES.store(total, Ordering::Relaxed);
    }
}

pub fn get_udp_capture() -> Vec<UdpCaptureEntry> {
    if let Ok(lock) = UDP_CAPTURE.lock() {
        if let Some(deque) = lock.as_ref() {
            return deque.iter().cloned().collect();
        }
    }
    Vec::new()
}

pub fn clear_udp_capture() {
    if let Ok(mut lock) = UDP_CAPTURE.lock() {
        if let Some(deque) = lock.as_mut() {
            deque.clear();
        }
    }
    UDP_CAPTURE_BYTES.store(0, Ordering::Relaxed);
}

/// Sample a random simulated ping in milliseconds from the configured interval [min_ms..=max_ms]
pub fn sample_spoofed_ping_ms() -> Option<u32> {
    let cfg = get_ping_spoof()?;
    if !cfg.enabled {
        return None;
    }
    let min = cfg.min_ms.min(cfg.max_ms);
    let max = cfg.min_ms.max(cfg.max_ms);
    if min == max {
        Some(min)
    } else {
        use rand::Rng;
        Some(rand::thread_rng().gen_range(min..=max))
    }
}

mod once_cell_or_lazy {
    use super::*;
    use std::sync::OnceLock;

    pub struct LazyState {
        inner: OnceLock<Arc<ProxyState>>,
    }

    impl LazyState {
        pub const fn new() -> Self {
            Self {
                inner: OnceLock::new(),
            }
        }

        pub fn get(&self) -> Arc<ProxyState> {
            self.inner
                .get_or_init(|| {
                    Arc::new(ProxyState {
                        local_port: AtomicU16::new(0),
                        target_server: RwLock::new(None),
                        client_endpoint: RwLock::new(None),
                        active_crypto: RwLock::new(Vec::new()),
                        running: AtomicBool::new(false),
                    })
                })
                .clone()
        }
    }
}

/// Start or ensure the UDP proxy is running, returning the local bound port.
pub async fn ensure_udp_proxy() -> Result<u16, String> {
    let state = PROXY_STATE.get();

    let current_port = state.local_port.load(Ordering::SeqCst);
    if current_port > 0 && state.running.load(Ordering::SeqCst) {
        return Ok(current_port);
    }

    // Bind local client-facing socket to 127.0.0.1 (ephemeral port)
    let client_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind local UDP proxy socket: {e}"))?;

    let bound_addr = client_sock
        .local_addr()
        .map_err(|e| format!("Failed to get local UDP proxy address: {e}"))?;
    let bound_port = bound_addr.port();

    // Bind upstream server-facing socket to 0.0.0.0 (routable out to internet)
    let upstream_sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("Failed to bind upstream UDP proxy socket: {e}"))?;

    state.local_port.store(bound_port, Ordering::SeqCst);
    state.running.store(true, Ordering::SeqCst);

    let client_sock = Arc::new(client_sock);
    let upstream_sock = Arc::new(upstream_sock);

    crate::applog::event(&format!(
        "udp_proxy: started Rocket League match UDP proxy on 127.0.0.1:{bound_port}"
    ));

    // Spawn bidirectional forwarding tasks
    let state_c2s = state.clone();
    let cs_c2s = client_sock.clone();
    let us_c2s = upstream_sock.clone();

    tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        let mut c2s_pkt_count = 0u64;
        while state_c2s.running.load(Ordering::Relaxed) {
            match cs_c2s.recv_from(&mut buf).await {
                Ok((len, src)) => {
                    // Learn or update client endpoint
                    {
                        let mut client_lock = state_c2s.client_endpoint.write().await;
                        *client_lock = Some(src);
                    }

                    // Forward to real game server (with C2S login bunch rewrite if spoofing name)
                    let target_opt = *state_c2s.target_server.read().await;
                    if let Some(target) = target_opt {
                        let cryptos = state_c2s.active_crypto.read().await.clone();

                        let encrypted = !cryptos.is_empty();
                        let (final_buf, plaintext, was_patched) = if !cryptos.is_empty() {
                            // Datagram proxying pipeline:
                            // 1. Decrypt C2S datagram with AES-256-CBC (whole datagram,
                            //    PKCS#7 padded; no trailing MAC in the current protocol)
                            // 2. Patch plaintext bunch (?Name=...)
                            // 3. Re-encrypt with the same reservation key
                            process_c2s_encrypted_datagram(&buf[..len], &cryptos)
                        } else {
                            // Unencrypted fallback
                            let mut unenc_buf = buf[..len].to_vec();
                            let mut effective_len = len;
                            let mut patched = false;
                            if let Some(active_cfg) = crate::psynet::load_active_spoof_from_disk() {
                                let spoof_name = if let Some(ref ns) = active_cfg.name_spoof {
                                    if ns.enabled && !ns.display_name.trim().is_empty() {
                                        Some(ns.display_name.trim())
                                    } else {
                                        None
                                    }
                                } else if !active_cfg.custom_name.trim().is_empty() {
                                    Some(active_cfg.custom_name.trim())
                                } else {
                                    None
                                };

                                if let Some(name) = spoof_name {
                                    patched = patch_c2s_login_bunch(&mut unenc_buf, &mut effective_len, name);
                                    unenc_buf.truncate(effective_len);
                                }
                            }
                            (unenc_buf, None, patched)
                        };

                        // Capture the plaintext for the monitor/dump, never the ciphertext
                        // when decryption succeeded.
                        let captured: &[u8] = plaintext.as_deref().unwrap_or(&final_buf);
                        record_udp_capture(
                            "C2S",
                            &format!("{target}"),
                            encrypted,
                            plaintext.is_some(),
                            captured,
                        );

                        c2s_pkt_count += 1;
                        if was_patched || c2s_pkt_count <= 5 || c2s_pkt_count % 120 == 0 {
                            let (summary, decoded_info) = decode_udp_packet_summary(captured, true);
                            crate::proxy::record_network_event(
                                "UDP",
                                "OUTBOUND",
                                &format!("{target}"),
                                if was_patched {
                                    "C2S_PATCHED"
                                } else if plaintext.is_some() {
                                    "C2S_DECRYPTED"
                                } else if encrypted {
                                    "C2S_ENCRYPTED"
                                } else {
                                    "C2S_PACKET"
                                },
                                &summary,
                                &decoded_info,
                                was_patched,
                            );
                        }

                        let _ = us_c2s.send_to(&final_buf, target).await;
                    }
                }
                Err(e) => {
                    crate::applog::event(&format!("udp_proxy: client recv error: {e}"));
                    break;
                }
            }
        }
    });

    let state_s2c = state.clone();
    let cs_s2c = client_sock.clone();
    let us_s2c = upstream_sock.clone();

    tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        let mut s2c_pkt_count = 0u64;
        while state_s2c.running.load(Ordering::Relaxed) {
            match us_s2c.recv_from(&mut buf).await {
                Ok((len, from_server)) => {
                    s2c_pkt_count += 1;

                    let cryptos = state_s2c.active_crypto.read().await.clone();
                    let encrypted = !cryptos.is_empty();
                    let plaintext = decrypt_datagram_payload(&buf[..len], &cryptos);
                    let scanned = plaintext.as_deref().unwrap_or(&buf[..len]);

                    // Capture the decrypted view for the monitor and dump. The original
                    // ciphertext is still what gets forwarded to the client.
                    record_udp_capture(
                        "S2C",
                        &format!("{from_server}"),
                        encrypted,
                        plaintext.is_some(),
                        scanned,
                    );

                    // Never drop a whole datagram to hide a loadout reset: UE3 reliable
                    // bunches share the packet stream, so dropping one can stall spawn
                    // and leave the client stuck in spectator.
                    let is_loadout_reset = payload_contains_loadout_reset(scanned);
                    if is_loadout_reset {
                        crate::applog::event("udp_proxy: server loadout reset observed (forwarded)");
                        crate::proxy::record_network_event(
                            "UDP",
                            "INBOUND",
                            &format!("{from_server}"),
                            "LOADOUT_RESET_SEEN",
                            "Server loadout reset observed and forwarded",
                            &format_payload_hexdump(scanned, 2048),
                            false,
                        );
                    } else if s2c_pkt_count <= 5 || s2c_pkt_count % 120 == 0 {
                        let (summary, decoded_info) = decode_udp_packet_summary(scanned, false);
                        crate::proxy::record_network_event(
                            "UDP",
                            "INBOUND",
                            &format!("{from_server}"),
                            if plaintext.is_some() {
                                "S2C_DECRYPTED"
                            } else if encrypted {
                                "S2C_ENCRYPTED"
                            } else {
                                "S2C_PACKET"
                            },
                            &summary,
                            &decoded_info,
                            false,
                        );
                    }

                    // Forward to client endpoint
                    let client_opt = *state_s2c.client_endpoint.read().await;
                    if let Some(client) = client_opt {
                        let _ = cs_s2c.send_to(&buf[..len], client).await;
                    }
                }
                Err(e) => {
                    crate::applog::event(&format!("udp_proxy: upstream recv error: {e}"));
                    break;
                }
            }
        }
    });

    Ok(bound_port)
}

/// Set active match reservation target address.
/// Parses IPv4/IPv6 address or resolves hostname.
pub async fn set_active_reservation(target_addr_str: &str) -> Result<u16, String> {
    set_active_reservation_with_crypto(target_addr_str, None).await
}

/// Set active match reservation target address with modern AES-256-CBC and HMAC-SHA256 session keys.
pub async fn set_active_reservation_with_crypto(
    target_addr_str: &str,
    crypto: Option<ReservationCrypto>,
) -> Result<u16, String> {
    let proxy_port = ensure_udp_proxy().await?;
    let state = PROXY_STATE.get();

    let clean = target_addr_str
        .trim()
        .trim_start_matches("udp://")
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let host_port = clean
        .split('?')
        .next()
        .unwrap_or(clean)
        .split('/')
        .next()
        .unwrap_or(clean)
        .trim();

    // Resolve address
    let resolved: SocketAddr = match host_port.parse() {
        Ok(sa) => sa,
        Err(_) => {
            // Try resolving via tokio
            let addrs = tokio::net::lookup_host(host_port)
                .await
                .map_err(|e| format!("Failed to resolve target server '{host_port}': {e}"))?;
            let mut found = None;
            for addr in addrs {
                found = Some(addr);
                break;
            }
            found.ok_or_else(|| format!("Could not resolve host: {host_port}"))?
        }
    };

    {
        let mut target_lock = state.target_server.write().await;
        *target_lock = Some(resolved);
    }
    {
        let mut client_lock = state.client_endpoint.write().await;
        *client_lock = None;
    }
    {
        let mut crypto_lock = state.active_crypto.write().await;
        match crypto.clone() {
            // A reservation without keys means the match stream is plaintext.
            None => crypto_lock.clear(),
            Some(crypto) => {
                crypto_lock.retain(|existing| {
                    existing.aes_key != crypto.aes_key || existing.aes_iv != crypto.aes_iv
                });
                crypto_lock.insert(0, crypto);
                crypto_lock.truncate(MAX_CRYPTO_CANDIDATES);
            }
        }
    }

    let crypto_mode = {
        let crypto_lock = state.active_crypto.read().await;
        if crypto_lock.is_empty() {
            "Plaintext Passthrough".to_string()
        } else {
            format!(
                "AES-256-CBC ({} candidate key{})",
                crypto_lock.len(),
                if crypto_lock.len() == 1 { "" } else { "s" }
            )
        }
    };
    crate::proxy::record_network_event(
        "UDP",
        "LOOPBACK",
        &format!("{resolved}"),
        "RESERVATION",
        &format!("Active Match Server Rerouted -> 127.0.0.1:{proxy_port}"),
        &format!(
            "Target Upstream Match Server: {resolved}\nLocal Bound UDP Port: {proxy_port}\nCrypto Mode: {crypto_mode}"
        ),
        true,
    );

    Ok(proxy_port)
}

/// UDP packet decrypt & rewrite routine:
/// 1. Decrypt the AES-256-CBC datagram (whole datagram, PKCS#7 padded) using
///    whichever reservation key actually decrypts it
/// 2. Patch `?Name=` UE login bunch if spoofing
/// 3. Re-encrypt with the same key
pub fn process_c2s_encrypted_datagram(
    packet: &[u8],
    cryptos: &[ReservationCrypto],
) -> (Vec<u8>, Option<Vec<u8>>, bool) {
    // 1. Decrypt datagram with AES-256-CBC
    let (mut payload, crypto) = match decrypt_datagram_with_candidates(packet, cryptos) {
        Some(found) => found,
        // Decrypt failed or padding invalid; passthrough original packet
        None => return (packet.to_vec(), None, false),
    };

    // 2. Patch plaintext bunch if name spoofing is active
    let mut effective_len = payload.len();
    let mut was_patched = false;

    if let Some(active_cfg) = crate::psynet::load_active_spoof_from_disk() {
        let spoof_name = if let Some(ref ns) = active_cfg.name_spoof {
            if ns.enabled && !ns.display_name.trim().is_empty() {
                Some(ns.display_name.trim())
            } else {
                None
            }
        } else if !active_cfg.custom_name.trim().is_empty() {
            Some(active_cfg.custom_name.trim())
        } else {
            None
        };

        if let Some(name) = spoof_name {
            was_patched = patch_c2s_login_bunch(&mut payload, &mut effective_len, name);
            payload.truncate(effective_len);
        }
    }

    if !was_patched {
        // If not patched, pass the original packet through but keep the decrypted
        // plaintext so the monitor can display it.
        return (packet.to_vec(), Some(payload), false);
    }

    // 3. Re-encrypt with the reservation key that decrypted this packet.
    match encrypt_datagram_payload(&payload, crypto) {
        (encrypted, true) => (encrypted, Some(payload), true),
        _ => (packet.to_vec(), Some(payload), false),
    }
}

/// Rewrite the `?Name=` value in a C2S login bunch, without changing its size.
///
/// The old implementation changed the length of the connection URL FString and
/// then tried to find the 12-bit `NumBunchBits` field to adjust it. That search
/// compares against `highest_bit` of the *whole datagram*, but a datagram can
/// hold several bunches — on the real capture it found no candidate at all and
/// fell back to blindly rewriting bits 0..11 of bytes 4..7, corrupting the login
/// bunch. The server then never spawned the player.
///
/// Keeping the byte count identical means the FString length and every bunch
/// header field stay valid. A longer spoof name is truncated to fit the slot;
/// a shorter one is space padded.
pub fn patch_c2s_login_bunch(buf: &mut [u8], len: &mut usize, target_name: &str) -> bool {
    let old_len = *len;
    if old_len < 16 || target_name.is_empty() {
        return false;
    }

    let needle = b"?Name=";
    // The login URL is bit-packed into the bunch; try every alignment. Shift 3
    // is the one observed in the real capture.
    for &shift in &[3usize, 0, 1, 2, 4, 5, 6, 7] {
        let mut unshifted = vec![0u8; old_len];
        if shift == 0 {
            unshifted.copy_from_slice(&buf[..old_len]);
        } else {
            for i in 0..old_len {
                let cur = buf[i] >> shift;
                let nxt = if i + 1 < old_len {
                    buf[i + 1] << (8 - shift)
                } else {
                    0
                };
                unshifted[i] = cur | nxt;
            }
        }

        let Some(pos) = unshifted.windows(needle.len()).position(|w| w == needle) else {
            continue;
        };
        let val_start = pos + needle.len();
        if val_start >= old_len {
            continue;
        }
        let mut val_end = val_start;
        while val_end < old_len
            && unshifted[val_end] >= 0x20
            && unshifted[val_end] != b'?'
            && unshifted[val_end] != b'&'
            && unshifted[val_end] != b'/'
        {
            val_end += 1;
        }
        let slot_len = val_end - val_start;
        if slot_len == 0 || slot_len > 64 {
            continue;
        }

        // Fit the spoofed name into exactly the same number of bytes so no
        // length field anywhere in the packet has to change.
        let mut replacement = target_name.as_bytes().to_vec();
        if replacement.len() > slot_len {
            replacement.truncate(slot_len);
        } else {
            replacement.resize(slot_len, b' ');
        }
        unshifted[val_start..val_end].copy_from_slice(&replacement);

        if shift == 0 {
            buf[..old_len].copy_from_slice(&unshifted);
        } else {
            let mut shifted = vec![0u8; old_len];
            for i in 0..old_len {
                shifted[i] |= (unshifted[i] << shift) & 0xFF;
                if i + 1 < old_len {
                    shifted[i + 1] |= unshifted[i] >> (8 - shift);
                }
            }
            // The low `shift` bits of byte 0 are not part of the unshifted view.
            let mask = (1u8 << shift) - 1;
            shifted[0] = (buf[0] & mask) | (shifted[0] & !mask);
            buf[..old_len].copy_from_slice(&shifted);
        }

        // `*len` is deliberately unchanged.
        crate::applog::event(&format!(
            "udp_proxy: patched C2S login name in place to '{}' (shift={}, slot={} bytes)",
            String::from_utf8_lossy(&replacement),
            shift,
            slot_len
        ));
        return true;
    }

    false
}

/// Try each known reservation key against a datagram. The first key whose
/// PKCS#7 padding validates wins; this tolerates PsyNet issuing a fresh (unused)
/// reservation key for the same match.
fn decrypt_datagram_with_candidates<'a>(
    packet: &[u8],
    cryptos: &'a [ReservationCrypto],
) -> Option<(Vec<u8>, &'a ReservationCrypto)> {
    if cryptos.is_empty() || packet.len() < 16 || packet.len() % 16 != 0 {
        return None;
    }
    for crypto in cryptos {
        let decryptor = match Aes256CbcDec::new_from_slices(&crypto.aes_key, &crypto.aes_iv) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let mut dec_buf = packet.to_vec();
        if let Ok(pt) = decryptor.decrypt_padded_mut::<Pkcs7>(&mut dec_buf) {
            return Some((pt.to_vec(), crypto));
        }
    }
    None
}

/// Decrypt an encrypted datagram and return the plaintext bunch payload.
pub fn decrypt_datagram_payload(packet: &[u8], cryptos: &[ReservationCrypto]) -> Option<Vec<u8>> {
    decrypt_datagram_with_candidates(packet, cryptos).map(|(plaintext, _)| plaintext)
}

/// Encrypt a plaintext payload into a match datagram: AES-256-CBC with PKCS#7
/// padding over the whole datagram. The current Rocket League protocol does not
/// append a MAC, so there is no appearing/disappearing 32-byte trailer.
pub fn encrypt_datagram_payload(payload: &[u8], crypto: &ReservationCrypto) -> (Vec<u8>, bool) {
    let encryptor = match Aes256CbcEnc::new_from_slices(&crypto.aes_key, &crypto.aes_iv) {
        Ok(e) => e,
        Err(_) => return (Vec::new(), false),
    };

    let data_len = payload.len();
    let mut enc_buf = vec![0u8; data_len + 16];
    enc_buf[..data_len].copy_from_slice(payload);

    match encryptor.encrypt_padded_mut::<Pkcs7>(&mut enc_buf, data_len) {
        Ok(ct) => (ct.to_vec(), true),
        Err(_) => (Vec::new(), false),
    }
}

/// Format bytes as a clean, aligned Wireshark/Hex-view table (hex bytes on left, ASCII on right)
pub fn format_payload_hexdump(data: &[u8], max_bytes: usize) -> String {
    let slice = if data.len() > max_bytes {
        &data[..max_bytes]
    } else {
        data
    };
    let mut out = String::new();
    for (i, chunk) in slice.chunks(16).enumerate() {
        let offset = i * 16;
        let mut hex_part = String::new();
        let mut ascii_part = String::new();
        for (j, &b) in chunk.iter().enumerate() {
            hex_part.push_str(&format!("{b:02x} "));
            if j == 7 {
                hex_part.push(' ');
            }
            if b >= 0x20 && b <= 0x7e {
                ascii_part.push(b as char);
            } else {
                ascii_part.push('.');
            }
        }
        let padding = 16 - chunk.len();
        if padding > 0 {
            for j in 0..padding {
                hex_part.push_str("   ");
                if chunk.len() + j == 7 {
                    hex_part.push(' ');
                }
            }
        }
        out.push_str(&format!("{offset:04x}   {hex_part:<49}  |{ascii_part}|\n"));
    }
    if data.len() > max_bytes {
        out.push_str(&format!("... [{} more bytes truncated]\n", data.len() - max_bytes));
    }
    out
}

/// Decode an Unreal Engine 3 UDP datagram into human-readable details and structure.
pub fn decode_udp_packet_summary(buf: &[u8], is_c2s: bool) -> (String, String) {
    if buf.is_empty() {
        return ("Empty UDP Datagram".to_string(), "0 bytes".to_string());
    }

    let len = buf.len();
    // In UE3, packets typically contain sequence numbers and bit-packed bunches.
    // Scan unshifted and shifted needle matches for known Control channel strings:
    let needles: &[(&[u8], &str)] = &[
        (b"?Name=", "NMT_Login (Player Login Bunch)"),
        (b"?Reservation=", "NMT_Login (Reservation Parameter)"),
        (b"NMT_Hello", "NMT_Hello (Handshake Init)"),
        (b"NMT_Challenge", "NMT_Challenge (Handshake Nonce)"),
        (b"NMT_Welcome", "NMT_Welcome (Map Spawn Welcome)"),
        (b"NMT_Netspeed", "NMT_Netspeed (Bandwidth Rate)"),
        (b"NMT_Failure", "NMT_Failure (Connection Error)"),
    ];

    let mut detected_type = if is_c2s { "C2S Bunch Stream (Input / Cmds)" } else { "S2C Bunch Stream (State / Physics)" };
    let mut details = format!("Direction: {}\nLength: {} bytes\n", if is_c2s { "Client -> Game Server" } else { "Game Server -> Client" }, len);

    // Scan for needles across bit-shifts [0..8]
    for &(needle, desc) in needles {
        let nlen = needle.len();
        for shift in 0..8 {
            let mut found = false;
            if shift == 0 {
                if buf.windows(nlen).any(|w| w == needle) {
                    found = true;
                }
            } else if len > 1 {
                for i in 0..len - 1 {
                    let mut matched = true;
                    for (b_idx, &nb) in needle.iter().enumerate() {
                        let c_idx = i + b_idx;
                        if c_idx + 1 >= len {
                            matched = false;
                            break;
                        }
                        let unshifted_byte = (buf[c_idx] >> shift) | (buf[c_idx + 1] << (8 - shift));
                        if unshifted_byte != nb {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        found = true;
                        break;
                    }
                }
            }
            if found {
                detected_type = desc;
                details.push_str(&format!("Matched Protocol Message: {desc} (shift={shift})\n"));
                break;
            }
        }
    }

    // Full aligned hex dump of the payload (up to 2048 bytes)
    details.push_str("\nPayload Hexdump:\n");
    details.push_str(&format_payload_hexdump(buf, 2048));

    let summary = format!("{detected_type} ({len} bytes)");
    (summary, details)
}

/// Check whether a decrypted match payload carries a server-side loadout reset.
/// Unreal bunches are bit-packed, so scan every byte alignment.
fn payload_contains_loadout_reset(payload: &[u8]) -> bool {
    const MARKERS: &[&[u8]] = &[
        b"ServerSetLoadout",
        b"ClientSetLoadout",
        b"SetCustomLoadout",
        b"SetLoadout",
    ];

    for &needle in MARKERS {
        if payload.len() < needle.len() {
            continue;
        }
        for shift in 0..8 {
            if shift == 0 {
                if payload.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
                continue;
            }
            for i in 0..(payload.len() - 1) {
                let mut matched = true;
                for (b_idx, &nb) in needle.iter().enumerate() {
                    let c_idx = i + b_idx;
                    if c_idx + 1 >= payload.len() {
                        matched = false;
                        break;
                    }
                    let byte = (payload[c_idx] >> shift) | (payload[c_idx + 1] << (8 - shift));
                    if byte != nb {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    return true;
                }
            }
        }
    }
    false
}

/// Get current bound local UDP proxy port if running.
pub fn get_local_port() -> Option<u16> {
    let state = PROXY_STATE.get();
    let p = state.local_port.load(Ordering::SeqCst);
    if p > 0 && state.running.load(Ordering::SeqCst) {
        Some(p)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};

    type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
    type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

    fn crypto_with(key: u8, iv: u8) -> ReservationCrypto {
        ReservationCrypto {
            aes_key: [key; 32],
            aes_iv: [iv; 16],
            hmac_key: [0x55; 32],
        }
    }

    fn encrypt_raw(raw: &[u8], crypto: &ReservationCrypto) -> Vec<u8> {
        let enc = Aes256CbcEnc::new_from_slices(&crypto.aes_key, &crypto.aes_iv).unwrap();
        let mut buf = vec![0u8; raw.len() + 16];
        buf[..raw.len()].copy_from_slice(raw);
        enc.encrypt_padded_mut::<Pkcs7>(&mut buf, raw.len())
            .unwrap()
            .to_vec()
    }

    #[test]
    fn test_process_c2s_encrypted_datagram_decryption() {
        let crypto = crypto_with(0x42, 0x24);

        // Construct mock UE3 connection payload
        let raw_payload = b"\x01\x00\x00\x00\x15\x00\x00\x00:?Name=OriginalPlayer?Team=0\x00";
        let ciphertext = encrypt_raw(raw_payload, &crypto);

        // Process through proxy decryption
        let (processed, plaintext, patched) =
            process_c2s_encrypted_datagram(&ciphertext, &[crypto.clone()]);

        // Since no spoofing rule is active on disk in test, was_patched = false and packet passes through
        assert!(!patched, "Without spoof config, packet passes unmodified");
        assert_eq!(processed, ciphertext, "Passthrough maintains byte parity");
        assert_eq!(
            plaintext.as_deref(),
            Some(&raw_payload[..]),
            "Decrypted plaintext must be available even on passthrough"
        );

        // Manual decryption should recover the exact payload (no MAC trailer).
        let dec = Aes256CbcDec::new_from_slices(&crypto.aes_key, &crypto.aes_iv).unwrap();
        let mut dec_buf = ciphertext.clone();
        let decrypted = dec.decrypt_padded_mut::<Pkcs7>(&mut dec_buf).unwrap();
        assert_eq!(decrypted, raw_payload, "Whole datagram decrypts to the payload");
    }

    #[test]
    fn test_encrypt_datagram_payload_roundtrip() {
        let crypto = crypto_with(0x31, 0x17);

        let raw_payload = b"\x01\x00\x00\x00\x08\x00\x00\x00R.Name.v1\x00";
        let (ciphertext, ok) = encrypt_datagram_payload(raw_payload, &crypto);
        assert!(ok, "encryption must succeed");
        assert_eq!(ciphertext.len() % 16, 0, "ciphertext must be a whole number of AES blocks");
        assert_ne!(&ciphertext[..], raw_payload, "ciphertext must not equal plaintext");

        let decrypted = decrypt_datagram_payload(&ciphertext, &[crypto.clone()])
            .expect("decrypt_datagram_payload must recover the plaintext");
        assert_eq!(decrypted, raw_payload, "round-trip must preserve payload byte-for-byte");
    }

    #[test]
    fn test_decrypt_tries_all_candidate_keys() {
        let stale = crypto_with(0x11, 0x22);
        let live = crypto_with(0x42, 0x24);

        let raw_payload = b"NMT_Welcome\x00?Name=Player";
        let ciphertext = encrypt_raw(raw_payload, &live);

        // Newest reservation key is wrong; the older one still decrypts the stream.
        let decrypted = decrypt_datagram_payload(&ciphertext, &[stale, live])
            .expect("must fall back to the key that actually decrypts");
        assert_eq!(decrypted, raw_payload);
    }

    #[test]
    fn test_payload_contains_loadout_reset_at_multiple_alignments() {
        let plain = b"prefix\x00ServerSetLoadout\x00suffix";
        assert!(payload_contains_loadout_reset(plain));

        // Bit-pack the marker at shift 3 to mimic UE bunch alignment.
        let marker = b"ClientSetLoadout";
        let mut packed = vec![0u8; marker.len() + 2];
        for (i, &byte) in marker.iter().enumerate() {
            packed[i] |= byte << 3;
            packed[i + 1] |= byte >> 5;
        }
        assert!(payload_contains_loadout_reset(&packed));

        assert!(!payload_contains_loadout_reset(b"NMT_Welcome\x00?Name=Player"));
    }

    #[test]
    fn test_login_name_patch_preserves_packet_structure() {
        // Real 132-byte C2S login bunch from the capture. It contains
        // `?Name=2` at bit shift 3. The patch must not change the length or any
        // header byte, otherwise the server rejects the bunch and never spawns.
        let wire = "2d4z5m6EHGI502kHzIKzIIDT9P0MVC9eUsj0skMEyWN3n7fjvw0KCP47/pwxWI5KyqYGKGtrLm4dNP8D3oTvhCUtPMqrniof6qdwOabGBst5SCJpo+AkO/bMb5g74PkLgxcqpjFw9EmQBJE2qKtqkqY/DaWzt3aHlnCYvDT2Ay5Nxg0GqqFZf2Hs+x8kk+YT";
        let key = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "8B93VmMkuo9J+TWTFdZ/KK380tckolQRa8WVdRu6RGM=",
        )
        .unwrap();
        let iv = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "fnG/YrVIlkaWskzXBWdPPg==",
        )
        .unwrap();
        let crypto = ReservationCrypto {
            aes_key: key.try_into().unwrap(),
            aes_iv: iv.try_into().unwrap(),
            hmac_key: [0; 32],
        };
        let ciphertext = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            wire,
        )
        .unwrap();
        let mut plaintext = decrypt_datagram_payload(&ciphertext, &[crypto])
            .expect("login bunch must decrypt");
        let original_header = plaintext[..8].to_vec();
        let original_len = plaintext.len();

        let mut len = original_len;
        assert!(patch_c2s_login_bunch(&mut plaintext, &mut len, "bits"));
        assert_eq!(len, original_len, "name patch must be length preserving");
        assert_eq!(
            plaintext[..8], original_header,
            "name patch must not touch the packet/bunch header"
        );
    }

    #[test]
    fn test_decrypt_real_capture_with_reservation_key() {
        // Key/IV from the first ReservationsReadyMessage in the real capture,
        // plus one C2S and one S2C datagram from udp-packets.json. This locks in
        // the whole-datagram AES-256-CBC + PKCS#7 framing used by the live match.
        let key = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "8B93VmMkuo9J+TWTFdZ/KK380tckolQRa8WVdRu6RGM=",
        )
        .unwrap();
        let iv = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "fnG/YrVIlkaWskzXBWdPPg==",
        )
        .unwrap();
        let crypto = ReservationCrypto {
            aes_key: key.try_into().unwrap(),
            aes_iv: iv.try_into().unwrap(),
            hmac_key: [0; 32],
        };

        for wire in [
            "UC0OdAhU8lqxpEi0D1Z4X/ZV40Xk3Jf96LXg9ygTMolT/2tZQOzn6E/iGM+vtv/o6XTPPPBj/Iw4f4tstmiNc9nmrd6pWAcnvLtEQelNMog=",
            "r4H68tTWrTDAnmD15CHYzVyFDBTzVu/unJ5rwIbHA+Qa+kgVCmv+RoBuIxHPKSHe",
        ] {
            let ciphertext = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                wire,
            )
            .unwrap();
            let plaintext = decrypt_datagram_payload(&ciphertext, &[crypto.clone()])
                .expect("real captured datagram must decrypt with the reservation key");
            assert!(!plaintext.is_empty());
        }
    }

    #[test]
    fn test_process_c2s_encrypted_datagram_rejects_corrupted_padding() {
        let crypto = crypto_with(0x42, 0x24);

        let garbage_ciphertext = vec![0xAAu8; 48];
        let (processed, plaintext, patched) =
            process_c2s_encrypted_datagram(&garbage_ciphertext, &[crypto]);
        assert!(!patched);
        assert_eq!(processed, garbage_ciphertext, "Corrupted/unmatched ciphertexts must fail-safe passthrough");
        assert!(plaintext.is_none(), "Undecryptable packets must not expose fake plaintext");
    }
}
