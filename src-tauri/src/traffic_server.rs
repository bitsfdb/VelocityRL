/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::TcpListener;

pub const TRAFFIC_SERVER_PORT: u16 = 1289;

fn full_body(bytes: impl Into<Bytes>) -> BoxBody<Bytes, Infallible> {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed()
}

pub fn start_traffic_web_server() {
    tauri::async_runtime::spawn(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], TRAFFIC_SERVER_PORT));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => {
                crate::applog::event(&format!("traffic_server: listening on http://127.0.0.1:{TRAFFIC_SERVER_PORT}"));
                l
            }
            Err(e) => {
                crate::applog::event(&format!("traffic_server: failed to bind 127.0.0.1:{TRAFFIC_SERVER_PORT}: {e}"));
                return;
            }
        };

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            tauri::async_runtime::spawn(async move {
                let service = service_fn(handle_traffic_http);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
}

async fn handle_traffic_http(req: Request<Incoming>) -> Result<Response<BoxBody<Bytes, Infallible>>, hyper::Error> {
    let path = req.uri().path().to_ascii_lowercase();
    let query = req.uri().query().unwrap_or_default();

    if path == "/" || path == "/index.html" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Cache-Control", "no-cache, no-store, must-revalidate")
            .body(full_body(INDEX_HTML))
            .unwrap());
    }

    if path == "/api/events" {
        let mut since_id: Option<u64> = None;
        let mut limit: Option<usize> = None;
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k.eq_ignore_ascii_case("since") {
                    since_id = v.parse().ok();
                } else if k.eq_ignore_ascii_case("limit") {
                    limit = v.parse().ok();
                }
            }
        }
        let events = crate::applog::get_traffic_events(since_id, limit).unwrap_or_default();
        let json_bytes = serde_json::to_vec(&events).unwrap_or_else(|_| b"[]".to_vec());

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "no-cache, no-store")
            .body(full_body(json_bytes))
            .unwrap());
    }

    if path == "/api/clear" {
        let _ = crate::applog::clear_traffic_events();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(b"{\"ok\":true}".to_vec()))
            .unwrap());
    }

    if path == "/api/toggle" {
        let current = crate::applog::is_traffic_debug();
        let new_state = !current;
        crate::applog::set_traffic_debug(new_state);
        let resp_json = format!("{{\"ok\":true,\"capturing\":{new_state}}}");
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(resp_json.into_bytes()))
            .unwrap());
    }

    if path == "/api/status" {
        let current = crate::applog::is_traffic_debug();
        let resp_json = format!("{{\"capturing\":{current}}}");
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .body(full_body(resp_json.into_bytes()))
            .unwrap());
    }

    if path == "/api/export" {
        let content = if let Some(path) = crate::applog::traffic_debug_path() {
            std::fs::read_to_string(&path).unwrap_or_else(|_| "(empty)".to_string())
        } else {
            "(log not initialised)".to_string()
        };

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Content-Disposition", "attachment; filename=\"traffic_debug.log\"")
            .body(full_body(content.into_bytes()))
            .unwrap());
    }

    Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(full_body(b"Not Found".to_vec()))
        .unwrap())
}

