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

                    // Forward to real game server
                    let target_opt = *state_c2s.target_server.read().await;
                    if let Some(target) = target_opt {
                        let packet_data = buf[..len].to_vec();
                        let us_clone = us_c2s.clone();
                        tokio::spawn(async move {
                            if let Some(target_ms) = sample_spoofed_ping_ms() {
                                let delay_ms = (target_ms / 2).max(1) as u64;
                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            }
                            if let Err(e) = us_clone.send_to(&packet_data, target).await {
                                crate::applog::event(&format!(
                                    "udp_proxy: send to game server {target} failed: {e}"
                                ));
                            }
                        });
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
                        let packet_data = buf[..len].to_vec();
                        let cs_clone = cs_s2c.clone();
                        tokio::spawn(async move {
                            if let Some(target_ms) = sample_spoofed_ping_ms() {
                                let delay_ms = (target_ms / 2).max(1) as u64;
                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            }
                            if let Err(e) = cs_clone.send_to(&packet_data, client).await {
                                crate::applog::event(&format!(
                                    "udp_proxy: forward to client {client} failed: {e}"
                                ));
                            }
                        });
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

    // Resolve address
    let resolved: SocketAddr = match target_addr_str.parse() {
        Ok(sa) => sa,
        Err(_) => {
            // Try resolving via tokio
            let addrs = tokio::net::lookup_host(target_addr_str)
                .await
                .map_err(|e| format!("Failed to resolve target server '{target_addr_str}': {e}"))?;
            let mut found = None;
            for addr in addrs {
                found = Some(addr);
                break;
            }
            found.ok_or_else(|| format!("Could not resolve host: {target_addr_str}"))?
        }
    };

    crate::applog::event(&format!(
        "udp_proxy: active reservation set -> real server {resolved} (proxied on 127.0.0.1:{proxy_port})"
    ));

    {
        let mut target_lock = state.target_server.write().await;
        *target_lock = Some(resolved);
    }
    {
        let mut client_lock = state.client_endpoint.write().await;
        *client_lock = None;
    }

    Ok(proxy_port)
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
