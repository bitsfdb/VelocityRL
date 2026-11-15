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

/// Shared state for the UDP Match Proxy.
struct ProxyState {
    /// Local UDP port Rocket League connects to (e.g. 127.0.0.1:local_port).
    local_port: AtomicU16,
    /// Real game server address parsed from AddReservationMessage.
    target_server: RwLock<Option<SocketAddr>>,
    /// Learned endpoint of the local Rocket League client.
    client_endpoint: RwLock<Option<SocketAddr>>,
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
                        let mut effective_len = len;
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
                                patch_c2s_login_bunch(&mut buf, &mut effective_len, name);
                            }
                        }
                        let _ = us_c2s.send_to(&buf[..effective_len], target).await;
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
        while state_s2c.running.load(Ordering::Relaxed) {
            match us_s2c.recv_from(&mut buf).await {
                Ok((len, _from_server)) => {
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

    crate::proxy::record_network_event(
        "UDP",
        "LOOPBACK",
        &format!("{resolved}"),
        "RESERVATION",
        &format!("Active Match Server Rerouted -> 127.0.0.1:{proxy_port}"),
        &format!("Target Upstream Match Server: {resolved}\nLocal Bound UDP Port: {proxy_port}"),
        true,
    );

    Ok(proxy_port)
}

/// Rewrite C2S Login bunch packet (`?Name=...`) if a spoofed name is active.
/// Implements bit-aligned UE bunch patching matching decompiled `rewrite_datagram_payload`.
pub fn patch_c2s_login_bunch(buf: &mut [u8], len: &mut usize, target_name: &str) -> bool {
    let old_len = *len;
    if old_len < 16 || target_name.is_empty() {
        return false;
    }

    let needle = b"?Name=";
    let shifts = [3usize, 0, 1, 2, 4, 5, 6, 7];

    for &shift in &shifts {
        let orig_buf = buf[..old_len].to_vec();
        // Bit-unshift buf into a temporary buffer to scan on byte boundaries
        let mut unshifted = vec![0u8; old_len + 1];
        if shift == 0 {
            unshifted[..old_len].copy_from_slice(&buf[..old_len]);
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

        // Search for needle `?Name=`
        if let Some(pos) = unshifted.windows(needle.len()).position(|w| w == needle) {
            let val_start = pos + needle.len();
            if val_start >= old_len {
                continue;
            }

            // Find terminator of old name ('?' or null/control/non-printable)
            let mut val_end = val_start;
            while val_end < old_len && unshifted[val_end] >= 0x20 && unshifted[val_end] != b'?' && unshifted[val_end] != b'&' && unshifted[val_end] != b'/' {
                val_end += 1;
            }

            let old_name_len = val_end - val_start;
            let new_name_bytes = target_name.as_bytes();
            let new_name_len = new_name_bytes.len();
            let byte_delta = new_name_len as isize - old_name_len as isize;

            // In Unreal Engine / Rocket League connection options:
            // The FString starts before the connection string options (e.g. at ':?Name=...')
            // Scan backwards from `?Name=` to find ':' (0x3A), which precedes options
            let mut colon_pos = pos;
            while colon_pos > 0 && unshifted[colon_pos] != b':' {
                colon_pos -= 1;
            }
            if unshifted[colon_pos] != b':' {
                colon_pos = pos;
            }

            // The 4-byte string length is located immediately preceding colon_pos (colon_pos - 4)
            let prefix_start = if colon_pos >= 4 { colon_pos - 4 } else { 0 };

            let mut patched_unshifted = Vec::with_capacity(old_len + new_name_len + 16);
            // 1. Copy prefix up to string length field
            patched_unshifted.extend_from_slice(&unshifted[..prefix_start]);

            // 2. Compute and append updated string length
            if colon_pos >= 4 {
                let orig_str_len = u32::from_le_bytes([
                    unshifted[prefix_start],
                    unshifted[prefix_start + 1],
                    unshifted[prefix_start + 2],
                    unshifted[prefix_start + 3],
                ]);
                let updated_str_len = ((orig_str_len as isize + byte_delta).max(0)) as u32;
                patched_unshifted.extend_from_slice(&updated_str_len.to_le_bytes());
            } else {
                patched_unshifted.extend_from_slice(&unshifted[prefix_start..colon_pos]);
            }

            // 3. Copy from colon_pos up to start of old name value (?Name=)
            patched_unshifted.extend_from_slice(&unshifted[colon_pos..val_start]);

            // 4. Append new name bytes
            patched_unshifted.extend_from_slice(new_name_bytes);

            // 5. Append remainder of packet
            patched_unshifted.extend_from_slice(&unshifted[val_end..old_len]);

            let patched_len = patched_unshifted.len();
            if patched_len > buf.len() {
                return false;
            }

            // Re-shift into temporary buffer
            let mut shifted = vec![0u8; patched_len + 4];
            if shift == 0 {
                shifted[..patched_len].copy_from_slice(&patched_unshifted[..patched_len]);
            } else {
                for i in 0..patched_len {
                    shifted[i] |= patched_unshifted[i] << shift;
                    shifted[i + 1] |= patched_unshifted[i] >> (8 - shift);
                }
                // Preserve lower bits of byte 0 from the original buffer (rewrite_datagram_payload parity)
                let mask = (1u8 << shift) - 1;
                shifted[0] = (orig_buf[0] & mask) | (shifted[0] & !mask);
            }

            let new_total_len = if shift == 0 {
                patched_len
            } else {
                (patched_len * 8 + shift + 7) / 8
            };

            if new_total_len > buf.len() {
                return false;
            }

            // Find highest set bit in the original packet (rewrite_datagram_payload parity)
            let mut highest_bit: isize = -1;
            for byte_idx in (0..old_len).rev() {
                let b = orig_buf[byte_idx];
                if b != 0 {
                    for bit_pos in (0..8).rev() {
                        if (b & (1 << bit_pos)) != 0 {
                            highest_bit = (byte_idx * 8 + bit_pos) as isize;
                            break;
                        }
                    }
                    if highest_bit >= 0 {
                        break;
                    }
                }
            }

            // Dynamically locate 12-bit NumBunchBits field
            let mut nbb_bit_offset: isize = -1;
            if highest_bit > 12 {
                let target_pos = (shift + 92) as isize;
                let max_scan = highest_bit - 12;
                for pos_idx in 0..=max_scan {
                    let mut nbb_val = 0u32;
                    for b in 0..12 {
                        let bit_coord = (pos_idx + b) as usize;
                        let bit = (orig_buf[bit_coord >> 3] >> (bit_coord & 7)) & 1;
                        nbb_val |= (bit as u32) << b;
                    }
                    if (nbb_val as isize) + pos_idx + 12 == highest_bit {
                        if nbb_bit_offset < 0 || (pos_idx - target_pos).abs() < (nbb_bit_offset - target_pos).abs() {
                            nbb_bit_offset = pos_idx;
                        }
                    }
                }
            }

            // If found, rewrite 12-bit NumBunchBits at its exact bit offset in the shifted buffer
            if nbb_bit_offset >= 0 && byte_delta != 0 {
                let mut old_nbb = 0u32;
                for b in 0..12 {
                    let bit_coord = (nbb_bit_offset + b) as usize;
                    let bit = (orig_buf[bit_coord >> 3] >> (bit_coord & 7)) & 1;
                    old_nbb |= (bit as u32) << b;
                }
                let bit_delta = byte_delta * 8;
                let updated_nbb = ((old_nbb as isize + bit_delta).max(0) as u32) & 0x0FFF;
                for b in 0..12 {
                    let bit_coord = (nbb_bit_offset + b) as usize;
                    let byte_pos = bit_coord >> 3;
                    let bit_pos = bit_coord & 7;
                    if byte_pos < shifted.len() {
                        if ((updated_nbb >> b) & 1) != 0 {
                            shifted[byte_pos] |= 1 << bit_pos;
                        } else {
                            shifted[byte_pos] &= !(1 << bit_pos);
                        }
                    }
                }
            } else if byte_delta != 0 && new_total_len >= 8 {
                // Fallback to standard header offset
                let bit_delta = byte_delta * 8;
                let raw_u32 = u32::from_le_bytes([shifted[4], shifted[5], shifted[6], shifted[7]]);
                let nbb = raw_u32 & 0x0FFF;
                let updated_nbb = ((nbb as isize + bit_delta).max(0) as u32) & 0x0FFF;
                let new_u32 = (raw_u32 & !0x0FFF) | updated_nbb;
                shifted[4..8].copy_from_slice(&new_u32.to_le_bytes());
            }

            buf[..new_total_len].copy_from_slice(&shifted[..new_total_len]);
            *len = new_total_len;

            crate::applog::event(&format!(
                "udp_proxy: patched C2S login bunch name to '{target_name}' (shift={shift}, nbb_offset={nbb_bit_offset}, len: {old_len} -> {len})"
            ));
            return true;
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