const INDEX_HTML: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>VelocityRL — Network Traffic Inspector</title>
    <style>
        :root {
            --bg: #121212;
            --panel: #1a1a1a;
            --panel-hover: #222222;
            --border: #333333;
            --text: #e8e8e8;
            --text-secondary: #a0a0a0;
            --muted: #888888;
            --accent: #4a9eff;
            --accent-blue: #4a9eff;
            --accent-green: #2ecc71;
            --accent-orange: #f39c12;
            --accent-red: #e74c3c;
            --font-mono: ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace;
        }

        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            background: var(--bg);
            color: var(--text);
            font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
            font-size: 13px;
            height: 100vh;
            display: flex;
            flex-direction: column;
            overflow: hidden;
            user-select: none;
        }

        /* Header */
        header {
            background: var(--panel);
            border-bottom: 1px solid var(--border);
            padding: 8px 16px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 12px;
            flex-shrink: 0;
        }

        .header-left {
            display: flex;
            align-items: center;
            gap: 12px;
        }

        .brand-title {
            font-weight: 600;
            font-size: 14px;
            color: var(--text);
            display: flex;
            align-items: center;
            gap: 8px;
        }

        .header-actions {
            display: flex;
            align-items: center;
            gap: 8px;
        }

        button {
            background: var(--panel);
            border: 1px solid var(--border);
            color: var(--muted);
            border-radius: 4px;
            padding: 5px 12px;
            font-size: 12px;
            font-family: inherit;
            font-weight: 500;
            cursor: pointer;
            display: inline-flex;
            align-items: center;
            gap: 5px;
            transition: all 0.1s ease;
        }

        button:hover {
            color: var(--text);
            background: var(--panel-hover);
            border-color: #444;
        }

        button.active {
            border-color: var(--accent);
            color: var(--text);
            background: #2a2a2a;
        }

        /* Toolbar */
        .toolbar {
            background: var(--panel);
            border-bottom: 1px solid var(--border);
            padding: 6px 16px;
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 12px;
            flex-wrap: wrap;
            flex-shrink: 0;
        }

        .search-box {
            position: relative;
            flex: 1;
            min-width: 240px;
            max-width: 480px;
        }

        .search-box input {
            width: 100%;
            background: var(--bg);
            border: 1px solid var(--border);
            color: var(--text);
            border-radius: 4px;
            padding: 5px 28px 5px 10px;
            font-family: var(--font-mono);
            font-size: 12px;
            outline: none;
        }

        .search-box input:focus {
            border-color: var(--accent);
        }

        .search-clear {
            position: absolute;
            right: 8px;
            top: 50%;
            transform: translateY(-50%);
            cursor: pointer;
            color: var(--muted);
            font-size: 11px;
            display: none;
        }

        .filter-strip {
            display: flex;
            gap: 4px;
            align-items: center;
        }

        .filter-btn {
            background: transparent;
            border: 1px solid transparent;
            color: var(--muted);
            padding: 4px 10px;
            font-size: 12px;
            border-radius: 4px;
        }

        .filter-btn:hover {
            color: var(--text);
            background: var(--panel-hover);
        }

        .filter-btn.active {
            border-color: var(--accent);
            color: var(--text);
            background: #2a2a2a;
            font-weight: 600;
        }

        .toolbar-options {
            display: flex;
            align-items: center;
            gap: 12px;
            font-size: 12px;
            color: var(--muted);
        }

        .toolbar-options label {
            display: flex;
            align-items: center;
            gap: 5px;
            cursor: pointer;
        }

        /* Main View */
        .main-container {
            display: flex;
            flex: 1;
            min-height: 0;
            overflow: hidden;
        }

        /* Left Table */
        .table-pane {
            flex: 1.15;
            min-width: 320px;
            border-right: 1px solid var(--border);
            display: flex;
            flex-direction: column;
            overflow: hidden;
            background: var(--bg);
        }

        .grid-header {
            display: flex;
            background: var(--panel);
            border-bottom: 1px solid var(--border);
            font-size: 11px;
            font-weight: 600;
            color: var(--muted);
            text-transform: uppercase;
            letter-spacing: 0.04em;
            padding: 7px 12px;
            flex-shrink: 0;
        }

        .grid-rows {
            flex: 1;
            overflow-y: auto;
            display: flex;
            flex-direction: column;
        }

        .col-id { width: 45px; flex-shrink: 0; color: var(--muted); font-family: var(--font-mono); }
        .col-time { width: 68px; flex-shrink: 0; color: var(--muted); font-family: var(--font-mono); }
        .col-dir { width: 95px; flex-shrink: 0; font-family: var(--font-mono); font-weight: 500; font-size: 11.5px; }
        .col-type { width: 65px; flex-shrink: 0; font-family: var(--font-mono); color: var(--muted); }
        .col-svc { flex: 1; min-width: 0; font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
        .col-status { width: 75px; flex-shrink: 0; text-align: right; font-family: var(--font-mono); font-size: 11.5px; }
        .col-size { width: 65px; flex-shrink: 0; text-align: right; color: var(--muted); font-family: var(--font-mono); font-size: 11.5px; }

        .traffic-row {
            display: flex;
            align-items: center;
            padding: 5px 12px;
            border-bottom: 1px solid rgba(51, 51, 51, 0.4);
            font-size: 12px;
            cursor: pointer;
        }

        .traffic-row:hover {
            background: var(--panel-hover);
        }

        .traffic-row.is-selected {
            background: #242424 !important;
            outline: 1px solid var(--accent);
        }

        .badge-dir-clt { color: #4a9eff; }
        .badge-dir-srv { color: #a0a0a0; }
        .badge-dir-udp { color: #f39c12; }

        .tag-patched {
            background: rgba(46, 204, 113, 0.15);
            color: #2ecc71;
            border: 1px solid rgba(46, 204, 113, 0.35);
            border-radius: 3px;
            padding: 0 4px;
            font-size: 9.5px;
            margin-left: 6px;
            font-weight: 700;
        }

        /* Right Inspector */
        .inspector-pane {
            flex: 1;
            min-width: 320px;
            display: flex;
            flex-direction: column;
            background: var(--panel);
            overflow: hidden;
        }

        .inspector-empty {
            display: flex;
            align-items: center;
            justify-content: center;
            height: 100%;
            color: var(--muted);
            font-size: 13px;
            text-align: center;
            padding: 20px;
        }

        .inspector-content {
            display: flex;
            flex-direction: column;
            height: 100%;
            min-height: 0;
        }

        .inspector-header {
            padding: 8px 14px;
            border-bottom: 1px solid var(--border);
            display: flex;
            justify-content: space-between;
            align-items: center;
            background: var(--panel);
            flex-shrink: 0;
            gap: 8px;
        }

        .meta-service-title {
            font-family: var(--font-mono);
            font-weight: 600;
            font-size: 13px;
            color: var(--text);
            overflow: hidden;
            text-overflow: ellipsis;
            white-space: nowrap;
        }

        .inspector-toolbar {
            padding: 6px 14px;
            background: #151515;
            border-bottom: 1px solid var(--border);
            display: flex;
            justify-content: space-between;
            align-items: center;
            font-size: 11.5px;
            color: var(--muted);
            flex-shrink: 0;
            font-family: var(--font-mono);
        }

        .inspector-payload-wrap {
            flex: 1;
            min-height: 0;
            overflow: auto;
            background: var(--bg);
            padding: 12px 14px;
        }

        pre.payload-code {
            margin: 0;
            font-family: var(--font-mono);
            font-size: 12px;
            line-height: 1.5;
            color: var(--text);
            white-space: pre-wrap;
            word-break: break-all;
            user-select: text;
        }

        .empty-indicator {
            padding: 40px 20px;
            text-align: center;
            color: var(--muted);
            font-size: 13px;
        }
    </style>
</head>
<body>
    <header>
        <div class="header-left">
            <div class="brand-title">
                <span>VelocityRL Network Inspector</span>
            </div>
            <span id="countText" style="color:var(--muted); font-family:var(--font-mono); font-size:11.5px;">0 packets</span>
        </div>
        <div class="header-actions">
            <button id="btnToggle" type="button">Pause</button>
            <button id="btnClear" type="button">Clear</button>
            <button id="btnExport" type="button">Export Log</button>
        </div>
    </header>

    <div class="toolbar">
        <div class="search-box">
            <input type="text" id="searchInput" placeholder="Filter packets by service, port, IDs, or JSON content..." spellcheck="false" autocomplete="off" />
            <span id="searchClear" class="search-clear">✕</span>
        </div>

        <div class="filter-strip" id="filterStrip">
            <button type="button" class="filter-btn active" data-filter="all">All</button>
            <button type="button" class="filter-btn" data-filter="ws">WebSocket</button>
            <button type="button" class="filter-btn" data-filter="loadout">Loadout &amp; Products</button>
            <button type="button" class="filter-btn" data-filter="skills">Skills &amp; Ranks</button>
            <button type="button" class="filter-btn" data-filter="udp">UDP (7000-9500)</button>
            <button type="button" class="filter-btn" data-filter="http">HTTP Intercept</button>
            <button type="button" class="filter-btn" data-filter="patched">Patched Only</button>
        </div>

        <div class="toolbar-options">
            <label><input type="checkbox" id="chkAutoScroll" checked /> Auto-scroll</label>
        </div>
    </div>

    <div class="main-container">
        <!-- Left Table -->
        <div class="table-pane">
            <div class="grid-header">
                <span class="col-id">#</span>
                <span class="col-time">Time</span>
                <span class="col-dir">Direction</span>
                <span class="col-type">Type</span>
                <span class="col-svc">Service / Endpoint</span>
                <span class="col-status">Status</span>
                <span class="col-size">Size</span>
            </div>
            <div class="grid-rows" id="gridRows">
                <div class="empty-indicator">Waiting for Rocket League network traffic...</div>
            </div>
        </div>

        <!-- Right Inspector -->
        <div class="inspector-pane">
            <div id="inspectorEmpty" class="inspector-empty">
                <p>Select a packet or frame to inspect its payload, headers, and RPC data.</p>
            </div>
            <div id="inspectorContent" class="inspector-content" style="display:none;">
                <div class="inspector-header">
                    <span id="metaService" class="meta-service-title">-</span>
                    <div style="display:flex; gap:6px; align-items:center;">
                        <button id="btnFormatToggle" type="button" style="padding:3px 8px; font-size:11px;">Show Raw</button>
                        <button id="btnCopyPayload" type="button" style="padding:3px 8px; font-size:11px;">Copy Payload</button>
                    </div>
                </div>
                <div class="inspector-toolbar">
                    <div id="metaRpcIds" style="display:flex; gap:12px;">
                        <span id="metaReqId">ReqID: -</span>
                        <span id="metaRespId">RespID: -</span>
                    </div>
                    <div style="display:flex; gap:12px;">
                        <span id="metaTime">Time: -</span>
                        <span id="metaSize">0 B</span>
                    </div>
                </div>
                <div class="inspector-payload-wrap">
                    <pre id="payloadView" class="payload-code"></pre>
                </div>
            </div>
        </div>
    </div>

    <script>
        let events = [];
        let selectedId = null;
        let isCapturing = true;
        let activeFilter = 'all';
        let filterText = '';
        let prettyJson = true;

        function formatBytes(bytes) {
            if (!bytes || bytes === 0) return '0 B';
            if (bytes < 1024) return bytes + ' B';
            if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
            return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
        }

        function escapeHtml(str) {
            return String(str || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
        }

        async function fetchEvents() {
            const lastId = events.length > 0 ? events[events.length - 1].id : 0;
            try {
                const res = await fetch(`/api/events?since=${lastId}&limit=200`);
                if (res.ok) {
                    const newItems = await res.json();
                    if (Array.isArray(newItems) && newItems.length > 0) {
                        for (const item of newItems) {
                            if (!events.some(e => e.id === item.id)) {
                                events.push(item);
                            }
                        }
                        if (events.length > 2500) events = events.slice(-1800);
                        renderGrid();
                    }
                }
            } catch {}
        }

        function matchesFilter(ev) {
            const cat = activeFilter;
            const catUpper = (ev.category || '').toUpperCase();
            const svcLower = (ev.service || '').toLowerCase();

            if (cat === 'ws' && !catUpper.includes('WS')) return false;
            if (cat === 'loadout' && !(svcLower.includes('loadout') || svcLower.includes('product') || svcLower.includes('inventory') || svcLower.includes('authplayer'))) return false;
            if (cat === 'skills' && !(svcLower.includes('skill') || svcLower.includes('rank') || svcLower.includes('leaderboard'))) return false;
            if (cat === 'udp' && !catUpper.includes('UDP')) return false;
            if (cat === 'http' && !(catUpper.includes('HTTP') || catUpper.includes('CONFIG') || catUpper.includes('BROKER'))) return false;
            if (cat === 'patched' && !ev.patched) return false;

            if (filterText) {
                const q = filterText;
                const matches = svcLower.includes(q)
                    || (ev.category || '').toLowerCase().includes(q)
                    || (ev.direction || '').toLowerCase().includes(q)
                    || (ev.req_id || '').toLowerCase().includes(q)
                    || (ev.resp_id || '').toLowerCase().includes(q)
                    || (ev.body || '').toLowerCase().includes(q);
                if (!matches) return false;
            }
            return true;
        }

        function renderGrid() {
            const grid = document.getElementById('gridRows');
            const countText = document.getElementById('countText');
            const filtered = events.filter(matchesFilter);

            countText.textContent = `${filtered.length} / ${events.length} packets`;

            if (filtered.length === 0) {
                grid.innerHTML = `<div class="empty-indicator">${events.length === 0 ? 'Waiting for Rocket League network traffic...' : 'No packets match active filters.'}</div>`;
                return;
            }

            let html = '';
            for (const ev of filtered) {
                const isSelected = ev.id === selectedId;
                const timeStr = ev.timestamp ? (ev.timestamp.split(' ')[1] || ev.timestamp) : '--';
                const isUdp = (ev.category || '').toUpperCase().includes('UDP');
                const isClt = ev.direction.includes('CLIENT->SRV') || ev.direction === 'REQ';
                const isSrv = ev.direction.includes('SRV->CLIENT') || ev.direction === 'RESP';
                const dirClass = isUdp ? 'badge-dir-udp' : (isClt ? 'badge-dir-clt' : (isSrv ? 'badge-dir-srv' : ''));
                const dirText = isClt ? 'CLT ➔ SRV' : (isSrv ? 'SRV ➔ CLT' : (isUdp ? 'UDP' : 'TUNNEL'));
                const sizeStr = formatBytes(ev.body_len || (ev.body ? ev.body.length : 0));
                const statusColor = ev.patched ? 'var(--accent-green)' : (ev.status && (ev.status.startsWith('4') || ev.status.startsWith('5')) ? 'var(--accent-red)' : 'var(--muted)');

                html += `
                    <div class="traffic-row ${isSelected ? 'is-selected' : ''}" data-id="${ev.id}">
                        <span class="col-id">${ev.id}</span>
                        <span class="col-time">${escapeHtml(timeStr)}</span>
                        <span class="col-dir ${dirClass}">${dirText}</span>
                        <span class="col-type">${escapeHtml(ev.category)}</span>
                        <span class="col-svc" title="${escapeHtml(ev.service)}">
                            ${escapeHtml(ev.service)}
                            ${ev.patched ? '<span class="tag-patched">PATCHED</span>' : ''}
                        </span>
                        <span class="col-status" style="color:${statusColor}; font-weight:600;">${escapeHtml(ev.status || (ev.patched ? 'PATCHED' : 'OK'))}</span>
                        <span class="col-size">${escapeHtml(sizeStr)}</span>
                    </div>
                `;
            }

            grid.innerHTML = html;

            grid.querySelectorAll('.traffic-row[data-id]').forEach(r => {
                r.addEventListener('click', () => {
                    const id = parseInt(r.dataset.id, 10);
                    selectRow(id);
                });
            });

            if (document.getElementById('chkAutoScroll').checked) {
                grid.scrollTop = grid.scrollHeight;
            }
        }

        function selectRow(id) {
            selectedId = id;
            document.querySelectorAll('.traffic-row').forEach(r => {
                if (parseInt(r.dataset.id, 10) === id) r.classList.add('is-selected');
                else r.classList.remove('is-selected');
            });
            const ev = events.find(e => e.id === id);
            renderInspector(ev);
        }

        function renderInspector(ev) {
            const emptyEl = document.getElementById('inspectorEmpty');
            const contentEl = document.getElementById('inspectorContent');
            if (!ev) {
                emptyEl.style.display = 'flex';
                contentEl.style.display = 'none';
                return;
            }
            emptyEl.style.display = 'none';
            contentEl.style.display = 'flex';

            document.getElementById('metaService').textContent = ev.service || '(none)';
            document.getElementById('metaReqId').textContent = ev.req_id ? `ReqID: ${ev.req_id}` : 'ReqID: -';
            document.getElementById('metaRespId').textContent = ev.resp_id ? `RespID: ${ev.resp_id}` : 'RespID: -';
            document.getElementById('metaTime').textContent = `Time: ${ev.timestamp || '--'}`;
            document.getElementById('metaSize').textContent = formatBytes(ev.body_len || (ev.body ? ev.body.length : 0));

            const pre = document.getElementById('payloadView');
            let text = ev.body || '';
            if (prettyJson && (text.trim().startsWith('{') || text.trim().startsWith('['))) {
                try {
                    text = JSON.stringify(JSON.parse(text), null, 2);
                } catch {}
            }
            pre.textContent = text || '(empty body)';
        }

        // Event Listeners
        document.getElementById('btnToggle').addEventListener('click', async () => {
            const res = await fetch('/api/toggle', { method: 'POST' });
            if (res.ok) {
                const data = await res.json();
                isCapturing = data.capturing;
                updateStatusUI();
            }
        });

        document.getElementById('btnClear').addEventListener('click', async () => {
            await fetch('/api/clear', { method: 'POST' });
            events = [];
            selectedId = null;
            renderGrid();
            renderInspector(null);
        });

        document.getElementById('btnExport').addEventListener('click', () => {
            window.location.href = '/api/export';
        });

        document.getElementById('btnFormatToggle').addEventListener('click', () => {
            prettyJson = !prettyJson;
            document.getElementById('btnFormatToggle').textContent = prettyJson ? 'Show Raw' : 'Format JSON';
            const ev = events.find(e => e.id === selectedId);
            if (ev) renderInspector(ev);
        });

        document.getElementById('btnCopyPayload').addEventListener('click', () => {
            const text = document.getElementById('payloadView').textContent;
            navigator.clipboard.writeText(text).then(() => {
                const btn = document.getElementById('btnCopyPayload');
                const orig = btn.textContent;
                btn.textContent = 'Copied!';
                setTimeout(() => btn.textContent = orig, 1200);
            });
        });

        const searchInput = document.getElementById('searchInput');
        const searchClear = document.getElementById('searchClear');
        searchInput.addEventListener('input', (e) => {
            filterText = e.target.value.trim().toLowerCase();
            searchClear.style.display = filterText ? 'block' : 'none';
            renderGrid();
        });
        searchClear.addEventListener('click', () => {
            searchInput.value = '';
            filterText = '';
            searchClear.style.display = 'none';
            renderGrid();
        });

        document.querySelectorAll('.filter-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                document.querySelectorAll('.filter-btn').forEach(b => b.classList.remove('active'));
                btn.classList.add('active');
                activeFilter = btn.dataset.filter || 'all';
                renderGrid();
            });
        });

        function updateStatusUI() {
            const btn = document.getElementById('btnToggle');
            if (btn) {
                btn.textContent = isCapturing ? 'Pause' : 'Resume';
            }
        }

        // Polling loop
        setInterval(fetchEvents, 600);
        fetchEvents();
    </script>
</body>
</html>
"###;
