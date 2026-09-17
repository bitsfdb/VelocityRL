const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const API_BASE = 'https://api.velocityrl.tech';
const PRIVACY_POLICY_URL = 'https://velocityrl.tech/privacy.html';

function normItemSlot(slot) {
    if (!slot) return '';
    const s = slot.toLowerCase();
    if (s.includes('decal')) return 'Decal';
    return slot;
}

function escHtml(str) {
    return String(str).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');
}

let ownedItem = null;
let wantedItem = null;
let items = [];
let currentCategory = 'All';

let ownedSearch, wantedSearch, ownedResults, wantedResults, applyBtn, statusText, progressBarContainer, progressFill, backupContainer;
let swapBusy = false;

let appLoading = true;
let swallowUiUntil = 0;

function isAppLoading() {
    return appLoading || performance.now() < swallowUiUntil;
}

function setShellInert(on) {
    document.querySelector('.sidebar')?.toggleAttribute('inert', on);
    document.querySelector('.main-wrap')?.toggleAttribute('inert', on);
}

function releaseAppLoading() {
    if (!appLoading) return;
    appLoading = false;
    swallowUiUntil = performance.now() + 400;
    document.body.classList.remove('is-loading');
    setShellInert(false);
    const overlay = document.getElementById('app-loading-overlay');
    if (overlay) {
        overlay.hidden = true;
        overlay.setAttribute('aria-busy', 'false');
        overlay.setAttribute('aria-hidden', 'true');
    }
    validate();
    const restoreBtn = document.getElementById('restore-btn');
    if (restoreBtn && restoreBtn.dataset.busy !== '1') restoreBtn.disabled = false;
}

function wireLoadingGate() {
    const block = (e) => {
        if (e.target.closest?.('#settings-modal')) return;
        if (!isAppLoading()) return;
        e.preventDefault();
        e.stopImmediatePropagation();
    };
    document.addEventListener('pointerdown', block, true);
    document.addEventListener('click', block, true);
}

async function fetchItemsFromAPI() {
    const allItems = [];
    const limit = 200;
    let offset = 0;

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), 10000);

    try {
        while (true) {
            const res = await fetch(`${API_BASE}/v2/rl/products?limit=${limit}&offset=${offset}`, {
                signal: controller.signal
            });
            if (!res.ok) throw new Error(`API ${res.status}`);
        const data = await res.json();
        for (const p of data.products) {
            allItems.push({
                id: p.id,
                product: p.name,
                slot: p.category,
                quality: p.quality,
                asset_package: p.internal_name,
                image_url: p.thumbnail_url ? `${API_BASE}${p.thumbnail_url}` : '',
                paintable: p.paintable ?? p.Paintable,
                paints: p.paints ?? p.Paints,
                paint: p.paint ?? p.Paint,
                attributes: p.Attributes || p.attributes,
            });
        }
        if (allItems.length >= data.meta.total_filtered || data.products.length < limit) break;
        offset += limit;
    }
    } finally {
        clearTimeout(timeoutId);
    }

    return allItems;
}

function formatError(err) {
    if (err == null || err === '') return 'Unknown error';
    if (typeof err === 'string') return err;
    if (err instanceof Error) return err.stack || err.message || String(err);
    if (typeof err === 'object') {
        if (typeof err.message === 'string' && err.message) {
            const extra = err.code != null ? `\ncode: ${err.code}` : '';
            return err.message + extra;
        }
        try {
            return JSON.stringify(err, null, 2);
        } catch {
            return String(err);
        }
    }
    return String(err);
}

function showToast(message, type = 'success') {
    if (type !== 'error') return;
    const container = document.getElementById('toast-container');
    if (!container) return;

    const raw = type === 'error' ? formatError(message) : String(message);
    const toast = document.createElement('div');
    toast.className = `toast ${type}`;

    const contentEl = document.createElement('div');
    contentEl.className = 'toast-content';

    if (type === 'error') {
        const discordLink = 'https://discord.gg/2HhBNbrGMj';
        contentEl.innerHTML = `<div class="toast-error-body">${escHtml(raw)}</div><a href="#" class="toast-link" onclick="event.preventDefault(); window.__TAURI__.core.invoke('plugin:shell|open', { path: '${discordLink}' })">Join Support Discord</a>`;
        const copyBtn = document.createElement('button');
        copyBtn.className = 'toast-copy-btn';
        copyBtn.type = 'button';
        copyBtn.textContent = 'Copy';
        copyBtn.title = 'Copy full error';
        copyBtn.addEventListener('click', async (e) => {
            e.preventDefault();
            e.stopPropagation();
            try {
                await navigator.clipboard.writeText(raw);
            } catch {

                const ta = document.createElement('textarea');
                ta.value = raw;
                document.body.appendChild(ta);
                ta.select();
                document.execCommand('copy');
                ta.remove();
            }
            copyBtn.textContent = 'Copied';
            setTimeout(() => { copyBtn.textContent = 'Copy'; }, 1500);
        });
        toast.appendChild(contentEl);
        toast.appendChild(copyBtn);
    } else {
        contentEl.innerHTML = raw;
        toast.appendChild(contentEl);
    }

    container.appendChild(toast);

    const ttl = type === 'error' ? 20000 : 6000;
    setTimeout(() => {
        toast.style.animation = 'toastSlideOut 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards';
        setTimeout(() => toast.remove(), 300);
    }, ttl);
}

const qColorMap = {
    'Common': 'q-common',
    'Uncommon': 'q-uncommon',
    'Rare': 'q-rare',
    'Very Rare': 'q-veryrare',
    'Import': 'q-import',
    'Exotic': 'q-exotic',
    'Black Market': 'q-blackmarket',
    'Limited': 'q-limited'
};

const qBgMap = {
    'Common': 'bg-common',
    'Uncommon': 'bg-uncommon',
    'Rare': 'bg-rare',
    'Very Rare': 'bg-veryrare',
    'Import': 'bg-import',
    'Exotic': 'bg-exotic',
    'Black Market': 'bg-blackmarket',
    'Limited': 'bg-limited'
};

function emptyStateHtml() {
    return '<div class="empty-state"><p>No item selected</p></div>';
}

function renderSelectedItem(container, item, onClear) {
    const pName = item.Product || item.product || 'Unknown';
    const pQuality = item.Quality || item.quality || 'Common';
    const pSlot = item.Slot || item.slot || '';
    const pId = item.ID ?? item.id;
    const pImg = item.image_url || item.src || '';
    const bgClass = qBgMap[pQuality] || 'bg-common';

    container.innerHTML = `
        <div class="clear-item-btn">×</div>
        ${pImg ? `<img src="${escHtml(pImg)}" class="selected-img" />` : ''}
        <h2>${escHtml(pName)}</h2>
        <span class="quality-badge ${bgClass}">${escHtml(pQuality)}</span>
        <p class="item-slot-label">${escHtml(pSlot)}${pId != null ? ` · <span style="color:#5b8cff">ID ${escHtml(String(pId))}</span>` : ''}</p>
    `;
    container.querySelector('.clear-item-btn').addEventListener('click', onClear);
    container.classList.add('selected');
}

async function initVersionBadge() {
    try {
        const info = await invoke('get_build_info');
        const label = `v${info.version} · ${info.build_number}`;
        const btn = document.getElementById('version-btn');
        if (btn) {
            btn.textContent = `What's New · ${label}`;
            btn.title = `VelocityRL ${label} (${info.build_hash})`;
        }
    } catch (_) {
        // fallback: keep the hardcoded text from index.html
    }
}

async function init() {
    wireLoadingGate();
    ownedSearch = document.getElementById('owned-search');
    wantedSearch = document.getElementById('wanted-search');
    ownedResults = document.getElementById('owned-results');
    wantedResults = document.getElementById('wanted-results');
    applyBtn = document.getElementById('apply-swap');
    statusText = document.getElementById('status-text');
    progressBarContainer = document.getElementById('progress-bar-container');
    progressFill = document.getElementById('progress-fill');
    backupContainer = document.getElementById('backup-container');

    setupSearch(ownedSearch, ownedResults, (item) => {
        ownedItem = item;
        renderSelectedItem(document.getElementById('owned-selected'), item, clearOwned);
        ownedSearch.value = item.Product || item.product || 'Unknown';
        validate();
    });

    setupSearch(wantedSearch, wantedResults, (item) => {
        wantedItem = item;
        renderSelectedItem(document.getElementById('wanted-selected'), item, clearWanted);
        wantedSearch.value = item.Product || item.product || 'Unknown';
        validate();
    });

    document.querySelectorAll('.nav-item[data-tab]').forEach(btn => {
        btn.onclick = () => {
            if (isAppLoading()) return;
            document.querySelectorAll('.nav-item').forEach(b => b.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            btn.classList.add('active');
            document.getElementById(btn.dataset.tab).classList.add('active');
            if (btn.dataset.tab === 'swapper-tab') refreshSwapRlHint();
            if (btn.dataset.tab === 'titles-tab') initTitlesTab();
            if (btn.dataset.tab === 'names-tab') initNamesTab();
            if (btn.dataset.tab === 'ranks-tab') initRanksTab();
            if (btn.dataset.tab === 'tracker-tab') initTrackerTab();
            if (btn.dataset.tab === 'camera-tab') initCameraTab();
            if (btn.dataset.tab === 'maps-tab') initWorkshopTab();
            if (btn.dataset.tab === 'misc-tab') initMiscTab();
        };
    });

    document.querySelectorAll('.subtab-btn').forEach(btn => {
        btn.onclick = () => {
            if (isAppLoading()) return;
            const paneId = btn.dataset.subtab;
            document.querySelectorAll('.subtab-btn').forEach(b => b.classList.remove('active'));
            document.querySelectorAll('.subtab-pane').forEach(p => p.classList.remove('active'));
            btn.classList.add('active');
            document.getElementById(paneId)?.classList.add('active');
            if (paneId === 'restore-pane') refreshBackups();
            if (paneId === 'presets-pane') refreshPresets();
            if (paneId === 'maplib-pane') refreshWorkshopLibrary();
            if (paneId === 'mappresets-pane') refreshWorkshopPresets();
            if (paneId === 'browser-pane') loadWorkshopCatalog(1, workshopCatalogQuery);
        };
    });

    document.querySelectorAll('.cat-btn').forEach(btn => {
        btn.onclick = () => {
            if (isAppLoading()) return;
            document.querySelectorAll('.cat-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            currentCategory = btn.dataset.slot;
            ownedSearch.dispatchEvent(new Event('input'));
            wantedSearch.dispatchEvent(new Event('input'));
        };
    });

    applyBtn.onclick = handleApply;
    document.getElementById('restore-btn').onclick = handleRestore;
    document.getElementById('website-btn').onclick = () => {
        if (isAppLoading()) return;
        window.__TAURI__.core.invoke('plugin:shell|open', { path: 'https://velocityrl.tech' });
    };
    document.getElementById('settings-btn').onclick = async () => {
        if (isAppLoading()) return;
        const cfg = await invoke('get_config').catch(() => ({ game_dir: '' }));
        document.getElementById('game-dir').value = cfg.game_dir || '';
        document.getElementById('settings-modal').classList.add('active');
    };
    document.getElementById('version-btn').onclick = async (e) => {
        if (isAppLoading()) return;
        if (e.shiftKey) {
            document.getElementById('dev-modal').classList.add('active');
            await refreshDevPanel();
            return;
        }
        openChangelog();
    };
    document.getElementById('close-changelog').onclick = () => document.getElementById('changelog-modal').classList.remove('active');
    document.getElementById('changelog-modal').onclick = (e) => { if (e.target === document.getElementById('changelog-modal')) document.getElementById('changelog-modal').classList.remove('active'); };
    document.getElementById('toggle-changelog-startup').onclick = async () => {
        const cfg = await invoke('get_config').catch(() => ({}));
        const newVal = cfg.changelog_on_startup === false;
        await invoke('save_config', { config: { ...cfg, changelog_on_startup: newVal } }).catch(() => {});
        document.getElementById('toggle-changelog-startup').textContent = newVal ? "Don't show on startup" : 'Show on startup';
        showToast(newVal ? 'Changelog will show on startup' : "Changelog hidden on startup", 'success');
    };
    document.getElementById('cancel-settings').onclick = handleCancelSettings;
    document.getElementById('close-settings').onclick = handleSaveSettings;
    document.getElementById('browse-dir').onclick = handleBrowse;
    document.getElementById('autodetect-dir').onclick = handleAutoDetect;    document.getElementById('settings-modal').onclick = (e) => {
        if (e.target === document.getElementById('settings-modal')) handleCancelSettings();
    };

    document.getElementById('close-dev-modal')?.addEventListener('click', () => {
        document.getElementById('dev-modal').classList.remove('active');
    });
    document.getElementById('dev-modal')?.addEventListener('click', (e) => {
        if (e.target === document.getElementById('dev-modal')) document.getElementById('dev-modal').classList.remove('active');
    });
    document.getElementById('dev-refresh-btn')?.addEventListener('click', refreshDevPanel);
    document.getElementById('dev-save-config-btn')?.addEventListener('click', async () => {
        const cfgEl = document.getElementById('dev-proxy-config');
        if (!cfgEl) return;
        try {
            JSON.parse(cfgEl.value);
            await invoke('save_psynet_config_json', { raw: cfgEl.value });
            showToast('Proxy config saved.', 'success');
        } catch (e) {
            showToast('Save failed: ' + e, 'error');
        }
    });
    document.getElementById('dev-save-replay-identity-btn')?.addEventListener('click', async () => {
        const pidEl = document.getElementById('dev-replay-player-id');
        const rnEl = document.getElementById('dev-replay-real-name');
        if (!pidEl || !rnEl) return;
        try {
            await invoke('save_replay_identity', { realName: rnEl.value, playerId: pidEl.value });
            showToast('Replay vault identity saved.', 'success');
            const cfgEl = document.getElementById('dev-proxy-config');
            if (cfgEl) {
                try {
                    const raw = await invoke('get_psynet_config_json');
                    cfgEl.value = JSON.stringify(JSON.parse(raw), null, 2);
                } catch {}
            }
        } catch (e) {
            showToast('Save failed: ' + e, 'error');
        }
    });
    document.getElementById('dev-open-log-btn')?.addEventListener('click', async () => {
        try {
            await invoke('open_log_folder');
        } catch (e) {
            showToast('Could not open log folder: ' + e, 'error');
        }
    });
    document.getElementById('dev-save-gamedir-btn')?.addEventListener('click', async () => {
        const input = document.getElementById('dev-game-dir-input');
        if (!input) return;
        const c = await invoke('get_config').catch(() => ({}));
        await invoke('save_config', { config: { ...c, game_dir: input.value.trim() } });
        showToast('Game dir saved.', 'success');
    });
    document.getElementById('dev-restart-proxy-btn')?.addEventListener('click', async () => {
        try {
            await invoke('restart_psynet_proxy');
            showToast('Proxy restarted.', 'success');
        } catch (e) {
            showToast('Proxy restart failed: ' + e, 'error');
        }
        setTimeout(refreshDevPanel, 1000);
    });
    document.getElementById('dev-stop-proxy-btn')?.addEventListener('click', async () => {
        try {
            await invoke('stop_psynet_proxy', { revertHosts: false });
            showToast('Proxy stopped.', 'success');
        } catch (e) {
            showToast('Proxy stop failed: ' + e, 'error');
        }
        setTimeout(refreshDevPanel, 1000);
    });
    document.getElementById('dev-delete-certs-btn')?.addEventListener('click', async () => {
        const msg = document.getElementById('dev-action-msg');
        if (msg) msg.textContent = 'Deleting certificates…';
        try {
            const res = await invoke('delete_ca_certificates');
            showToast(res || 'Certificates deleted.', 'success');
            if (msg) msg.textContent = res || 'Certificates deleted.';
        } catch (e) {
            showToast('Delete failed: ' + e, 'error');
            if (msg) msg.textContent = 'Delete failed: ' + e;
        }
        setTimeout(refreshDevPanel, 1000);
    });
    document.getElementById('dev-save-proxydir-btn')?.addEventListener('click', async () => {
        const input = document.getElementById('dev-proxy-dir-input');
        const msg = document.getElementById('dev-action-msg');
        if (!input) return;
        try {
            const saved = await invoke('save_proxy_dir', { path: input.value.trim() });
            if (msg) msg.textContent = saved ? `Saved: ${saved}` : 'Proxy dir reset to auto-detect.';
            setTimeout(refreshDevPanel, 500);
        } catch (e) {
            if (msg) msg.textContent = 'Save failed: ' + e;
        }
    });
    document.getElementById('dev-export-diag-btn')?.addEventListener('click', async () => {
        const msg = document.getElementById('dev-action-msg');
        if (msg) msg.textContent = 'Bundling diagnostics…';
        try {
            const path = await invoke('export_diagnostics');
            try {
                if (navigator.clipboard && navigator.clipboard.writeText) {
                    await navigator.clipboard.writeText(path);
                }
            } catch (_) {}
            if (msg) msg.textContent = `Exported (copied to clipboard!): ${path}`;
            const dir = path.substring(0, Math.max(path.lastIndexOf('\\'), path.lastIndexOf('/')));
            if (dir) window.__TAURI__?.core?.invoke('plugin:shell|open', { path: dir });
        } catch (e) {
            if (msg) msg.textContent = 'Export failed: ' + e;
        }
    });

    await loadData();
}

let loadingPercent = 0;
const loadingSteps = {
    'Checking for repairs...': 5,
    'Loading item database...': 15,
    'Downloading item database (fallback)...': 20,
    'Loading config...': 35,
    'Loading customizations...': 50,
    'Ensuring PsyNet hosts...': 65,
    'Starting proxy server...': 80,
    'Starting up...': 95,
};

function updateLoadingText(text) {
    const p = document.querySelector('.app-loading-text');
    if (p) p.textContent = text;
    const fill = document.getElementById('app-loading-fill');
    const pct = document.getElementById('app-loading-percent');
    if (loadingSteps[text] !== undefined) {
        loadingPercent = loadingSteps[text];
    }
    if (fill) fill.style.width = loadingPercent + '%';
    if (pct) pct.textContent = loadingPercent + '%';
}

async function loadBackups() {
    refreshBackups();
}

async function loadData() {
    try {
        updateStatus('Please Wait...', false);
        updateLoadingText('Loading config...');

        const [repair, config, itemsResult] = await Promise.all([
            invoke('check_integrity').catch(e => { console.warn('Repair check failed:', e); return null; }),
            invoke('get_config').catch(e => { console.warn('Config load failed:', e); return { game_dir: '' }; }),
            invoke('get_items').catch(() => null),
        ]);

        if (repair && repair.repaired) {
            sessionStorage.setItem('velocityrl_repair_report', JSON.stringify(repair));
        }

        if (itemsResult) {
            items = itemsResult;
        } else {

            invoke('get_items').catch(() => {}).then(fetched => {
                if (fetched) { items = fetched; }
            });
        }

        if (config && config.game_dir) {
            document.getElementById('game-dir').value = config.game_dir;
            invoke('repair_engine_refs').then((msg) => {
                if (!msg) return;
                const text = String(msg);
                if (text.includes('Color palette') || text.includes('newer than TAGame') || text.includes('Reset for verify')) {
                    showToast(text, 'error');
                } else if (!text.includes('already')) {
                    showToast(text, 'success');
                }
            }).catch(() => {});
        } else {

            invoke('detect_game_dir').catch(() => []).then(async (installs) => {
                if (!installs || !installs.length) {
                    document.getElementById('settings-modal').classList.add('active');
                    return;
                }
                document.getElementById('game-dir').value = installs[0].path;
                if (installs.length === 1) {
                    await invoke('save_config', { config: { ...config, game_dir: installs[0].path } }).catch(() => {});
                    showToast(`Rocket League detected: ${installs[0].label}`, 'success');
                    refreshPaletteStatus();
                } else {
                    document.getElementById('settings-modal').classList.add('active');
                    showInstallChooser(installs);
                }
            });
        }

        updateStatus('bitsfdb', false);
        invoke('cleanup_temp_files').catch(() => {});

        updateLoadingText('Ensuring PsyNet hosts...');
        try { await invoke('ensure_psynet_hosts'); } catch (e) { console.warn('psynet hosts:', e); }

        updateLoadingText('Starting proxy server...');
        autoStartPsyNetProxy().catch(e => console.warn('proxy autostart:', e));

        updateLoadingText('Starting up...');
        loadingPercent = 100;
        const fill = document.getElementById('app-loading-fill');
        const pct = document.getElementById('app-loading-percent');
        if (fill) fill.style.width = '100%';
        if (pct) pct.textContent = '100%';
        await new Promise(r => setTimeout(r, 150));
        releaseAppLoading();
        wirePresetsUI();
    } catch (err) {
        releaseAppLoading();
        updateStatus('Init Failure', true);
        appDialog({ title: 'Initialization Failed', message: `VelocityRL failed to start:\n${err.message || err}` }).catch(() => {});
        console.error(err);
        invoke('report_diagnostic', { payload: {
            event:     'init_fail',
            context:   'init',
            message:   String(err?.message ?? err),
            backtrace: err?.stack ?? null,
        }}).catch(() => {});
    }
}

function clearOwned() {
    ownedItem = null;
    const container = document.getElementById('owned-selected');
    container.innerHTML = emptyStateHtml();
    container.classList.remove('selected');
    document.getElementById('owned-search').value = '';
    validate();
}

function clearWanted() {
    wantedItem = null;
    const container = document.getElementById('wanted-selected');
    container.innerHTML = emptyStateHtml();
    container.classList.remove('selected');
    document.getElementById('wanted-search').value = '';
    validate();
}

window.clearOwned = clearOwned;
window.clearWanted = clearWanted;

const RESTORE_SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#6b7280" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/></svg>';

let presetsLoadedOnce = false;

let presetModalState = {
    preset: null,
    isImport: false,
    code: null,
    missingMaps: [],
    itemsPage: 1,
    itemsPerPage: 6,
    mapsPage: 1,
    mapsPerPage: 4,
};

async function openPresetPreviewModal(preset, { isImport = false, code = null } = {}) {
    const modal = document.getElementById('preset-preview-modal');
    if (!modal) return;

    presetModalState.preset = preset;
    presetModalState.isImport = isImport;
    presetModalState.code = code;
    presetModalState.itemsPage = 1;
    presetModalState.mapsPage = 1;

    const titleEl = document.getElementById('preset-preview-title');
    if (titleEl) titleEl.textContent = preset.name || 'Preset Loadout';

    const swaps = preset.swaps || [];
    const maps = preset.maps || [];

    const badgeItems = document.getElementById('preset-badge-items');
    if (badgeItems) {
        badgeItems.textContent = `${swaps.length} / 100 Items`;
        badgeItems.style.display = swaps.length > 0 ? 'inline-flex' : 'none';
    }
    const badgeMaps = document.getElementById('preset-badge-maps');
    if (badgeMaps) {
        badgeMaps.textContent = `${maps.length} / 30 Maps`;
        badgeMaps.style.display = maps.length > 0 ? 'inline-flex' : 'none';
    }

    const library = await invoke('workshop_get_map_library').catch(() => []);
    const mapsWithStatus = maps.map(m => {
        const isDownloaded = library.some(e =>
            (e.name && m.name && e.name.toLowerCase() === m.name.toLowerCase()) ||
            (m.download_url && e.source_url === m.download_url) ||
            (m.id && e.path && e.path.includes(m.id))
        );
        return { ...m, isDownloaded };
    });
    presetModalState.preset.maps = mapsWithStatus;

    const missingMaps = mapsWithStatus.filter(m => !m.isDownloaded);
    presetModalState.missingMaps = missingMaps;

    const banner = document.getElementById('preset-download-banner');
    const bannerTitle = document.getElementById('preset-download-banner-title');
    const bannerDesc = document.getElementById('preset-download-banner-desc');
    if (banner && bannerTitle && bannerDesc) {
        if (maps.length > 0) {
            banner.style.display = 'block';
            if (missingMaps.length > 0) {
                bannerTitle.textContent = `${missingMaps.length} map${missingMaps.length === 1 ? '' : 's'} to download`;
                bannerDesc.textContent = `${maps.length - missingMaps.length} already in your library. When you proceed, ${missingMaps.length} will be downloaded and saved to your Workshop Library automatically.`;
            } else {
                bannerTitle.textContent = `All ${maps.length} map${maps.length === 1 ? '' : 's'} already downloaded`;
                bannerDesc.textContent = `All maps in this preset are ready locally.`;
            }
        } else {
            banner.style.display = 'none';
        }
    }

    const tabItemsBtn = document.getElementById('preset-tab-items-btn');
    const tabMapsBtn = document.getElementById('preset-tab-maps-btn');
    const itemsView = document.getElementById('preset-items-view');
    const mapsView = document.getElementById('preset-maps-view');

    const switchTab = (tab) => {
        if (tab === 'maps') {
            tabMapsBtn?.classList.add('active');
            tabItemsBtn?.classList.remove('active');
            if (mapsView) mapsView.style.display = 'flex';
            if (itemsView) itemsView.style.display = 'none';
            renderPresetMapsPage();
        } else {
            tabItemsBtn?.classList.add('active');
            tabMapsBtn?.classList.remove('active');
            if (itemsView) itemsView.style.display = 'flex';
            if (mapsView) mapsView.style.display = 'none';
            renderPresetItemsPage();
        }
    };

    if (tabItemsBtn) {
        tabItemsBtn.style.display = swaps.length > 0 ? 'inline-block' : 'none';
        tabItemsBtn.onclick = () => switchTab('items');
    }
    if (tabMapsBtn) {
        tabMapsBtn.style.display = maps.length > 0 ? 'inline-block' : 'none';
        tabMapsBtn.onclick = () => switchTab('maps');
    }

    if (swaps.length === 0 && maps.length > 0) {
        switchTab('maps');
    } else {
        switchTab('items');
    }

    const actionBtn = document.getElementById('preset-preview-action');
    const progressWrap = document.getElementById('preset-download-progress-wrap');
    if (progressWrap) progressWrap.style.display = 'none';

    if (actionBtn) {
        actionBtn.disabled = false;
        if (isImport) {
            actionBtn.textContent = missingMaps.length > 0
                ? `Download ${missingMaps.length} Map${missingMaps.length === 1 ? '' : 's'} & Save`
                : 'Save Preset';
        } else {
            actionBtn.textContent = missingMaps.length > 0
                ? `Download ${missingMaps.length} Map${missingMaps.length === 1 ? '' : 's'} & Apply`
                : 'Apply Preset';
        }

        actionBtn.onclick = async () => {
            actionBtn.disabled = true;
            if (missingMaps.length > 0) {
                if (progressWrap) progressWrap.style.display = 'flex';
                const pStatus = document.getElementById('preset-download-progress-status');
                const pCount = document.getElementById('preset-download-progress-count');
                const pBar = document.getElementById('preset-download-progress-bar');
                if (pStatus) pStatus.textContent = `Downloading 1 of ${missingMaps.length}: ${missingMaps[0].name}...`;
                if (pCount) pCount.textContent = `0 / ${missingMaps.length}`;
                if (pBar) pBar.style.width = '10%';

                try {
                    await invoke('preset_download_missing_maps', { maps: missingMaps });
                    if (pBar) pBar.style.width = '100%';
                    if (pStatus) pStatus.textContent = 'All maps downloaded successfully.';
                } catch (err) {
                    showToast(`Some maps failed to download: ${err}`, 'warning');
                }
            }

            if (isImport && code) {
                try {
                    const saved = await invoke('import_preset_code', { code });
                    showToast(`Preset <strong>${escHtml(saved.name)}</strong> saved`, 'success');
                    modal.classList.remove('active');
                    refreshPresets();
                } catch (err) {
                    showToast(String(err), 'error');
                    actionBtn.disabled = false;
                }
            } else if (!isImport && preset.id) {
                modal.classList.remove('active');
                await applyPresetDirect(preset);
            }
        };
    }

    const closeBtn = document.getElementById('preset-preview-close');
    const cancelBtn = document.getElementById('preset-preview-cancel');
    const closeModal = () => modal.classList.remove('active');
    if (closeBtn) closeBtn.onclick = closeModal;
    if (cancelBtn) cancelBtn.onclick = closeModal;

    modal.classList.add('active');
}

function renderPresetItemsPage() {
    const swaps = presetModalState.preset?.swaps || [];
    const list = document.getElementById('preset-items-list');
    const pageInfo = document.getElementById('preset-items-page-info');
    const prevBtn = document.getElementById('preset-items-prev');
    const nextBtn = document.getElementById('preset-items-next');
    if (!list) return;

    if (!swaps.length) {
        list.innerHTML = '<div class="backup-empty">No item swaps in this preset.</div>';
        if (pageInfo) pageInfo.textContent = 'Page 0 of 0';
        if (prevBtn) prevBtn.disabled = true;
        if (nextBtn) nextBtn.disabled = true;
        return;
    }

    const perPage = presetModalState.itemsPerPage;
    const totalPages = Math.ceil(swaps.length / perPage) || 1;
    presetModalState.itemsPage = Math.max(1, Math.min(presetModalState.itemsPage, totalPages));
    const page = presetModalState.itemsPage;

    const start = (page - 1) * perPage;
    const itemsToShow = swaps.slice(start, start + perPage);

    list.innerHTML = itemsToShow.map(s => {
        const slot = normItemSlot(s.slot || 'Item');
        const paint = s.paint_id > 0 ? `<span class="quality-badge bg-uncommon" style="font-size:9px;padding:2px 5px;">Paint ${s.paint_id}</span>` : '';
        return `
            <div class="preset-item-row">
                <span class="preset-item-slot">${escHtml(slot)}</span>
                <div class="preset-item-names">
                    <span style="color:var(--text);font-weight:500;">${escHtml(s.owned_name)}</span>
                    <span class="preset-item-arrow">→</span>
                    <span style="color:var(--accent-blue);font-weight:600;">${escHtml(s.wanted_name)}</span>
                </div>
                ${paint}
            </div>
        `;
    }).join('');

    if (pageInfo) pageInfo.textContent = `Page ${page} of ${totalPages} (${swaps.length} items)`;
    if (prevBtn) {
        prevBtn.disabled = page <= 1;
        prevBtn.onclick = () => { presetModalState.itemsPage--; renderPresetItemsPage(); };
    }
    if (nextBtn) {
        nextBtn.disabled = page >= totalPages;
        nextBtn.onclick = () => { presetModalState.itemsPage++; renderPresetItemsPage(); };
    }
}

function renderPresetMapsPage() {
    const maps = presetModalState.preset?.maps || [];
    const list = document.getElementById('preset-maps-list');
    const pageInfo = document.getElementById('preset-maps-page-info');
    const prevBtn = document.getElementById('preset-maps-prev');
    const nextBtn = document.getElementById('preset-maps-next');
    if (!list) return;

    if (!maps.length) {
        list.innerHTML = '<div class="backup-empty" style="grid-column:1/-1;">No workshop maps in this preset.</div>';
        if (pageInfo) pageInfo.textContent = 'Page 0 of 0';
        if (prevBtn) prevBtn.disabled = true;
        if (nextBtn) nextBtn.disabled = true;
        return;
    }

    const perPage = presetModalState.mapsPerPage;
    const totalPages = Math.ceil(maps.length / perPage) || 1;
    presetModalState.mapsPage = Math.max(1, Math.min(presetModalState.mapsPage, totalPages));
    const page = presetModalState.mapsPage;

    const start = (page - 1) * perPage;
    const mapsToShow = maps.slice(start, start + perPage);

    list.innerHTML = mapsToShow.map(m => {
        const thumb = m.thumbnail_url
            ? `<img class="preset-map-thumb" src="${escHtml(m.thumbnail_url)}" alt="${escHtml(m.name)}" onerror="this.outerHTML='<div class=\\'preset-map-thumb-placeholder\\'>Map</div>'">`
            : `<div class="preset-map-thumb-placeholder">Map</div>`;
        const pill = m.isDownloaded
            ? `<span class="map-status-pill map-status-downloaded">In Library</span>`
            : `<span class="map-status-pill map-status-pending">Needs Download</span>`;
        return `
            <div class="preset-map-card">
                ${thumb}
                <div class="preset-map-info">
                    <h4 class="preset-map-name" title="${escHtml(m.name)}">${escHtml(m.name)}</h4>
                    ${pill}
                </div>
            </div>
        `;
    }).join('');

    if (pageInfo) pageInfo.textContent = `Page ${page} of ${totalPages} (${maps.length} maps)`;
    if (prevBtn) {
        prevBtn.disabled = page <= 1;
        prevBtn.onclick = () => { presetModalState.mapsPage--; renderPresetMapsPage(); };
    }
    if (nextBtn) {
        nextBtn.disabled = page >= totalPages;
        nextBtn.onclick = () => { presetModalState.mapsPage++; renderPresetMapsPage(); };
    }
}

async function refreshPresets() {
    const list = document.getElementById('preset-list');
    if (!list) return;
    try {
        const presets = await invoke('get_presets');
        if (!presets.length) {
            list.innerHTML = '<div class="backup-empty">No presets yet. Set up swaps, then click "Save current as preset".</div>';
        } else {
            list.innerHTML = '';
            presets.forEach(p => {
                const row = document.createElement('div');
                row.className = 'backup-item';
                row.style.display = 'flex';
                row.style.alignItems = 'center';
                row.style.gap = '10px';

                const swapCount = (p.swaps || []).length;
                const mapCount = (p.maps || []).length;
                const swapBadge = `<span class="preset-stat-badge" style="font-size:10px;padding:2px 6px;">${swapCount} items</span>`;
                const mapBadge = mapCount > 0 ? `<span class="preset-stat-badge" style="font-size:10px;padding:2px 6px;background:rgba(33,150,243,0.1);color:#2196f3;border-color:rgba(33,150,243,0.3);">${mapCount} maps</span>` : '';
                const summaryLine = (p.swaps || []).slice(0, 3).map(s => `${s.owned_name} → ${s.wanted_name}`).join(', ') + ((p.swaps || []).length > 3 ? '...' : '');

                row.innerHTML = `
                    <div style="flex:1;min-width:0;">
                        <div style="display:flex;align-items:center;gap:6px;flex-wrap:wrap;">
                            <strong>${escHtml(p.name)}</strong>
                            ${swapBadge}
                            ${mapBadge}
                        </div>
                        <div style="font-size:12px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;margin-top:2px;">${escHtml(summaryLine || (mapCount > 0 ? `${mapCount} map(s)` : 'Empty preset'))}</div>
                    </div>`;
                const mkBtn = (label, cls, fn) => {
                    const b = document.createElement('button');
                    b.className = `action-btn ${cls}`;
                    b.textContent = label;
                    b.style.padding = '6px 12px';
                    b.style.fontSize = '12px';
                    b.onclick = fn;
                    return b;
                };
                row.appendChild(mkBtn('Apply', '', () => applyPreset(p)));
                row.appendChild(mkBtn('View', 'action-btn-secondary', () => openPresetPreviewModal(p, { isImport: false })));
                row.appendChild(mkBtn('Share', 'action-btn-secondary', (e) => sharePreset(p, e.currentTarget)));
                row.appendChild(mkBtn('Delete', 'action-btn-secondary', async () => {
                    await invoke('delete_preset', { id: p.id });
                    refreshPresets();
                }));
                list.appendChild(row);
            });
        }
        presetsLoadedOnce = true;
        refreshSwapHistory();
    } catch (e) {
        list.innerHTML = `<div class="backup-empty">Failed to load presets: ${escHtml(String(e))}</div>`;
    }
}

async function applyPresetDirect(p) {
    updateStatus('Applying preset...', false);
    showProgress(true, 30);
    try {
        const results = await invoke('apply_preset', { id: p.id });
        showProgress(true, 100);
        const fails = results.filter(r => r.startsWith('FAIL'));
        if (fails.length === 0) {
            showToast(`Preset <strong>${escHtml(p.name)}</strong> applied`, 'success');
        } else {
            showToast(`Preset applied with ${fails.length} failure(s) — see swap history`, 'warning');
        }
        await refreshBackups();
    } catch (e) {
        showToast(String(e), 'error');
    } finally {
        setTimeout(() => { showProgress(false); updateStatus('bitsfdb', false); }, 1500);
    }
}

async function applyPreset(p) {
    const maps = p.maps || [];
    const library = await invoke('workshop_get_map_library').catch(() => []);
    const missing = maps.filter(m => !library.some(e =>
        (e.name && m.name && e.name.toLowerCase() === m.name.toLowerCase()) ||
        (m.download_url && e.source_url === m.download_url)
    ));
    if (missing.length > 0) {
        openPresetPreviewModal(p, { isImport: false });
        return;
    }
    const mapNote = maps.length > 0 ? ` Includes ${maps.length} map(s).` : '';
    if (!(await askConfirm(`Apply preset "${p.name}"? This applies ${(p.swaps || []).length} swap(s).${mapNote}`, 'Apply Preset'))) return;
    await applyPresetDirect(p);
}

async function copyText(text) {

    try { await navigator.clipboard.writeText(text); return true; } catch {}
    try { await invoke('copy_to_clipboard', { text }); return true; } catch {}
    try {
        const ta = document.createElement('textarea');
        ta.value = text;
        ta.style.cssText = 'position:fixed;opacity:0;';
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        ta.remove();
        return true;
    } catch { return false; }
}

function closeShareDropdown() {
    document.querySelectorAll('.share-dropdown').forEach(d => d.remove());
}

async function sharePreset(p, anchorEl) {
    try {
        closeShareDropdown();
        const code = await invoke('export_preset_code', { id: p.id });
        const copied = await copyText(code);

        const dd = document.createElement('div');
        dd.className = 'share-dropdown';
        dd.style.cssText = 'position:fixed;z-index:1000;background:var(--bg-secondary,#1a1a1a);border:1px solid var(--border,#333);border-radius:8px;padding:10px 12px;box-shadow:0 8px 24px rgba(0,0,0,.5);max-width:min(420px, calc(100vw - 24px));width:min(420px, calc(100vw - 24px));box-sizing:border-box;left:0;top:0;';
        dd.innerHTML = `
            <div style="font-size:12px;font-weight:600;margin-bottom:6px;">${escHtml(p.name)} — code ${copied ? 'copied to clipboard' : 'not copied'}</div>
            <div style="font-size:11px;color:var(--text-secondary);margin-bottom:8px;">Paste it anywhere to share. It contains the swaps only — no personal info.</div>
            <button class="action-btn action-btn-secondary" id="share-copy-again" type="button" style="width:100%;">${copied ? 'Copy again' : 'Copy code'}</button>`;
        dd.querySelector('#share-copy-again')?.addEventListener('click', async () => {
            const ok = await copyText(code);
            showToast(ok ? 'Code copied.' : 'Copy failed — select and copy manually.', ok ? 'success' : 'error');
        });

        document.body.appendChild(dd);
        const anchor = anchorEl || dd.previousElementSibling;
        const ar = anchor?.getBoundingClientRect?.() || { left: 24, bottom: 80, top: 40 };
        dd.style.position = 'fixed';
        let left = Math.min(ar.left, window.innerWidth - 440);
        left = Math.max(12, left);
        let top = ar.bottom + 6;
        const ddH = dd.offsetHeight || 120;
        if (top + ddH > window.innerHeight - 12) {
            top = Math.max(12, ar.top - ddH - 6);
        }
        dd.style.left = `${left}px`;
        dd.style.top = `${top}px`;

        setTimeout(() => {
            const onOutside = (ev) => {
                if (!dd.contains(ev.target)) { closeShareDropdown(); document.removeEventListener('mousedown', onOutside); window.removeEventListener('scroll', onScroll, true); }
            };
            const onScroll = () => closeShareDropdown();
            document.addEventListener('mousedown', onOutside);
            window.addEventListener('scroll', onScroll, true);
        }, 0);
    } catch (e) {
        showToast(String(e), 'error');
    }
}

function appDialog({ title = 'VelocityRL', message = '', input = null, okLabel = 'OK', cancelLabel = 'Cancel' } = {}) {
    return new Promise(resolve => {
        const overlay = document.getElementById('app-dialog-overlay');
        if (!overlay) { resolve(input !== null ? (window.prompt(message) || '') : window.confirm(message)); return; }
        const titleEl = document.getElementById('app-dialog-title');
        const msgEl = document.getElementById('app-dialog-message');
        const inputGroup = document.getElementById('app-dialog-input-group');
        const inputEl = document.getElementById('app-dialog-input');
        const okBtn = document.getElementById('app-dialog-ok');
        const cancelBtn = document.getElementById('app-dialog-cancel');
        const closeBtn = document.getElementById('app-dialog-close');
        titleEl.textContent = title;
        msgEl.textContent = message;
        if (input !== null) {
            inputGroup.style.display = 'block';
            inputEl.value = input;
        } else {
            inputGroup.style.display = 'none';
            inputEl.value = '';
        }
        okBtn.textContent = okLabel;
        cancelBtn.textContent = cancelLabel;
        overlay.classList.add('active');
        const finish = (value) => {
            overlay.classList.remove('active');
            okBtn.onclick = cancelBtn.onclick = closeBtn.onclick = null;
            inputEl.onkeydown = overlay.onkeydown = null;
            resolve(value);
        };
        okBtn.onclick = () => finish(input !== null ? inputEl.value.trim() : true);
        cancelBtn.onclick = () => finish(input !== null ? null : false);
        closeBtn.onclick = () => finish(input !== null ? null : false);
        inputEl.onkeydown = (e) => {
            if (e.key === 'Enter') { e.preventDefault(); finish(inputEl.value.trim()); }
            if (e.key === 'Escape') { e.preventDefault(); finish(null); }
        };
        overlay.onkeydown = (e) => {
            if (e.key === 'Escape') { e.preventDefault(); finish(input !== null ? null : false); }
        };
        if (input !== null) setTimeout(() => { inputEl.focus(); inputEl.select(); }, 50);
    });
}

function askConfirm(message, title) {
    return appDialog({ title: title || 'Confirm', message, okLabel: 'OK', cancelLabel: 'Cancel' });
}

function threeWayDialog({ title = 'VelocityRL', message = '', okLabel = 'OK', extraLabel = 'Option', cancelLabel = 'Cancel' } = {}) {
    return new Promise(resolve => {
        const overlay = document.getElementById('app-dialog-overlay');
        if (!overlay) { resolve(null); return; }
        const titleEl = document.getElementById('app-dialog-title');
        const msgEl = document.getElementById('app-dialog-message');
        const inputGroup = document.getElementById('app-dialog-input-group');
        const okBtn = document.getElementById('app-dialog-ok');
        const cancelBtn = document.getElementById('app-dialog-cancel');
        const closeBtn = document.getElementById('app-dialog-close');
        titleEl.textContent = title;
        msgEl.textContent = message;
        inputGroup.style.display = 'none';
        okBtn.textContent = okLabel;
        cancelBtn.textContent = cancelLabel;

        cancelBtn.style.display = 'none';
        const extraBtn = document.createElement('button');
        extraBtn.className = 'action-btn action-btn-secondary';
        extraBtn.type = 'button';
        extraBtn.textContent = extraLabel;
        extraBtn.style.marginRight = 'auto';
        const btnRow = okBtn.parentElement;
        btnRow.insertBefore(extraBtn, okBtn);
        overlay.classList.add('active');
        const finish = (value) => {
            overlay.classList.remove('active');
            okBtn.onclick = cancelBtn.onclick = closeBtn.onclick = extraBtn.onclick = null;
            extraBtn.remove();
            cancelBtn.style.display = '';
            resolve(value);
        };
        okBtn.onclick = () => finish('ok');
        extraBtn.onclick = () => finish('extra');
        cancelBtn.onclick = () => finish(null);
        closeBtn.onclick = () => finish(null);
    });
}

async function refreshSwapHistory() {
    const list = document.getElementById('preset-history-list');
    if (!list) return;
    try {
        const history = await invoke('get_swap_history');
        if (!history.length) {
            list.innerHTML = '<div class="backup-empty">Nothing here yet. Swap something and it shows up.</div>';
            return;
        }
        const kindText = {
            swap: 'Swapped',
            restore: 'Restored',
            preset_apply: 'Used preset',
            preset_save: 'Saved preset',
            map_install: 'Loaded map',
            map_restore: 'Restored map',
            random: 'Random car',
            reswap: 'Re-applied all swaps',
        };
        const when = (iso) => {
            try {
                const d = new Date(iso);
                const today = new Date().toDateString() === d.toDateString();
                const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
                return today ? time : `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${time}`;
            } catch { return ''; }
        };
        list.innerHTML = history.slice().reverse().map(h => {
            const what = (h.swaps || []).map(s => `${s.owned_name} is now ${s.wanted_name}`).join(', ');
            const line = what || h.note || '';
            const action = kindText[h.kind] || h.kind;
            return `<div class="backup-item" style="display:flex;align-items:baseline;gap:10px;padding:8px 12px;font-size:13px;">
                <span style="color:var(--accent-blue);font-weight:600;white-space:nowrap;">${escHtml(action)}</span>
                <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escHtml(line)}</span>
                <span style="color:var(--muted);white-space:nowrap;font-size:12px;">${escHtml(when(h.at))}</span>
            </div>`;
        }).join('');
    } catch (e) {
        list.innerHTML = `<div class="backup-empty">History unavailable: ${escHtml(String(e))}</div>`;
    }
}

function wirePresetsUI() {
    const saveBtn = document.getElementById('preset-save-btn');
    if (saveBtn && saveBtn.dataset.wired !== '1') {
        saveBtn.dataset.wired = '1';
        saveBtn.onclick = async () => {
            const name = await appDialog({ title: 'Save preset', message: 'Preset name:', input: 'My preset', okLabel: 'Save' });
            if (!name) return;
            try {
                await invoke('save_preset', { name: name.trim() });
                showToast(`Preset <strong>${escHtml(name.trim())}</strong> saved`, 'success');
                refreshPresets();
            } catch (e) {
                showToast(String(e), 'error');
            }
        };
    }
    document.getElementById('preset-import-btn')?.addEventListener('click', async () => {
        const code = await appDialog({ title: 'Import preset', message: 'Paste a preset code:', input: '', okLabel: 'Next' });
        if (!code) return;
        let preview;
        try {
            preview = await invoke('peek_preset_code', { code: code.trim() });
        } catch (e) {
            showToast(String(e), 'error');
            return;
        }

        await openPresetPreviewModal(preview, { isImport: true, code: code.trim() });
    });
    document.getElementById('preset-random-btn')?.addEventListener('click', async () => {
        try {
            updateStatus('Rolling random car...', false);
            const swaps = await invoke('get_swaps').catch(() => []);
            const ownedIds = (swaps || []).map(s => s.owned_id);

            const plan = await invoke('random_swap_plan', { ownedIds });
            const previewItems = plan.map(p => `• ${p.wanted_name} (${p.owned_name})`).join('\n');
            if (!(await askConfirm(`Randomize car with ${plan.length} item categories?\n\n${previewItems}\n\nThis loadout will also be saved to your Presets.`, 'Randomize Car'))) {
                updateStatus('bitsfdb', false);
                return;
            }

            showProgress(true, 40);
            const results = await invoke('apply_swap_plan', { plan });
            showProgress(true, 80);
            const fails = results.filter(r => r.startsWith('FAIL'));

            let presetName = 'Random Car';
            try {
                const existing = await invoke('get_presets').catch(() => []);
                let num = 1;
                while (existing.some(p => p.name === `Random Car ${num}`)) {
                    num++;
                }
                presetName = `Random Car ${num}`;
                await invoke('save_preset', { name: presetName });
                refreshPresets();
            } catch (err) {
                console.warn('Auto-save random car preset:', err);
            }

            showProgress(true, 100);
            if (fails.length === 0) {
                showToast(`🎲 Random car applied and saved as <strong>${escHtml(presetName)}</strong>!`, 'success');
            } else {
                showToast(`Random car applied with ${fails.length} error(s) and saved as <strong>${escHtml(presetName)}</strong>`, 'warning');
            }
            await refreshBackups();
        } catch (e) {
            showToast(String(e), 'error');
        } finally {
            setTimeout(() => { showProgress(false); updateStatus('bitsfdb', false); }, 1500);
        }
    });
    document.getElementById('preset-history-clear')?.addEventListener('click', async () => {
        try {
            await invoke('clear_swap_history');
            refreshSwapHistory();
        } catch (e) {
            showToast(String(e), 'error');
        }
    });
}

async function refreshBackups() {
    if (!backupContainer) return;
    wireReswapButton();
    backupContainer.innerHTML = '<div class="backup-empty">Scanning for backups...</div>';
    try {
        const backups = await invoke('get_backups');

        try {
            const swaps = await invoke('get_swaps');
            const reswapBtn = document.getElementById('reswap-btn');
            if (reswapBtn) reswapBtn.disabled = (!swaps || swaps.length === 0);
        } catch { }

        if (backups.length === 0) {
            backupContainer.innerHTML = '<div class="backup-empty">No active modifications detected. Your files are clean.</div>';
            return;
        }
        backupContainer.innerHTML = '';
        backups.forEach((file, i) => {
            const div = document.createElement('div');
            div.className = 'backup-item';
            let pImg = file.image_url || '';
            if (!pImg && items && items.length > 0) {
                const fileName = file.path.split(/[/\\]/).pop();
                const cleanName = fileName.toLowerCase().replace('.bak', '').replace('.upk', '');
                const matched = items.find(i => {
                    const dbPkg = (i.asset_package || '').toLowerCase().replace('.upk', '');
                    if (!dbPkg || dbPkg === 'none') return false;
                    return dbPkg === cleanName || (dbPkg.length > 4 && (cleanName.includes(dbPkg) || dbPkg.includes(cleanName)));
                });
                if (matched && matched.image_url) {
                    pImg = matched.image_url;
                }
            }
            const swapLabel = file.swap_from && file.swap_to
                ? `${escHtml(file.swap_from)} is now ${escHtml(file.swap_to)}`
                : '';
            const thumb = (src) => src
                ? `<img src="${escHtml(src)}" class="flyout-img" style="width: 44px; height: 44px; border-radius: 6px; object-fit: contain; background: rgba(0,0,0,0.2);" onerror="this.style.display='none'" />`
                : '';
            div.innerHTML = `
                <div style="display: flex; align-items: center; gap: 12px; flex: 1; min-width: 0;">
                    ${thumb(pImg)}
                    ${file.swap_to_image ? thumb(file.swap_to_image) : ''}
                    <div style="min-width:0;">
                        <div class="backup-name">${escHtml(file.name)}</div>
                        ${swapLabel ? `<div class="backup-date" style="color:var(--accent-blue);">${swapLabel}</div>` : `<div class="backup-date">Modified Product</div>`}
                    </div>
                </div>
                <div class="restore-mini-btn" title="Restore this file" data-restore-index="${i}" style="display:flex;align-items:center;gap:6px;padding:8px 14px;border-radius:6px;background:var(--bg-secondary);border:1px solid var(--border);cursor:pointer;color:var(--text);font-size:12px;white-space:nowrap;flex-shrink:0;">
                    ${RESTORE_SVG}
                    <span>Restore</span>
                </div>`;
            div.querySelector('.restore-mini-btn').onclick = (e) => {
                e.stopPropagation();
                restoreSingle(file.path);
            };
            backupContainer.appendChild(div);
        });
    } catch (err) {
        console.error(err);
        backupContainer.innerHTML = '<div class="backup-empty backup-empty-error">Failed to retrieve backup list.</div>';
        try { await invoke('append_launch_log', { message: `ui: get_backups failed: ${String(err)}` }); } catch {}
    }
}

async function restoreSingle(path) {
    if (isAppLoading()) return;
    try {
        updateStatus('Restoring...', false);
        await invoke('restore_single_backup', { path });
        updateStatus('Restored', false);
        refreshBackups();
        setTimeout(() => updateStatus('bitsfdb', false), 2000);
    } catch (err) {
        updateStatus('Error', true);
        showToast(String(err), 'error');
    }
}

function updateStatus(text, isError = false) {
    if (!statusText) return;
    statusText.textContent = text;
    statusText.style.color = isError ? 'var(--danger)' : 'var(--text-secondary)';
}

function showProgress(show, percent = 0) {
    if (!progressBarContainer) return;
    if (show) {
        progressBarContainer.classList.remove('hidden');
        progressFill.style.width = `${percent}%`;
    } else {
        progressBarContainer.classList.add('hidden');
    }
}

function setupSearch(input, resultsDiv, selectionHandler) {
    input.addEventListener('input', (e) => {
        const term = e.target.value.toLowerCase();

        let lockCategory = currentCategory;
        if (input.id === 'wanted-search' && ownedItem) {
            lockCategory = (ownedItem.Slot || ownedItem.slot || 'All');
        }

        if (term.length < 2 && lockCategory === 'All') {
            resultsDiv.style.display = 'none';
            return;
        }

        const matches = items.filter(item => {
            const pName = (item.Product || item.product || '').toLowerCase();
            const pAsset = (item.AssetPackage || item.asset_package || '').toLowerCase();
            const pSlot = item.Slot || item.slot || '';

            const invalidTypes = ['series', 'crate', 'currency', 'premium', 'unknown'];
            if (invalidTypes.includes(normSlot(pSlot))) return false;

            const matchesTerm = term.length < 2 || pName.includes(term) || pAsset.includes(term);
            const matchesCat = lockCategory === 'All' || normSlot(pSlot) === normSlot(lockCategory);
            return matchesTerm && matchesCat;
        }).slice(0, 50);
        renderResults(matches, resultsDiv, selectionHandler);
    });
    input.addEventListener('focus', () => {
        let lockCategory = currentCategory;
        if (input.id === 'wanted-search' && ownedItem) {
            lockCategory = (ownedItem.Slot || ownedItem.slot || 'All');
        }

        if (lockCategory !== 'All' && input.value === '') {
            const matches = items.filter(item => {
                return normSlot(item.Slot || item.slot) === normSlot(lockCategory);
            }).slice(0, 50);
            renderResults(matches, resultsDiv, selectionHandler);
        }
    });
    document.addEventListener('click', (e) => {
        if (!input.contains(e.target) && !resultsDiv.contains(e.target)) {
            resultsDiv.style.display = 'none';
        }
    });
}

function renderResults(matches, resultsDiv, selectionHandler) {
    resultsDiv.innerHTML = '';
    if (matches.length === 0) {
        resultsDiv.style.display = 'none';
        return;
    }
    matches.forEach(item => {
        const div = document.createElement('div');
        div.className = 'flyout-row';
        const pName = item.Product || item.product || 'Unknown';
        const pSlot = item.Slot || item.slot || '';
        const pId = item.ID ?? item.id;
        const pImg = item.image_url || item.src || '';

        div.innerHTML = `
            ${pImg ? `<img src="${escHtml(pImg)}" class="flyout-img" onerror="this.style.display='none';this.nextElementSibling.style.display='flex'" /><div class="flyout-img" style="display:none;align-items:center;justify-content:center;"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#6b7280" stroke-width="1.5"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg></div>` : '<div class="flyout-img" style="display:flex;align-items:center;justify-content:center;"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#6b7280" stroke-width="1.5"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg></div>'}
            <div class="flyout-info">
                <span class="item-name">${escHtml(pName)}</span>
                <span style="font-size: 10px; color: var(--text-secondary)">${escHtml(pSlot)}${pId != null ? ` · <span style="color:#5b8cff">ID ${escHtml(String(pId))}</span>` : ''}</span>
            </div>
        `;
        div.onclick = () => {
            selectionHandler(item);
            resultsDiv.style.display = 'none';
        };
        resultsDiv.appendChild(div);
    });
    resultsDiv.style.display = 'block';
}

function normSlot(s) { return String(s || '').toLowerCase().replace(/[\s_-]+/g, ''); }

const UNPAINTABLE_SLOTS = new Set([
    'playeranthem', 'anthem',
    'playertitle', 'title',
    'crate', 'blueprint', 'currency',
    'engineaudio',
    'paintfinish',
    'avatarborder', 'avatar',
]);

const PAINT_HINT_UNPAINTABLE = "Unavailable — paint swaps cause items to become invisible in-game. Coming in a later update.";

function coercePaintableFlag(value) {
    if (value == null || value === '') return null;
    if (typeof value === 'boolean') return value;
    if (typeof value === 'number') return value !== 0;
    if (Array.isArray(value)) return value.length > 0;
    if (typeof value === 'object') {
        if ('paintable' in value) return coercePaintableFlag(value.paintable);
        if ('enabled' in value) return coercePaintableFlag(value.enabled);
        return null;
    }
    const s = String(value).trim().toLowerCase();
    if (['true', 'yes', '1', 'paintable'].includes(s)) return true;
    if (['false', 'no', '0', 'unpaintable', 'none'].includes(s)) return false;
    return null;
}

function itemAttrEntries(item) {
    const raw = item?.Attributes || item?.attributes;
    if (!raw) return [];
    if (Array.isArray(raw)) return raw;
    if (typeof raw === 'object') {
        return Object.entries(raw).map(([key, value]) => ({ key, value }));
    }
    return [];
}

function attrKey(entry) {
    return String(entry?.Key || entry?.key || entry?.Name || entry?.name || '').toLowerCase();
}

function attrValue(entry) {
    return entry?.Value ?? entry?.value;
}

function itemIsPaintable(item) {
    if (!item) return false;

    const explicitKeys = ['paintable', 'Paintable', 'paints', 'Paints'];
    for (const k of explicitKeys) {
        if (item[k] !== undefined && item[k] !== null && item[k] !== '') {
            const flag = coercePaintableFlag(item[k]);
            if (flag !== null) return flag;
        }
    }
    const paintField = item.paint ?? item.Paint;
    if (typeof paintField === 'boolean' || typeof paintField === 'number' || Array.isArray(paintField)) {
        const flag = coercePaintableFlag(paintField);
        if (flag !== null) return flag;
    }

    const attrs = itemAttrEntries(item);
    for (const entry of attrs) {
        const k = attrKey(entry);
        if (k === 'paintable' || k === 'painted' || k === 'paint') {
            const flag = coercePaintableFlag(attrValue(entry));
            if (flag !== null) return flag;
        }
    }

    return false;
}

function findItemByProductId(productId) {
    const n = Number(productId);
    if (!n || !Array.isArray(items)) return null;
    return items.find((it) => Number(it.ID ?? it.id) === n) || null;
}

function resetPaintToNone(swatchId, selectId, selectedLabelId) {
    const wrap = document.getElementById(swatchId);
    const select = document.getElementById(selectId);
    const selectedEl = document.getElementById(selectedLabelId);
    if (select) select.value = '0';
    wrap?.querySelectorAll('.paint-swatch').forEach((btn) => {
        const on = btn.dataset.paint === '0';
        btn.classList.toggle('is-active', on);
        btn.setAttribute('aria-checked', on ? 'true' : 'false');
    });
    if (selectedEl) selectedEl.textContent = paintLabel(0);
}

function setPaintBlockEnabled(opts) {
    const {
        blockId, swatchId, selectId, selectedLabelId, hintId,
        enabled, defaultHint, hideHintWhenEnabled,
    } = opts;
    const block = document.getElementById(blockId);
    const wrap = document.getElementById(swatchId);
    const hint = hintId ? document.getElementById(hintId) : null;
    if (block) {
        block.classList.toggle('is-disabled', !enabled);
        block.setAttribute('aria-disabled', enabled ? 'false' : 'true');
    }
    wrap?.querySelectorAll('.paint-swatch').forEach((btn) => {
        btn.disabled = !enabled;
        btn.tabIndex = enabled ? 0 : -1;
    });
    if (!enabled) {
        resetPaintToNone(swatchId, selectId, selectedLabelId);
        if (hint) {
            hint.textContent = PAINT_HINT_UNPAINTABLE;
            hint.hidden = false;
        }
        return;
    }
    if (hint) {
        if (hideHintWhenEnabled) {
            hint.hidden = true;
        } else if (defaultHint) {
            hint.textContent = defaultHint;
            hint.hidden = false;
        }
    }
}

function syncSwapPaintUi() {

    const enabled = false;
    setPaintBlockEnabled({
        blockId: 'swap-paint-block',
        swatchId: 'swap-paint-swatches',
        selectId: 'swap-paint',
        selectedLabelId: 'swap-paint-selected',
        hintId: 'swap-paint-hint',
        enabled,
        hideHintWhenEnabled: true,
    });
}

function validate() {
    syncSwapPaintUi();
    if (!applyBtn) return;
    const oSlot = ownedItem ? normSlot(ownedItem.Slot || ownedItem.slot) : '';
    const wSlot = wantedItem ? normSlot(wantedItem.Slot || wantedItem.slot) : '';
    const typesMatch = !ownedItem || !wantedItem || oSlot === wSlot;
    applyBtn.disabled = appLoading || swapBusy || !(ownedItem && wantedItem && typesMatch);
}

async function refreshSwapRlHint() {
    const hint = document.getElementById('swap-rl-hint');
    if (!hint) return;
    try {
        hint.hidden = !(await invoke('is_rocket_league_running'));
    } catch {
        hint.hidden = true;
    }
}

(function wireKillRl() {
    document.getElementById('kill-rl-btn')?.addEventListener('click', async (e) => {
        e.preventDefault();
        const btn = e.currentTarget;
        btn.textContent = 'Stopping...';
        btn.style.pointerEvents = 'none';
        try {
            const result = await invoke('kill_rocket_league');
            showToast(result || 'Rocket League closed', 'success');
            setTimeout(() => refreshSwapRlHint(), 1500);
        } catch (err) {
            showToast(String(err), 'error');
            btn.textContent = 'close it now';
            btn.style.pointerEvents = '';
        }
    });
})();

let rlToastShown = false;
let rlRunningPoll = null;
function startRlRunningPoll() {
    if (rlRunningPoll) return;
    rlRunningPoll = setInterval(async () => {
        if (isAppLoading()) return;
        try {
            const running = await invoke('is_rocket_league_running');
            if (running && !rlToastShown) {
                rlToastShown = true;
                showRlRunningToast();
            } else if (!running) {
                rlToastShown = false;
            }
        } catch {}
    }, 5000);
}
function showRlRunningToast() {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const toast = document.createElement('div');
    toast.className = 'toast warning';
    toast.id = 'rl-running-toast';
    toast.innerHTML = `
        <div class="toast-content">
            <div style="margin-bottom:6px;font-weight:600;">Rocket League is running</div>
            <div style="font-size:12px;color:var(--text-secondary);margin-bottom:8px;">Close it before swapping or restoring items.</div>
            <a href="#" id="toast-kill-rl-btn" style="font-weight:700;color:#fff;text-decoration:underline;cursor:pointer;">Close Rocket League</a>
        </div>
    `;
    container.appendChild(toast);
    toast.querySelector('#toast-kill-rl-btn')?.addEventListener('click', async (e) => {
        e.preventDefault();
        const link = e.currentTarget;
        link.textContent = 'Stopping...';
        link.style.pointerEvents = 'none';
        try {
            const result = await invoke('kill_rocket_league');
            showToast(result || 'Rocket League closed', 'success');
            toast.remove();
            rlToastShown = false;
        } catch (err) {
            showToast(String(err), 'error');
            link.textContent = 'Close Rocket League';
            link.style.pointerEvents = '';
        }
    });
    setTimeout(() => {
        if (toast.parentNode) {
            toast.style.animation = 'toastSlideOut 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards';
            setTimeout(() => toast.remove(), 300);
            rlToastShown = false;
        }
    }, 5000);
}

async function openSettingsForPath() {
    const cfg = await invoke('get_config').catch(() => ({ game_dir: '' }));
    document.getElementById('game-dir').value = cfg.game_dir || '';
    document.getElementById('settings-modal').classList.add('active');
    const btn = document.getElementById('autodetect-dir');
    btn.classList.add('path-btn-highlight');
    setTimeout(() => btn.classList.remove('path-btn-highlight'), 2000);
    invoke('detect_game_dir').then(installs => {
        if (installs && installs.length > 1) showInstallChooser(installs);
    }).catch(() => {});
}

async function handleApply() {
    if (isAppLoading() || swapBusy || !applyBtn) return;
    swapBusy = true;
    applyBtn.disabled = true;
    let interval;
    try {
        await refreshSwapRlHint();
        if (!ownedItem || !wantedItem) {
            showToast('Select an owned item and a target asset first.', 'error');
            return;
        }
        updateStatus('Please Wait...', false);
        showProgress(true, 15);
        let p = 15;
        interval = setInterval(() => { if (p < 85) p += 5; showProgress(true, p); }, 400);
        const ownedId = (ownedItem.ID !== undefined ? ownedItem.ID : ownedItem.id).toString();
        const wantedId = (wantedItem.ID !== undefined ? wantedItem.ID : wantedItem.id).toString();
        let paintId = Number(document.getElementById('swap-paint')?.value || 0);
        if (!itemIsPaintable(wantedItem)) paintId = 0;
        const swapResult = await invoke('apply_swap', { ownedId, wantedId, paintId });
        clearInterval(interval);
        interval = null;
        showProgress(true, 100);
        updateStatus('Swap Complete', false);
        const ownedName = ownedItem.product || ownedItem.Product || 'item';
        const wantedName = wantedItem.product || wantedItem.Product || 'item';
        const paintName = paintLabel(paintId);
        const paintBit = paintId > 0 ? ` (${escHtml(paintName)})` : '';
        showToast(`Swapped <strong>${escHtml(ownedName)}</strong> → <strong>${escHtml(wantedName)}</strong>${paintBit}`, 'success');

        if (swapResult && swapResult.includes && swapResult.includes('Warnings:')) {
            const warningPart = swapResult.split('Warnings:\n')[1];
            if (warningPart) {
                showToast(`${escHtml(warningPart.trim())}`, 'warning');
            }
        }
        setTimeout(() => { showProgress(false); updateStatus('bitsfdb', false); }, 3000);
    } catch (err) {
        if (interval) clearInterval(interval);
        updateStatus('Swap Failed', true);
        showProgress(false);
        const msg = String(err);
        if (msg.includes('Game directory not set') || msg.includes('Game directory not configured') || msg.includes('Game directory not valid')) {
            showGameDirToast();
        } else {
            showToast(msg, 'error');
        }
        console.error(err);
        invoke('report_diagnostic', { payload: {
            event:     'swap_fail',
            context:   'handleApply',
            message:   String(err),
            backtrace: err?.stack ?? null,
            owned_id:  ownedItem ? String(ownedItem.id ?? ownedItem.ID ?? '') : null,
            wanted_id: wantedItem ? String(wantedItem.id ?? wantedItem.ID ?? '') : null,
        }}).catch(() => {});
    } finally {
        if (interval) clearInterval(interval);
        swapBusy = false;
        validate();
    }
}

async function handleRestore() {
    if (isAppLoading()) return;
    const restoreBtn = document.getElementById('restore-btn');
    if (restoreBtn?.dataset.busy === '1') return;
    if (restoreBtn) {
        restoreBtn.dataset.busy = '1';
        restoreBtn.disabled = true;
    }
    try {
        await refreshSwapRlHint();
        updateStatus('Running Restoration...', false);
        const result = await invoke('restore_backups');
        updateStatus(result, false);
        refreshBackups();
        setTimeout(() => updateStatus('bitsfdb', false), 3000);
    } catch (err) {
        updateStatus('Restore Failed', true);
        const msg = String(err);
        if (msg.includes('Game directory not set') || msg.includes('Game directory not configured') || msg.includes('game_dir')) {
            showGameDirToast();
        } else {
            showToast(`Restore Error: ${msg}`, 'error');
        }
        console.error(err);
    } finally {
        if (restoreBtn) {
            restoreBtn.disabled = false;
            delete restoreBtn.dataset.busy;
        }
    }
}

async function handleSaveSettings() {
    const dir = document.getElementById('game-dir').value.trim();
    const input = document.getElementById('game-dir');

    if (dir) {
        try {
            const resolved = await invoke('validate_game_dir', { path: dir });
            if (resolved && resolved !== dir) {
                input.value = resolved;
            }
        } catch (err) {
            input.classList.add('input-shake');
            setTimeout(() => input.classList.remove('input-shake'), 600);
            showToast(String(err), 'error');
            return;
        }
    }

    const existing = await invoke('get_config').catch(() => ({}));
    const savedDir = await invoke('save_config', { config: { ...existing, game_dir: input.value.trim() } })
        .catch(e => { console.warn('Save config failed:', e); return input.value.trim(); });
    if (savedDir) input.value = savedDir;
    document.getElementById('settings-modal').classList.remove('active');
    document.getElementById('install-chooser').style.display = 'none';
    showToast(dir ? 'Settings saved' : 'Game path cleared', 'success');
    refreshPaletteStatus();
}

async function handleCancelSettings() {
    const existing = await invoke('get_config').catch(() => ({ game_dir: '' }));
    document.getElementById('game-dir').value = existing.game_dir || '';
    document.getElementById('settings-modal').classList.remove('active');
    document.getElementById('install-chooser').style.display = 'none';
}

async function handleAutoDetect() {
    const installs = await invoke('detect_game_dir').catch(() => []);
    if (installs.length === 0) {
        showToast('Could not auto-detect Rocket League. Please browse manually.', 'error');
    } else if (installs.length === 1) {
        document.getElementById('game-dir').value = installs[0].path;
        const existing = await invoke('get_config').catch(() => ({}));
        await invoke('save_config', { config: { ...existing, game_dir: installs[0].path } }).catch(() => {});
        showToast(`${installs[0].label} install detected and saved`, 'success');
        refreshPaletteStatus();
    } else {
        showInstallChooser(installs);
    }
}

function showInstallChooser(installs) {
    const container = document.getElementById('install-chooser');
    container.innerHTML = '';
    const label = document.createElement('p');
    label.style.cssText = 'font-size:13px;color:var(--text-secondary);margin-bottom:8px;';
    label.textContent = 'Multiple installs found - pick one:';
    container.appendChild(label);
    installs.forEach(install => {
        const btn = document.createElement('button');
        btn.className = 'chooser-btn';
        btn.innerHTML = `<strong>${escHtml(install.label)}</strong><span>${escHtml(install.path)}</span>`;
        btn.onclick = async () => {
            document.getElementById('game-dir').value = install.path;
            const existing = await invoke('get_config').catch(() => ({}));
            await invoke('save_config', { config: { ...existing, game_dir: install.path } }).catch(() => {});
            container.innerHTML = '';
            container.style.display = 'none';
            showToast(`${install.label} selected and saved`, 'success');
            refreshPaletteStatus();
        };
        container.appendChild(btn);
    });
    container.style.display = 'block';
}

async function handleBrowse() {
    const dir = await open({ directory: true, multiple: false, title: 'Select Rocket League CookedPCConsole folder' });
    if (dir) {
        let finalDir = dir;
        try {
            const resolved = await invoke('validate_game_dir', { path: dir });
            if (resolved) finalDir = resolved;
        } catch (_) {}
        document.getElementById('game-dir').value = finalDir;
        const existing = await invoke('get_config').catch(() => ({}));
        await invoke('save_config', { config: { ...existing, game_dir: finalDir } }).catch(() => {});
        showToast('Game path selected and saved', 'success');
        refreshPaletteStatus();
    }
}

async function checkForUpdates() {

    try {
        const version = await invoke('check_for_updates');
        if (!version) {

            try {
                const current = await window.__TAURI__.app.getVersion();
                const res = await fetch('https://api.github.com/repos/bitsfdb/VelocityRL/releases/latest');
                if (!res.ok) return;
                const data = await res.json();
                const latest = (data.tag_name || '').replace(/^v/, '');
                if (!latest || latest === current) return;
                invoke('append_launch_log', { message: `updater: github fallback sees v${latest}` }).catch(() => {});
                const url = escHtml(data.html_url || 'https://github.com/bitsfdb/VelocityRL/releases/latest');
                showToast(
                    `Update v${escHtml(latest)} available - <a href="#" class="toast-link" onclick="event.preventDefault(); window.__TAURI__.core.invoke('plugin:shell|open', { path: '${url}' })">Download</a>`,
                    'warning'
                );
            } catch (_) {}
            return;
        }
        const toast = document.createElement('div');
        toast.className = 'toast warning';
        toast.innerHTML = `<div class="toast-content">Update v${escHtml(version)} available - <a href="#" class="toast-link" id="install-update-link">Install Now</a></div>`;
        document.getElementById('toast-container')?.appendChild(toast);
        document.getElementById('install-update-link')?.addEventListener('click', async (e) => {
            e.preventDefault();
            toast.remove();
            const progToast = document.createElement('div');
            progToast.className = 'toast warning';
            progToast.innerHTML = `<div class="toast-content">Downloading update v${escHtml(version)}... <span id="update-progress-pct">0%</span></div>
                <div style="height:4px;background:rgba(255,255,255,.15);border-radius:2px;margin-top:6px;overflow:hidden;">
                    <div id="update-progress-fill" style="height:100%;width:0%;background:currentColor;border-radius:2px;transition:width .3s;"></div>
                </div>`;
            document.getElementById('toast-container')?.appendChild(progToast);

            let failed = false;
            try {
                const { listen } = window.__TAURI__.event;
                const unProgress = await listen('updater://progress', (ev) => {
                    const { percent } = ev.payload || {};
                    const fill = document.getElementById('update-progress-fill');
                    const pct = document.getElementById('update-progress-pct');
                    if (fill && typeof percent === 'number') fill.style.width = `${percent}%`;
                    if (pct && typeof percent === 'number') pct.textContent = `${percent}%`;
                });
                const unFail = await listen('updater://failed', () => {
                    failed = true;
                    progToast.remove();
                });
                try {
                    await invoke('install_update');
                    unProgress();
                    unFail();
                    if (failed) return;
                    progToast.remove();
                    showToast('Update installed! Restarting...', 'success');
                    setTimeout(() => window.__TAURI__.process.relaunch(), 2000);
                } catch (err) {
                    unProgress();
                    unFail();
                    progToast.remove();
                    if (!failed) showToast(`Update failed: ${escHtml(String(err))}`, 'error');
                }
            } catch (_) {

                try {
                    await invoke('install_update');
                    showToast('Update installed! Restarting...', 'success');
                    setTimeout(() => window.__TAURI__.process.relaunch(), 2000);
                } catch (err) {
                    showToast(`Update failed: ${escHtml(String(err))}`, 'error');
                }
            }
        });
    } catch (err) {
        invoke('append_launch_log', { message: `updater: unexpected invoke error: ${err}` }).catch(() => {});
    }
}

async function refreshDevPanel() {
    try {
        const cfg = await invoke('get_config').catch(() => ({}));
        const gameDirInput = document.getElementById('dev-game-dir-input');
        if (gameDirInput && !gameDirInput.dataset.wired) {
            gameDirInput.value = cfg.game_dir || '';
            gameDirInput.dataset.wired = '1';
            gameDirInput.addEventListener('change', async () => {
                const c = await invoke('get_config').catch(() => ({}));
                await invoke('save_config', { config: { ...c, game_dir: gameDirInput.value.trim() } });
                showToast('Game dir saved.', 'success');
            });
        }

        const proxyDirInput = document.getElementById('dev-proxy-dir-input');
        if (proxyDirInput) {

            if (document.activeElement !== proxyDirInput) {
                const override = await invoke('get_proxy_dir_override').catch(() => null);
                const status = await invoke('get_psynet_status').catch(() => ({}));
                proxyDirInput.value = override || status.proxy_dir || '';
            }
            if (!proxyDirInput.dataset.wired) {
                proxyDirInput.dataset.wired = '1';
                proxyDirInput.addEventListener('change', async () => {
                    const msg = document.getElementById('dev-action-msg');
                    try {
                        const saved = await invoke('save_proxy_dir', { path: proxyDirInput.value.trim() });
                        if (msg) msg.textContent = saved ? `Saved: ${saved}` : 'Proxy dir reset to auto-detect.';
                    } catch (e) {
                        if (msg) msg.textContent = 'Save failed: ' + e;
                    }
                });
            }
        }

        const logDirEl = document.getElementById('dev-log-dir');
        try {
            const logDir = await invoke('get_logs_dir');
            if (logDirEl) logDirEl.textContent = `Log dir: ${logDir}`;
        } catch (e) {
            if (logDirEl) logDirEl.textContent = `Log dir: error - ${e}`;
        }

        const proxyEl = document.getElementById('dev-proxy-status');
        try {
            const status = await invoke('get_psynet_status');
            if (proxyEl) proxyEl.textContent = `Proxy: ${status.running ? 'running' : 'stopped'}`;
        } catch (e) {
            if (proxyEl) proxyEl.textContent = `Proxy: error - ${e}`;
        }

        const pidEl = document.getElementById('dev-replay-player-id');
        const rnEl = document.getElementById('dev-replay-real-name');
        if (pidEl && rnEl) {
            try {
                const id = await invoke('get_replay_identity');
                if (document.activeElement !== pidEl) pidEl.value = id.player_id || '';
                if (document.activeElement !== rnEl) rnEl.value = id.real_name || '';
            } catch {}
        }

        const logEl = document.getElementById('dev-log-output');
        if (logEl) {
            const tail = await invoke('get_log_tail', { lines: 100 });
            logEl.textContent = tail;
            logEl.scrollTop = logEl.scrollHeight;
        }

        const cfgEl = document.getElementById('dev-proxy-config');
        if (cfgEl) {
            try {
                const raw = await invoke('get_psynet_config_json');
                const parsed = JSON.parse(raw);
                cfgEl.value = JSON.stringify(parsed, null, 2);
            } catch (e) {
                cfgEl.value = '// Error loading config: ' + e;
            }
        }
    } catch (err) {
        console.warn('Dev panel refresh failed:', err);
    }
}

function showGameDirToast() {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const toast = document.createElement('div');
    toast.className = 'toast error';
    toast.style.pointerEvents = 'auto';
    toast.innerHTML = `
        <div class="toast-content">
            <div style="margin-bottom:8px;font-weight:600;">Game path not set or incorrect</div>
            <div style="display:flex;gap:10px;align-items:center;flex-wrap:wrap;">
                <a href="#" id="gd-fix-btn" style="font-weight:700;color:#fff;text-decoration:underline;cursor:pointer;">Fix</a>
                <span style="color:var(--text-secondary);font-size:11px;">·</span>
                <a href="#" id="gd-auto-btn" style="font-size:12px;color:var(--accent-blue);text-decoration:underline;cursor:pointer;">Not sure what to pick? Click me!</a>
            </div>
        </div>
    `;
    container.appendChild(toast);
    toast.querySelector('#gd-fix-btn').addEventListener('click', (e) => {
        e.preventDefault();
        openSettingsForPath();
        toast.remove();
    });
    toast.querySelector('#gd-auto-btn').addEventListener('click', async (e) => {
        e.preventDefault();
        toast.remove();
        document.getElementById('settings-modal').classList.add('active');
        await handleAutoDetect();
    });
    setTimeout(() => {
        toast.style.animation = 'toastSlideOut 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards';
        setTimeout(() => toast.remove(), 300);
    }, 9000);
}

function semverGte(tag, min) {
    const parse = t => t.replace(/^v/, '').split('.').map(Number);
    const [a, b, c] = parse(tag);
    const [x, y, z] = parse(min);
    return a !== x ? a > x : b !== y ? b > y : c >= z;
}

function formatChangelogNotes(raw) {
    const match = (raw || '').match(/<!--\s*release notes\s*-->([\s\S]*?)<!--\s*\/release notes\s*-->/i);
    let text = match ? match[1].trim() : (raw || 'No notes.');

    text = text
        .split('\n')
        .filter(line => !/\b(MITM|PerCon|ws\.rlpp|api\.rlpp|openssl_trust|ClassPropertyConfig)\b/i.test(line))
        .join('\n')
        .trim() || 'No notes.';
    return text
        .split('\n')
        .map(line => {
            const escaped = escHtml(line).replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
            if (/^\+/.test(line)) return `<span class="cl-add">${escaped}</span>`;
            if (/^-/.test(line))  return `<span class="cl-remove">${escaped}</span>`;
            return escaped;
        })
        .join('\n');
}

const TITLE_SPOOF_KEY = 'velocityrl_title_spoof';
const LOGO_SPOOF_KEY = 'velocityrl_logo_spoof';
const BLOG_SPOOF_KEY = 'velocityrl_blog_spoof';
const FAKE_RANKS_KEY = 'velocityrl_fake_ranks';
const CAMERA_SPOOF_KEY = 'velocityrl_camera_spoof';
const DEFAULT_SEASON23_LOGO_URL = 'https://api.velocityrl.tech/thumbnails/rl_jpn.png';
const DEFAULT_BLOG_MOTD = 'Use VelocityRL';
const DEFAULT_CAMERA_LIMITS = {
    fov: { min: 60, max: 1000, interval: 1 },
    height: { min: 40, max: 1000, interval: 1 },
    distance: { min: 100, max: 1000, interval: 1 },
};
let titlesDb = { titles: [], categories: {} };
let titlesTabReady = false;
let psynetProxyRunning = false;
let spoofSaveInFlight = false;
let proxyEnsurePromise = null;
let closeGuardAttached = false;
let closeModalOpen = false;
let closeInProgress = false;
let donorPick = null;
let displayPick = null;
let titleSwaps = [];
let titleSwapsLoaded = false;

let userEditedCustomText = false;

function debounce(fn, ms) {
    let t = null;
    return (...args) => {
        clearTimeout(t);
        t = setTimeout(() => fn(...args), ms);
    };
}

function flashButtonLabel(el, feedback, ms = 1500, onDone) {
    if (!el || el.dataset.labelFlashing === '1') return;
    const original = el.dataset.originalLabel || el.textContent;
    el.dataset.originalLabel = original;
    el.dataset.labelFlashing = '1';
    el.textContent = feedback;
    const canDisable = 'disabled' in el;
    const wasDisabled = canDisable ? el.disabled : false;
    if (canDisable) el.disabled = true;
    el.style.pointerEvents = 'none';
    clearTimeout(el._labelFlashTimer);
    el._labelFlashTimer = setTimeout(() => {
        el.textContent = original;
        if (canDisable) el.disabled = wasDisabled;
        el.style.pointerEvents = '';
        delete el.dataset.labelFlashing;
        if (typeof onDone === 'function') onDone();
    }, ms);
}

function loadSavedSpoof() {
    let titles = {};
    try { titles = JSON.parse(localStorage.getItem(TITLE_SPOOF_KEY) || '{}'); } catch {  }
    return { ...titles };
}

function normalizeHex6(raw) {
    if (!raw || typeof raw !== 'string') return '';
    let s = raw.trim().replace(/^#/, '').toUpperCase();
    if (!/^[0-9A-F]{6}$/.test(s)) return '';
    return s;
}

function normalizeTitleColor(tc) {
    if (!tc || typeof tc !== 'object') return null;
    const color = normalizeHex6(tc.color || tc.Color || '');
    if (!color) return null;
    const glow = normalizeHex6(tc.glow_color || tc.GlowColor || tc.glow || '');

    if (color === 'FFFFFF' && (!glow || glow === 'FFFFFF')) return null;
    const out = { color };
    if (glow) out.glow_color = glow;
    return out;
}

function readTitleColorFromForm() {
    const on = !!document.getElementById('title-color-custom')?.checked;
    if (!on) return null;
    return normalizeTitleColor({
        color: document.getElementById('title-color-hex')?.value || '',
        glow_color: document.getElementById('title-glow-hex')?.value || '',
    });
}

function setTitleColorForm(tc) {
    const enable = document.getElementById('title-color-custom');
    const colorHex = document.getElementById('title-color-hex');
    const glowHex = document.getElementById('title-glow-hex');
    const colorPick = document.getElementById('title-color-picker');
    const glowPick = document.getElementById('title-glow-picker');
    const n = normalizeTitleColor(tc);
    if (enable) {
        enable.checked = !!n;
        syncNameSpoofSwitchAria(enable);
    }
    if (n) {
        if (colorHex) colorHex.value = n.color;
        if (colorPick) colorPick.value = `#${n.color}`;
        const glow = n.glow_color || n.color;
        if (glowHex) glowHex.value = n.glow_color || '';
        if (glowPick) glowPick.value = `#${glow}`;
    }
}

function wireTitleColorInputs() {
    const enable = document.getElementById('title-color-custom');
    const colorHex = document.getElementById('title-color-hex');
    const glowHex = document.getElementById('title-glow-hex');
    const colorPick = document.getElementById('title-color-picker');
    const glowPick = document.getElementById('title-glow-picker');
    if (!enable || enable.dataset.wired === '1') return;
    enable.dataset.wired = '1';
    enable.addEventListener('change', () => {
        syncNameSpoofSwitchAria(enable);
        updateTitlePreview();
    });
    const syncPickToHex = (pick, hexEl) => {
        pick?.addEventListener('input', () => {
            if (hexEl) hexEl.value = (pick.value || '').replace(/^#/, '').toUpperCase();
            if (enable && !enable.checked) {
                enable.checked = true;
                syncNameSpoofSwitchAria(enable);
            }
            updateTitlePreview();
        });
    };
    const syncHexToPick = (hexEl, pick) => {
        hexEl?.addEventListener('input', () => {
            const n = normalizeHex6(hexEl.value);
            if (n && pick) pick.value = `#${n}`;
            if (n && enable && !enable.checked) {
                enable.checked = true;
                syncNameSpoofSwitchAria(enable);
            }
            updateTitlePreview();
        });
    };
    syncPickToHex(colorPick, colorHex);
    syncPickToHex(glowPick, glowHex);
    syncHexToPick(colorHex, colorPick);
    syncHexToPick(glowHex, glowPick);
}

function normalizeSwapEntry(s) {
    if (!s || typeof s !== 'object') return null;
    const equip_title_id = String(s.equip_title_id || '').trim();
    if (!equip_title_id) return null;
    const entry = {
        equip_title_id,
        display_title_id: String(s.display_title_id || '').trim(),
        custom_text: String(s.custom_text || '').trim(),
        category: String(s.category || '').trim(),
    };
    const tc = normalizeTitleColor(s.title_color);
    if (tc) entry.title_color = tc;
    return entry;
}

function migrateTitleSwaps(saved) {
    if (!saved || typeof saved !== 'object') return [];
    if (Array.isArray(saved.swaps)) {
        return saved.swaps.map(normalizeSwapEntry).filter(Boolean);
    }
    return [normalizeSwapEntry(saved)].filter(Boolean);
}

function loadTitleSwapsFromStorage() {
    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(TITLE_SPOOF_KEY) || '{}'); } catch {  }
    titleSwaps = migrateTitleSwaps(saved);
    titleSwapsLoaded = true;
}

function ensureTitleSwapsLoaded() {
    if (!titleSwapsLoaded) loadTitleSwapsFromStorage();
}

function pickerSwapEntry() {
    const custom = document.getElementById('title-custom-text')?.value?.trim() || '';
    let displayId = document.getElementById('title-display-id')?.value?.trim() || '';

    if (!displayId && custom) displayId = 'custom';
    const entry = normalizeSwapEntry({
        equip_title_id: document.getElementById('title-equip-id')?.value?.trim() || '',
        display_title_id: displayId,
        custom_text: custom,
        category: lookCategory(),
        title_color: readTitleColorFromForm(),
    });
    return entry;
}

function lookCategory() {
    if (displayPick) {
        return String(displayPick.category || displayPick.Category || '').trim();
    }
    const displayId = document.getElementById('title-display-id')?.value?.trim() || '';
    if (displayId && displayId !== 'custom') {
        const t = findTitleById(displayId);
        return String(t?.category || t?.Category || '').trim();
    }
    return '';
}

function titleSpoofPayload() {
    ensureTitleSwapsLoaded();
    const first = titleSwaps[0] || {};

    return {
        enabled: titleSwaps.length > 0,
        method: 'raw',
        swaps: titleSwaps.map((s) => ({ ...s })),
        equip_title_id: first.equip_title_id || '',
        display_title_id: first.display_title_id || '',
        custom_text: first.custom_text || '',
        category: first.category || '',
    };
}

function readLocalJson(key) {
    try { return JSON.parse(localStorage.getItem(key) || '{}'); } catch { return {}; }
}

function toolSliceFromDiskOrLocal(disk, key, field) {
    if (disk && disk[field] != null && typeof disk[field] === 'object') {
        return disk[field];
    }
    const local = readLocalJson(key);
    return local[field] || local;
}

function persistTitleSpoofLocal() {
    const payload = titleSpoofPayload();
    localStorage.setItem(TITLE_SPOOF_KEY, JSON.stringify({
        enabled: payload.enabled,
        method: 'raw',
        swaps: payload.swaps,
        equip_title_id: payload.equip_title_id,
        display_title_id: payload.display_title_id,
        custom_text: payload.custom_text,
        category: payload.category,
    }));
}

async function writeTitleSpoofConfig() {
    persistTitleSpoofLocal();
    await invoke('save_psynet_spoof', { payload: titleSpoofPayload() });
}

async function hydrateSpoofToolsFromDisk() {
    let disk = {};
    try {
        disk = await invoke('get_psynet_spoof') || {};
    } catch (e) {
        invoke('append_launch_log', { message: `psynet: get_psynet_spoof failed: ${e}` }).catch(() => {});
        disk = {};
    }

    if (Array.isArray(disk.swaps) || disk.equip_title_id) {
        const titleSaved = {
            enabled: disk.enabled !== false && (Array.isArray(disk.swaps) ? disk.swaps.length > 0 : !!disk.equip_title_id),
            method: 'raw',
            swaps: Array.isArray(disk.swaps) ? disk.swaps : undefined,
            equip_title_id: disk.equip_title_id || '',
            display_title_id: disk.display_title_id || '',
            custom_text: disk.custom_text || '',
            category: disk.category || '',
        };
        localStorage.setItem(TITLE_SPOOF_KEY, JSON.stringify(titleSaved));
        titleSwaps = migrateTitleSwaps(titleSaved);
        titleSwapsLoaded = true;
    } else {
        loadTitleSwapsFromStorage();
    }

    {
        const fr = toolSliceFromDiskOrLocal(disk, FAKE_RANKS_KEY, 'fake_ranks');
        const fake_ranks = (fr && typeof fr === 'object' && ('enabled' in fr || fr.playlists || fr.reward_levels))
            ? clampFakeRanksRewardWins({ ...fr, reward_levels: fr.reward_levels ? { ...fr.reward_levels } : undefined })
            : { enabled: false, playlists: {} };
        localStorage.setItem(FAKE_RANKS_KEY, JSON.stringify({ fake_ranks }));
    }

    {
        const cam = toolSliceFromDiskOrLocal(disk, CAMERA_SPOOF_KEY, 'camera_spoof');
        const camera_spoof = (cam && typeof cam === 'object' && ('enabled' in cam || cam.fov))
            ? cam
            : {
                enabled: false,
                fov: { ...DEFAULT_CAMERA_LIMITS.fov },
                height: { ...DEFAULT_CAMERA_LIMITS.height },
                distance: { ...DEFAULT_CAMERA_LIMITS.distance },
            };
        localStorage.setItem(CAMERA_SPOOF_KEY, JSON.stringify({ camera_spoof }));
    }

    {
        const ls = toolSliceFromDiskOrLocal(disk, LOGO_SPOOF_KEY, 'logo_spoof');
        let logo_spoof;
        if (ls && typeof ls === 'object' && ('enabled' in ls || ls.logo_url != null)) {
            const enabled = !!ls.enabled;
            let logo_url = String(ls.logo_url || '').trim();

            if (enabled && !logo_url) logo_url = DEFAULT_SEASON23_LOGO_URL;
            logo_spoof = { enabled, logo_url };
        } else {
            logo_spoof = { enabled: false, logo_url: DEFAULT_SEASON23_LOGO_URL };
        }
        localStorage.setItem(LOGO_SPOOF_KEY, JSON.stringify({ logo_spoof }));
    }

    {
        const bs = toolSliceFromDiskOrLocal(disk, BLOG_SPOOF_KEY, 'blog_spoof');
        const blog_spoof = (bs && typeof bs === 'object' && ('enabled' in bs || bs.motd != null))
            ? { enabled: !!bs.enabled, motd: bs.motd || '' }
            : { enabled: false, motd: DEFAULT_BLOG_MOTD };
        localStorage.setItem(BLOG_SPOOF_KEY, JSON.stringify({ blog_spoof }));
    }

    applyHydratedToolsToUi();

    return payloadFromHydratedLocal();
}

function applyHydratedToolsToUi() {
    const fr = readLocalJson(FAKE_RANKS_KEY);
    if (document.getElementById('fake-ranks-enabled')) {
        loadFakeRanksFromSaved(fr);
    }
    const cam = readLocalJson(CAMERA_SPOOF_KEY);
    if (document.getElementById('camera-spoof-enabled')) {
        loadCameraFromSaved(cam);
    }
    const logo = readLocalJson(LOGO_SPOOF_KEY).logo_spoof || {};
    const logoEn = document.getElementById('logo-spoof-enabled');
    const logoUrl = document.getElementById('logo-spoof-url');
    if (logoEn) {
        logoEn.checked = !!logo.enabled;
        syncNameSpoofSwitchAria(logoEn);
    }
    if (logoUrl) logoUrl.value = logo.logo_url || DEFAULT_SEASON23_LOGO_URL;

    const blog = readLocalJson(BLOG_SPOOF_KEY).blog_spoof || {};
    const blogEn = document.getElementById('blog-spoof-enabled');
    const blogMotd = document.getElementById('blog-spoof-motd');
    if (blogEn) {
        blogEn.checked = !!blog.enabled;
        syncNameSpoofSwitchAria(blogEn);
    }
    if (blogMotd) blogMotd.value = blog.motd || DEFAULT_BLOG_MOTD;
}

function payloadFromHydratedLocal() {
    ensureTitleSwapsLoaded();
    const titles = titleSpoofPayload();
    const frRaw = readLocalJson(FAKE_RANKS_KEY).fake_ranks || { enabled: false, playlists: {} };
    const fr = clampFakeRanksRewardWins(
        frRaw && typeof frRaw === 'object'
            ? { ...frRaw, reward_levels: frRaw.reward_levels ? { ...frRaw.reward_levels } : undefined }
            : frRaw
    );
    const camera_spoof = readLocalJson(CAMERA_SPOOF_KEY).camera_spoof || {
        enabled: false,
        fov: { ...DEFAULT_CAMERA_LIMITS.fov },
        height: { ...DEFAULT_CAMERA_LIMITS.height },
        distance: { ...DEFAULT_CAMERA_LIMITS.distance },
    };
    const logo_spoof = readLocalJson(LOGO_SPOOF_KEY).logo_spoof || { enabled: false, logo_url: '' };
    const blog_spoof = readLocalJson(BLOG_SPOOF_KEY).blog_spoof || { enabled: false, motd: '' };
    return {
        ...titles,
        method: 'raw',
        fake_ranks: fr,
        camera_spoof,
        logo_spoof,
        blog_spoof,
    };
}

function preserveEnabledLogoBlogFromDisk(payload, disk) {
    const out = { ...payload };
    if (disk?.logo_spoof?.enabled) {
        const diskUrl = String(disk.logo_spoof.logo_url || '').trim();
        const outUrl = String(out.logo_spoof?.logo_url || '').trim();

        if (!out.logo_spoof?.enabled || !outUrl) {
            out.logo_spoof = {
                enabled: true,
                logo_url: diskUrl || outUrl || DEFAULT_SEASON23_LOGO_URL,
            };
            localStorage.setItem(LOGO_SPOOF_KEY, JSON.stringify({ logo_spoof: out.logo_spoof }));
        }
    }
    if (disk?.blog_spoof?.enabled && !out.blog_spoof?.enabled) {
        out.blog_spoof = {
            enabled: true,
            motd: disk.blog_spoof.motd || out.blog_spoof?.motd || '',
        };
        localStorage.setItem(BLOG_SPOOF_KEY, JSON.stringify({ blog_spoof: out.blog_spoof }));
    }
    return out;
}

async function anySpoofToolEnabled(payload) {
    const p = payload || payloadFromHydratedLocal();
    if (p.enabled && p.swaps?.length) return true;
    if (p.fake_ranks?.enabled) return true;
    if (p.camera_spoof?.enabled) return true;
    if (p.logo_spoof?.enabled) return true;
    if (p.blog_spoof?.enabled) return true;
    if (PALETTE_UI_DISABLED) return false;
    try {
        const pal = await invoke('get_palette_status');
        if (pal?.applied) return true;
    } catch {   }
    return false;
}

function titleLabel(id, fallbackText) {
    const t = findTitleById(id);
    const raw = fallbackText || t?.text || t?.Text || (id ? String(id).replace(/_/g, ' ') : '-');
    return formatTitleText(raw);
}

function titleTextShadow(glow) {
    if (!glow) return 'none';
    return `0 0 4px ${glow}, 0 0 10px ${glow}, 0 0 20px ${glow}cc, 0 0 36px ${glow}66`;
}

function titleColors(title) {
    const cat = titleCategoryId(title);
    let { color, glow } = cat ? categoryColors(cat) : { color: '#c8c8c8', glow: '' };
    if (title) {
        const pickColor = normalizeHexColor(title.color || title.Color || '');
        const pickGlow = normalizeHexColor(title.glow || title.GlowColor || title.glow_color || '');
        if (pickGlow && !glow) glow = pickGlow;
        if (pickColor && (!cat || color === '#c8c8c8')) color = pickColor;
    }
    return { color: color || '#c8c8c8', glow: glow || '', cat };
}

function titleInlineStyle(title) {
    const { color, glow } = titleColors(title);
    return `color:${color};text-shadow:${titleTextShadow(glow)}`;
}

function titleChipInlineStyle(catId) {
    return titleInlineStyle(catId ? { category: catId } : null);
}

function renderTitleSwapList() {
    ensureTitleSwapsLoaded();
    const list = document.getElementById('title-swap-list');
    const restoreAll = document.getElementById('title-restore-all-btn');
    if (!list) return;
    if (!titleSwaps.length) {
        list.innerHTML = '<div class="backup-empty">No title remaps yet. Pick a donor and a look (or custom text), then Add swap.</div>';
        if (restoreAll) restoreAll.hidden = true;
        return;
    }
    if (restoreAll) restoreAll.hidden = false;
    list.innerHTML = titleSwaps.map((s, i) => {
        const donor = titleLabel(s.equip_title_id);
        const look = s.custom_text || titleLabel(s.display_title_id);
        const donorTitle = findTitleById(s.equip_title_id);
        const donorStyle = titleInlineStyle(donorTitle);

        const lookTitle = (s.display_title_id && s.display_title_id !== 'custom')
            ? findTitleById(s.display_title_id)
            : null;

        let tc = s.title_color;
        if (tc && String(tc.color || '').toUpperCase() === 'FFFFFF'
            && String(tc.glow_color || tc.color || '').toUpperCase() === 'FFFFFF') {
            tc = null;
        }
        const lookStyle = titleInlineStyle(
            tc
                ? { Color: tc.color, GlowColor: tc.glow_color || '' }
                : (lookTitle
                    ? { ...lookTitle, category: s.category || lookTitle.category || lookTitle.Category || '' }
                    : (s.category ? { category: s.category } : null))
        );
        const metaIds = s.display_title_id && s.display_title_id !== 'custom'
            ? `${escHtml(s.equip_title_id)} → ${escHtml(s.display_title_id)}`
            : `${escHtml(s.equip_title_id)}${s.category ? ` · ${escHtml(s.category)}` : ''}`;
        return `<div class="backup-item" data-index="${i}">
            <div>
                <div class="backup-name title-swap-row-preview">
                    <span class="title-preview-chip title-preview-muted title-preview-chip-sm" style="${donorStyle}">${formatTitleHtml(donor)}</span>
                    <span class="title-swap-arrow">→</span>
                    <span class="title-preview-chip title-preview-chip-sm" style="${lookStyle}">${formatTitleHtml(look)}</span>
                </div>
                <div class="backup-date">${metaIds}</div>
            </div>
            <div class="restore-mini-btn" title="Restore this title" data-restore-index="${i}">Restore</div>
        </div>`;
    }).join('');
    list.querySelectorAll('[data-restore-index]').forEach((btn) => {
        btn.onclick = (e) => {
            e.stopPropagation();
            restoreTitleSwap(Number(btn.dataset.restoreIndex));
        };
    });
}

function setProxyUi(running) {
    psynetProxyRunning = !!running;
}

async function refreshProxyStatus() {
    try {
        const st = await invoke('get_psynet_status');
        setProxyUi(st.running);
    } catch {
        setProxyUi(false);
    }
}

async function autoStartPsyNetProxy() {
    let payload = payloadFromHydratedLocal();
    try {
        const disk = await invoke('get_psynet_spoof') || {};
        payload = preserveEnabledLogoBlogFromDisk(payload, disk);
        applyHydratedToolsToUi();
    } catch {   }
    try {

        await invoke('save_psynet_spoof', { payload });
    } catch (e) {
        invoke('append_launch_log', { message: `psynet: boot write spoof failed: ${e}` }).catch(() => {});
    }
    try {
        const st = await invoke('get_psynet_status');
        if (st.running) {
            setProxyUi(true);
            invoke('append_launch_log', { message: 'psynet: existing proxy healthy - boot config written (hot-reload), skip restart' }).catch(() => {});
            return;
        }
    } catch {   }
    try {
        showToast('Starting PsyNet proxy - approve UAC if prompted…', 'success');
        const st = await invoke('start_psynet_proxy', {});
        setProxyUi(st.running);
        if (st.running) {
            showToast('PsyNet proxy up. Launch Rocket League only after this toast.', 'success');
        }
    } catch (e) {
        setProxyUi(false);
        invoke('append_launch_log', { message: `psynet: auto-start failed: ${e}` }).catch(() => {});
        showToast(e, 'error');
    }
}

async function ensurePsyNetFromApp(reason) {
    if (proxyEnsurePromise) return proxyEnsurePromise;
    proxyEnsurePromise = (async () => {
        try {
            await refreshProxyStatus();
            if (psynetProxyRunning) {
                showToast(`${reason} saved — proxy running (hot-reload). Keep VelocityRL open.`, 'success');
                return true;
            }
            return false;
        } catch {
            setProxyUi(false);
            return false;
        } finally {
            proxyEnsurePromise = null;
        }
    })();
    return proxyEnsurePromise;
}

async function runToolSave(btn, reason, partialPayload, { enabled = true } = {}) {
    if (isAppLoading() || spoofSaveInFlight) return null;
    if (btn?.dataset.saving === '1' || btn?.dataset.labelFlashing === '1') return null;
    spoofSaveInFlight = true;
    if (btn) {
        btn.dataset.saving = '1';
        btn.disabled = true;
    }
    try {
        await invoke('save_psynet_spoof', {
            payload: { method: 'raw', ...partialPayload },
        });
        if (enabled) {
            await ensurePsyNetFromApp(reason);
        }
        return true;
    } finally {
        spoofSaveInFlight = false;
        if (btn) {
            btn.disabled = false;
            delete btn.dataset.saving;
        }
    }
}

function promptCloseModal() {
    const overlay = document.getElementById('close-psynet-modal');
    if (!overlay) return Promise.resolve('stay');
    return new Promise((resolve) => {
        let settled = false;
        const finish = (choice) => {
            if (settled) return;
            settled = true;
            document.removeEventListener('keydown', onKey);
            if (choice !== 'stay') {

                const choices = document.getElementById('close-psynet-choices');
                const shutting = document.getElementById('close-psynet-shutting-down');
                if (choices) choices.style.display = 'none';
                if (shutting) shutting.style.display = 'block';
                const title = document.getElementById('close-psynet-title');
                if (title) title.textContent = 'Shutting down…';

                overlay.onclick = null;
            } else {
                overlay.classList.remove('active');
            }
            resolve(choice);
        };
        const onKey = (e) => {
            if (e.key === 'Escape') finish('stay');
        };
        overlay.querySelectorAll('[data-close-choice]').forEach((btn) => {
            btn.onclick = () => finish(btn.dataset.closeChoice);
        });
        overlay.onclick = (e) => {
            if (e.target === overlay) finish('stay');
        };
        document.addEventListener('keydown', onKey);
        overlay.classList.add('active');
    });
}

async function stopProxyOnClose(revertHosts) {
    try {
        await Promise.race([
            invoke('stop_psynet_proxy', { revertHosts }),
            new Promise((_, reject) => setTimeout(() => reject(new Error('timeout')), 3000)),
        ]);
    } catch (e) {
        console.warn('stop_psynet_proxy on close:', e);
    }
}

function finishAppClose(appWindow, revertHosts) {
    closeInProgress = true;
    invoke('force_exit').catch(() => {
        try { appWindow.destroy(); } catch {}
    });
}

function attachCloseGuard() {
    if (closeGuardAttached) return;
    const winApi = window.__TAURI__?.window;
    if (!winApi?.getCurrentWindow) return;
    closeGuardAttached = true;
    const appWindow = winApi.getCurrentWindow();
    appWindow.onCloseRequested((event) => {
        event.preventDefault();
        finishAppClose(appWindow, true);
    });
}

function syncNameSpoofSwitchAria(el) {
    if (!el) return;
    el.setAttribute('aria-checked', el.checked ? 'true' : 'false');
}

const PAINT_NAMES = {
    0: 'None', 1: 'Crimson', 2: 'Lime', 3: 'Black', 4: 'Orange', 5: 'Sky Blue',
    6: 'Cobalt', 7: 'Saffron', 8: 'Grey', 9: 'Pink', 10: 'Forest Green',
    11: 'Purple', 12: 'Titanium White',
};

const RANK_ICON_CDN = 'https://trackercdn.com/cdn/tracker.gg/rocket-league/ranks/';
const RANK_PLAYLISTS = [
    { id: '10', label: 'Ranked Duel 1v1' },
    { id: '11', label: 'Ranked Doubles 2v2' },
    { id: '13', label: 'Ranked Standard 3v3' },
    { id: '27', label: 'Hoops' },
    { id: '28', label: 'Rumble' },
    { id: '29', label: 'Dropshot' },
    { id: '30', label: 'Snow Day' },
    { id: '34', label: 'Tournaments' },
    { id: '61', label: 'Heatseeker' },
    { id: '63', label: 'Knockout' },
];

const SEASON_REWARD_LEVELS = [
    { level: 0, name: 'Unranked' },
    { level: 1, name: 'Bronze' },
    { level: 2, name: 'Silver' },
    { level: 3, name: 'Gold' },
    { level: 4, name: 'Platinum' },
    { level: 5, name: 'Diamond' },
    { level: 6, name: 'Champion' },
    { level: 7, name: 'Grand Champion' },
    { level: 8, name: 'Supersonic Legend' },
];

function seasonRewardMeta(level) {
    return SEASON_REWARD_LEVELS.find((r) => r.level === level) || null;
}

function clampSeasonLevelWins(n) {
    const v = Number(n);
    if (!Number.isFinite(v)) return null;
    return Math.max(0, Math.min(10, Math.round(v)));
}

function clampFakeRanksRewardWins(fake_ranks) {
    if (!fake_ranks || typeof fake_ranks !== 'object') return fake_ranks;
    if (fake_ranks.reward_levels) {
        const wins = fake_ranks.reward_levels.season_level_wins;
        if (Number.isFinite(wins)) {
            const clamped = clampSeasonLevelWins(wins);
            if (clamped !== null) fake_ranks.reward_levels.season_level_wins = clamped;
        }
        if (Number.isFinite(fake_ranks.reward_levels.season_level)) {
            fake_ranks.reward_levels.season_level = Math.max(0, Math.min(8, Math.round(fake_ranks.reward_levels.season_level)));
        }
    }
    if (fake_ranks.playlists && typeof fake_ranks.playlists === 'object') {
        Object.keys(fake_ranks.playlists).forEach((pid) => {
            const ov = fake_ranks.playlists[pid];
            if (ov && typeof ov === 'object') {
                if (Number.isFinite(ov.display_mmr)) {
                    ov.display_mmr = Math.max(0, Math.min(3000, Math.round(ov.display_mmr)));
                    ov.mu = Number(((ov.display_mmr - 100) / 20).toFixed(4));
                }
                if (Number.isFinite(ov.tier)) {
                    ov.tier = Math.max(0, Math.min(22, Math.round(ov.tier)));
                }
            }
        });
    }
    return fake_ranks;
}

function buildSeasonRewardSelect() {
    const sel = document.getElementById('fake-ranks-season-level');
    if (!sel || sel.dataset.built === '1') return;
    sel.dataset.built = '1';
    const keep = sel.querySelector('option[value=""]');
    sel.innerHTML = '';
    if (keep) sel.appendChild(keep);
    else {
        const opt = document.createElement('option');
        opt.value = '';
        opt.textContent = 'Keep real';
        sel.appendChild(opt);
    }
    SEASON_REWARD_LEVELS.forEach(({ level, name }) => {
        const opt = document.createElement('option');
        opt.value = String(level);
        opt.textContent = name;
        sel.appendChild(opt);
    });
}

const RL_RANKS = [
    { tier: 0, name: 'Unranked', mmr: 0 },
    { tier: 1, name: 'Bronze I', mmr: 118 },
    { tier: 2, name: 'Bronze II', mmr: 218 },
    { tier: 3, name: 'Bronze III', mmr: 298 },
    { tier: 4, name: 'Silver I', mmr: 398 },
    { tier: 5, name: 'Silver II', mmr: 498 },
    { tier: 6, name: 'Silver III', mmr: 598 },
    { tier: 7, name: 'Gold I', mmr: 698 },
    { tier: 8, name: 'Gold II', mmr: 798 },
    { tier: 9, name: 'Gold III', mmr: 898 },
    { tier: 10, name: 'Platinum I', mmr: 998 },
    { tier: 11, name: 'Platinum II', mmr: 1098 },
    { tier: 12, name: 'Platinum III', mmr: 1198 },
    { tier: 13, name: 'Diamond I', mmr: 1298 },
    { tier: 14, name: 'Diamond II', mmr: 1398 },
    { tier: 15, name: 'Diamond III', mmr: 1498 },
    { tier: 16, name: 'Champion I', mmr: 1598 },
    { tier: 17, name: 'Champion II', mmr: 1698 },
    { tier: 18, name: 'Champion III', mmr: 1798 },
    { tier: 19, name: 'Grand Champion I', mmr: 1848 },
    { tier: 20, name: 'Grand Champion II', mmr: 1898 },
    { tier: 21, name: 'Grand Champion III', mmr: 1948 },
    { tier: 22, name: 'Supersonic Legend', mmr: 1916 },
];
let ranksTabReady = false;
let fakeRanksPickerTarget = null;

let fakeRanksQueueOrder = [];

let fakeRanksPlaylistState = {};

const MMR_MIN = 0;

function clampMmr(val, fallback = 0) {
    if (val === '' || val === null || val === undefined) {
        return fallback != null ? clampMmr(fallback, 0) : 0;
    }
    const n = Number(val);
    if (!Number.isFinite(n)) {
        return fallback != null ? clampMmr(fallback, 0) : 0;
    }
    return Math.max(MMR_MIN, Math.round(n));
}

let fakeRanksAddFormState = { tier: 19, mmr: clampMmr(rankMeta(19).mmr) };

function playlistLabel(id) {
    return RANK_PLAYLISTS.find((p) => p.id === id)?.label || `Playlist ${id}`;
}

function rankMeta(tier) {
    return RL_RANKS.find((r) => r.tier === tier) || RL_RANKS[0];
}

function rankIconUrl(tier) {
    if (tier >= 19) return `${RANK_ICON_CDN}s15rank${tier}.png`;
    return `${RANK_ICON_CDN}s4-${tier}.png`;
}

function rankIconSrc(tier) {
    return `ranks/tier-${tier}.png`;
}

function rankDivisionForTier(tier) {
    if (tier <= 0) return 0;
    return (tier - 1) % 3;
}

function rankOverrideFromState(tier, displayMmr, division = 0) {
    const meta = rankMeta(tier);
    const mmr = clampMmr(displayMmr, meta.mmr);
    const mu = Number(((mmr - 100) / 20).toFixed(4));
    const div = (tier <= 0 || tier >= 22) ? 0 : Math.max(0, Math.min(3, Number(division) || 0));
    return {
        display_mmr: mmr,
        mu,
        tier: Math.max(0, Math.min(22, Number(tier) || 0)),
        division: div,
    };
}

function tierFromOverride(ov) {
    if (!ov || typeof ov !== 'object') return 19;
    if (Number.isFinite(ov.tier)) return Math.max(0, Math.min(22, Number(ov.tier)));
    return 19;
}

function mmrFromOverride(ov, tier) {
    if (ov && Number.isFinite(ov.display_mmr)) return clampMmr(ov.display_mmr);
    if (ov && Number.isFinite(ov.mu)) return clampMmr(ov.mu * 20 + 100);
    return clampMmr(rankMeta(tier).mmr);
}

function ensureFakeRanksEntry(id, fallbackTier = 19) {
    if (!fakeRanksPlaylistState[id]) {
        const tier = fallbackTier;
        fakeRanksPlaylistState[id] = { tier, mmr: clampMmr(rankMeta(tier).mmr), division: 0 };
    }
}

function syncFakeRanksAddFormUi() {
    const meta = rankMeta(fakeRanksAddFormState.tier);
    const icon = document.getElementById('fake-ranks-add-rank-icon');
    const name = document.getElementById('fake-ranks-add-rank-name');
    const mmrInput = document.getElementById('fake-ranks-add-mmr');
    const divSelect = document.getElementById('fake-ranks-add-division');
    if (icon) {
        icon.src = rankIconSrc(fakeRanksAddFormState.tier);
        icon.alt = meta.name;
        icon.onerror = () => { icon.onerror = null; icon.src = rankIconUrl(fakeRanksAddFormState.tier); };
    }
    if (name) name.textContent = meta.name;
    if (mmrInput && document.activeElement !== mmrInput) {
        mmrInput.value = String(clampMmr(fakeRanksAddFormState.mmr ?? meta.mmr));
    }
    if (divSelect) {
        divSelect.disabled = (fakeRanksAddFormState.tier <= 0 || fakeRanksAddFormState.tier >= 22);
    }
}

function buildFakeRanksPlaylistSelect() {
    const sel = document.getElementById('fake-ranks-playlist-add');
    const addBtn = document.getElementById('fake-ranks-add-btn');
    if (!sel) return;
    const configured = new Set(fakeRanksQueueOrder);
    const available = RANK_PLAYLISTS.filter((p) => !configured.has(p.id));
    const prev = sel.value;
    if (!available.length) {
        sel.innerHTML = '<option value="">All playlists configured</option>';
        sel.disabled = true;
        if (addBtn) addBtn.disabled = true;
        return;
    }
    sel.disabled = false;
    if (addBtn) addBtn.disabled = false;
    sel.innerHTML = available.map(({ id, label }) => (
        `<option value="${escHtml(id)}">${escHtml(label)}</option>`
    )).join('');
    if (available.some((p) => p.id === prev)) sel.value = prev;
    else sel.value = available[0].id;
}

function renderFakeRanksQueue() {
    const list = document.getElementById('fake-ranks-queue-list');
    const removeAll = document.getElementById('fake-ranks-remove-all-btn');
    if (!list) return;
    if (!fakeRanksQueueOrder.length) {
        list.innerHTML = '<div class="backup-empty">No playlist overrides yet. Pick a playlist and rank above, then Add playlist.</div>';
        if (removeAll) removeAll.hidden = true;
        buildFakeRanksPlaylistSelect();
        return;
    }
    if (removeAll) removeAll.hidden = false;
    list.innerHTML = fakeRanksQueueOrder.map((id, i) => {
        const st = fakeRanksPlaylistState[id] || { tier: 19, mmr: rankMeta(19).mmr, division: 0 };
        const meta = rankMeta(st.tier);
        const mmr = clampMmr(st.mmr ?? meta.mmr);
        const hasDiv = st.tier > 0 && st.tier < 22;
        const curDiv = st.division ?? 0;
        const divHtml = hasDiv ? `
            <select class="rank-queue-division-select" data-playlist="${escHtml(id)}" aria-label="${escHtml(playlistLabel(id))} Division">
                <option value="0"${curDiv === 0 ? ' selected' : ''}>Div I</option>
                <option value="1"${curDiv === 1 ? ' selected' : ''}>Div II</option>
                <option value="2"${curDiv === 2 ? ' selected' : ''}>Div III</option>
                <option value="3"${curDiv === 3 ? ' selected' : ''}>Div IV</option>
            </select>` : '';
        return `<div class="backup-item" data-playlist="${escHtml(id)}">
            <div>
                <div class="backup-name rank-queue-row-preview">
                    <span class="rank-queue-playlist">${escHtml(playlistLabel(id))}</span>
                    <span class="rank-queue-sep">·</span>
                    <img class="rank-playlist-icon rank-queue-icon" src="${rankIconSrc(st.tier)}" width="22" height="22" alt="${escHtml(meta.name)}">
                    <span class="rank-queue-rank">${escHtml(meta.name)}</span>
                    ${divHtml}
                    <input type="number" class="rank-queue-mmr-input" data-playlist="${escHtml(id)}" min="0" step="1" inputmode="numeric" autocomplete="off" value="${mmr}" aria-label="${escHtml(playlistLabel(id))} MMR">
                </div>
                <div class="backup-date">Playlist ${escHtml(id)}</div>
            </div>
            <div class="rank-queue-actions">
                <button type="button" class="rank-queue-edit-btn" data-edit-playlist="${escHtml(id)}" title="Change rank">Edit rank</button>
                <div class="restore-mini-btn" data-remove-index="${i}" title="Remove playlist">Remove</div>
            </div>
        </div>`;
    }).join('');
    list.querySelectorAll('.rank-queue-division-select').forEach((sel) => {
        sel.addEventListener('change', () => {
            const pid = sel.dataset.playlist;
            ensureFakeRanksEntry(pid);
            fakeRanksPlaylistState[pid].division = Number(sel.value) || 0;
        });
    });
    list.querySelectorAll('.rank-queue-mmr-input').forEach((input) => {
        input.addEventListener('input', () => {
            const pid = input.dataset.playlist;
            ensureFakeRanksEntry(pid);
            const val = input.value;
            if (val === '') {
                fakeRanksPlaylistState[pid].mmr = null;
                return;
            }
            let n = Number(val);
            if (Number.isFinite(n)) {
                if (n < MMR_MIN) {
                    n = MMR_MIN;
                    input.value = String(MMR_MIN);
                }
                fakeRanksPlaylistState[pid].mmr = n;
            }
        });
        input.addEventListener('blur', () => {
            const pid = input.dataset.playlist;
            ensureFakeRanksEntry(pid);
            const st = fakeRanksPlaylistState[pid];
            const meta = rankMeta(st.tier);
            const clamped = clampMmr(input.value, st.mmr ?? meta.mmr);
            input.value = String(clamped);
            st.mmr = clamped;
        });
    });
    list.querySelectorAll('[data-edit-playlist]').forEach((btn) => {
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            openFakeRanksPicker(btn.dataset.editPlaylist, btn);
        });
    });
    list.querySelectorAll('[data-remove-index]').forEach((btn) => {
        btn.onclick = (e) => {
            e.stopPropagation();
            removeFakeRanksPlaylist(Number(btn.dataset.removeIndex));
        };
    });
    buildFakeRanksPlaylistSelect();
}

function addFakeRanksPlaylistFromForm() {
    if (isAppLoading()) return;
    const sel = document.getElementById('fake-ranks-playlist-add');
    const mmrInput = document.getElementById('fake-ranks-add-mmr');
    const divSelect = document.getElementById('fake-ranks-add-division');
    const id = sel?.value?.trim();
    if (!id) {
        showToast('All playlists are already configured.', 'error');
        return;
    }
    const meta = rankMeta(fakeRanksAddFormState.tier);
    const rawVal = mmrInput?.value !== '' ? mmrInput.value : fakeRanksAddFormState.mmr;
    const mmrVal = clampMmr(rawVal, meta.mmr);
    const divVal = Number(divSelect?.value) || 0;
    const isNoDivTier = fakeRanksAddFormState.tier <= 0 || fakeRanksAddFormState.tier >= 22;
    fakeRanksPlaylistState[id] = {
        tier: fakeRanksAddFormState.tier,
        mmr: mmrVal,
        division: isNoDivTier ? 0 : divVal,
    };
    if (!fakeRanksQueueOrder.includes(id)) fakeRanksQueueOrder.push(id);
    renderFakeRanksQueue();
}

function removeFakeRanksPlaylist(index) {
    if (index < 0 || index >= fakeRanksQueueOrder.length) return;
    const id = fakeRanksQueueOrder[index];
    fakeRanksQueueOrder.splice(index, 1);
    delete fakeRanksPlaylistState[id];
    renderFakeRanksQueue();
}

function removeAllFakeRanksPlaylists() {
    if (isAppLoading()) return;
    fakeRanksQueueOrder = [];
    fakeRanksPlaylistState = {};
    renderFakeRanksQueue();
}

function openFakeRanksPicker(target, anchorEl) {
    fakeRanksPickerTarget = target;
    const pop = document.getElementById('fake-ranks-picker-popover');
    if (!pop || !anchorEl) return;
    let activeTier;
    if (target === '__add__') {
        activeTier = fakeRanksAddFormState.tier;
    } else {
        const st = fakeRanksPlaylistState[target] || { tier: 19, mmr: rankMeta(19).mmr };
        activeTier = st.tier;
    }
    pop.innerHTML = RL_RANKS.map((r) => (
        `<button type="button" class="rank-tier-btn rank-tier-btn-compact" data-tier="${r.tier}" role="option" title="${escHtml(r.name)}">`
        + `<img src="${rankIconSrc(r.tier)}" width="28" height="28" alt="">`
        + `<span>${escHtml(r.name)}</span>`
        + '</button>'
    )).join('');
    pop.querySelectorAll('.rank-tier-btn').forEach((btn) => {
        btn.classList.toggle('is-active', Number(btn.dataset.tier) === activeTier);
        btn.addEventListener('click', () => {
            const tier = Number(btn.dataset.tier);
            const meta = rankMeta(tier);
            if (target === '__add__') {
                fakeRanksAddFormState.tier = tier;
                fakeRanksAddFormState.mmr = clampMmr(meta.mmr);
                syncFakeRanksAddFormUi();
            } else {
                ensureFakeRanksEntry(target, tier);
                fakeRanksPlaylistState[target].tier = tier;
                fakeRanksPlaylistState[target].mmr = clampMmr(meta.mmr);
                renderFakeRanksQueue();
            }
            closeFakeRanksPicker();
        });
    });
    const rect = anchorEl.getBoundingClientRect();
    pop.style.top = `${rect.bottom + 6}px`;
    pop.style.left = `${Math.min(rect.left, window.innerWidth - 280)}px`;
    pop.classList.remove('hidden');
}

function closeFakeRanksPicker() {
    document.getElementById('fake-ranks-picker-popover')?.classList.add('hidden');
    fakeRanksPickerTarget = null;
}

function fakeRanksPayloadFromUi() {
    const enabledEl = document.getElementById('fake-ranks-enabled');
    const seasonEl = document.getElementById('fake-ranks-season-level');
    const winsEl = document.getElementById('fake-ranks-season-wins');
    const enabled = !!enabledEl?.checked;
    const playlists = {};
    fakeRanksQueueOrder.forEach((id) => {
        const st = fakeRanksPlaylistState[id];
        if (!st) return;
        const mmrInput = document.querySelector(`.rank-queue-mmr-input[data-playlist="${id}"]`);
        const meta = rankMeta(st.tier);
        const rawMmr = mmrInput && mmrInput.value !== '' ? mmrInput.value : (st.mmr ?? meta.mmr);
        const mmrVal = clampMmr(rawMmr, meta.mmr);
        playlists[id] = rankOverrideFromState(st.tier, mmrVal, st.division ?? 0);
    });
    const fake_ranks = { enabled, playlists };
    const seasonRaw = seasonEl?.value?.trim() ?? '';
    const winsRaw = winsEl?.value?.trim() ?? '';
    if (seasonRaw !== '' || winsRaw !== '') {
        fake_ranks.reward_levels = {};
        if (seasonRaw !== '') {
            const level = Number(seasonRaw);
            if (Number.isFinite(level) && seasonRewardMeta(level)) {
                fake_ranks.reward_levels.season_level = level;
            }
        }
        if (winsRaw !== '') {
            const wins = clampSeasonLevelWins(winsRaw);
            if (wins !== null) fake_ranks.reward_levels.season_level_wins = wins;
        }
    }
    return fake_ranks;
}

function applyLegacyDefaultToPlaylists(fr) {
    const def = fr.default;
    if (!def) return;
    RANK_PLAYLISTS.forEach(({ id }) => {
        if (fr.playlists?.[id] || fakeRanksPlaylistState[id]) return;
        const tier = tierFromOverride(def);
        fakeRanksPlaylistState[id] = {
            tier,
            mmr: mmrFromOverride(def, tier),
            division: Number(def.division) || 0,
        };
        if (!fakeRanksQueueOrder.includes(id)) fakeRanksQueueOrder.push(id);
    });
}

function loadFakeRanksFromSaved(saved) {
    buildSeasonRewardSelect();
    const fr = saved?.fake_ranks || (saved && ('enabled' in saved || saved.playlists || saved.reward_levels) ? saved : {});
    const enabledEl = document.getElementById('fake-ranks-enabled');
    const seasonEl = document.getElementById('fake-ranks-season-level');
    const winsEl = document.getElementById('fake-ranks-season-wins');
    if (enabledEl) {
        enabledEl.checked = !!fr.enabled;
        syncNameSpoofSwitchAria(enabledEl);
    }
    fakeRanksPlaylistState = {};
    fakeRanksQueueOrder = [];
    if (fr.playlists && typeof fr.playlists === 'object') {
        Object.entries(fr.playlists).forEach(([id, ov]) => {
            const tier = tierFromOverride(ov);
            fakeRanksPlaylistState[id] = {
                tier,
                mmr: clampMmr(mmrFromOverride(ov, tier)),
                division: Number(ov.division) || 0,
            };
            fakeRanksQueueOrder.push(id);
        });
    }
    applyLegacyDefaultToPlaylists(fr);
    fakeRanksAddFormState = { tier: 19, mmr: clampMmr(rankMeta(19).mmr), division: 0 };
    if (seasonEl) seasonEl.value = '';
    if (winsEl) winsEl.value = '';
    if (fr.reward_levels) {
        if (Number.isFinite(fr.reward_levels.season_level) && seasonEl) {
            const level = Math.round(fr.reward_levels.season_level);
            if (seasonRewardMeta(level)) {
                seasonEl.value = String(level);
            }
        }
        if (Number.isFinite(fr.reward_levels.season_level_wins) && winsEl) {
            const wins = clampSeasonLevelWins(fr.reward_levels.season_level_wins);
            if (wins !== null) winsEl.value = String(wins);
        }
    }
    syncFakeRanksAddFormUi();
    renderFakeRanksQueue();
}

function initRanksTab() {
    const enabledEl = document.getElementById('fake-ranks-enabled');
    const saveBtn = document.getElementById('fake-ranks-save-btn');
    if (!enabledEl || !saveBtn) return;

    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(FAKE_RANKS_KEY) || '{}'); } catch {  }
    loadFakeRanksFromSaved(saved);

    if (ranksTabReady) return;
    ranksTabReady = true;

    enabledEl.addEventListener('change', () => syncNameSpoofSwitchAria(enabledEl));

    document.getElementById('fake-ranks-add-rank-pick')?.addEventListener('click', (e) => {
        e.stopPropagation();
        openFakeRanksPicker('__add__', e.currentTarget);
    });
    const addMmrEl = document.getElementById('fake-ranks-add-mmr');
    addMmrEl?.addEventListener('input', (e) => {
        const val = e.target.value;
        if (val === '') {
            fakeRanksAddFormState.mmr = null;
            return;
        }
        let n = Number(val);
        if (Number.isFinite(n)) {
            if (n < MMR_MIN) {
                n = MMR_MIN;
                e.target.value = String(MMR_MIN);
            }
            fakeRanksAddFormState.mmr = n;
        }
    });
    addMmrEl?.addEventListener('blur', (e) => {
        const meta = rankMeta(fakeRanksAddFormState.tier);
        const clamped = clampMmr(e.target.value, meta.mmr);
        e.target.value = String(clamped);
        fakeRanksAddFormState.mmr = clamped;
    });
    document.getElementById('fake-ranks-add-btn')?.addEventListener('click', addFakeRanksPlaylistFromForm);
    document.getElementById('fake-ranks-remove-all-btn')?.addEventListener('click', removeAllFakeRanksPlaylists);

    document.addEventListener('click', (e) => {
        if (!e.target.closest('.rank-picker-popover')
            && !e.target.closest('.rank-playlist-pick')
            && !e.target.closest('.rank-queue-edit-btn')) {
            closeFakeRanksPicker();
        }
    });

    saveBtn.addEventListener('click', async () => {
        if (isAppLoading() || spoofSaveInFlight) return;
        const enabled = !!enabledEl.checked;
        const fake_ranks = fakeRanksPayloadFromUi();
        const hasPlaylists = fake_ranks.playlists && Object.keys(fake_ranks.playlists).length > 0;
        const hasRewardLevels = fake_ranks.reward_levels && (
            Number.isFinite(fake_ranks.reward_levels.season_level)
            || Number.isFinite(fake_ranks.reward_levels.season_level_wins)
        );
        if (enabled && !hasPlaylists && !hasRewardLevels) {
            showToast('Add at least one playlist override, set season reward, or turn off fake ranks.', 'error');
            return;
        }
        try {

            localStorage.setItem(FAKE_RANKS_KEY, JSON.stringify({ fake_ranks }));
            await runToolSave(saveBtn, 'fake ranks', { fake_ranks }, { enabled });
            flashButtonLabel(saveBtn, enabled ? 'Saved' : 'Saved (off)');
            if (!enabled) {
                showToast('Fake ranks off.', 'success');
            } else if (!psynetProxyRunning) {
                showToast('Fake ranks saved.', 'success');
            }
        } catch (e) {
            showToast(String(e), 'error');
        }
    });
}

let cameraTabReady = false;

function cameraLimitFromUi(axis) {
    const minEl = document.getElementById(`camera-${axis}-min`);
    const maxEl = document.getElementById(`camera-${axis}-max`);
    const intEl = document.getElementById(`camera-${axis}-interval`);
    const def = DEFAULT_CAMERA_LIMITS[axis];
    let min = Number(minEl?.value);
    let max = Number(maxEl?.value);
    let interval = Number(intEl?.value);
    if (!Number.isFinite(min) || !Number.isFinite(max) || (min <= 0 && max <= 0)) {
        return { ...def };
    }
    if (!Number.isFinite(interval) || interval <= 0) interval = def.interval;
    if (max < min) max = min;
    return { min, max, interval };
}

function setCameraLimitUi(axis, lim) {
    const def = DEFAULT_CAMERA_LIMITS[axis];
    const l = lim && typeof lim === 'object' ? lim : def;
    const min = Number.isFinite(l.min) ? l.min : def.min;
    const max = Number.isFinite(l.max) && l.max > 0 ? l.max : def.max;
    const interval = Number.isFinite(l.interval) && l.interval > 0 ? l.interval : def.interval;
    const minEl = document.getElementById(`camera-${axis}-min`);
    const maxEl = document.getElementById(`camera-${axis}-max`);
    const intEl = document.getElementById(`camera-${axis}-interval`);
    if (minEl) minEl.value = String(min);
    if (maxEl) maxEl.value = String(max);
    if (intEl) intEl.value = String(interval);
}

function applyCameraDefaultsToUi() {
    setCameraLimitUi('fov', DEFAULT_CAMERA_LIMITS.fov);
    setCameraLimitUi('height', DEFAULT_CAMERA_LIMITS.height);
    setCameraLimitUi('distance', DEFAULT_CAMERA_LIMITS.distance);
}

function cameraSpoofPayloadFromUi() {
    return {
        enabled: !!document.getElementById('camera-spoof-enabled')?.checked,
        fov: cameraLimitFromUi('fov'),
        height: cameraLimitFromUi('height'),
        distance: cameraLimitFromUi('distance'),
    };
}

function loadCameraFromSaved(saved) {
    const cam = saved.camera_spoof || {};
    const enabledEl = document.getElementById('camera-spoof-enabled');
    if (enabledEl) {
        enabledEl.checked = !!cam.enabled;
        syncNameSpoofSwitchAria(enabledEl);
    }
    setCameraLimitUi('fov', cam.fov);
    setCameraLimitUi('height', cam.height);
    setCameraLimitUi('distance', cam.distance);
}

function initCameraTab() {
    const enabledEl = document.getElementById('camera-spoof-enabled');
    const saveBtn = document.getElementById('camera-save-btn');
    if (!enabledEl || !saveBtn) return;

    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(CAMERA_SPOOF_KEY) || '{}'); } catch {  }
    if (!saved.camera_spoof) {
        applyCameraDefaultsToUi();
        if (enabledEl) {
            enabledEl.checked = false;
            syncNameSpoofSwitchAria(enabledEl);
        }
    } else {
        loadCameraFromSaved(saved);
    }

    if (cameraTabReady) return;
    cameraTabReady = true;

    enabledEl.addEventListener('change', () => syncNameSpoofSwitchAria(enabledEl));
    document.getElementById('camera-reset-defaults-btn')?.addEventListener('click', () => {
        if (isAppLoading()) return;
        applyCameraDefaultsToUi();
        showToast('Restored defaults', 'success');
    });

    saveBtn.addEventListener('click', async () => {
        if (isAppLoading() || spoofSaveInFlight) return;
        const camera_spoof = cameraSpoofPayloadFromUi();
        try {
            localStorage.setItem(CAMERA_SPOOF_KEY, JSON.stringify({ camera_spoof }));
            await runToolSave(saveBtn, 'camera', { camera_spoof }, { enabled: camera_spoof.enabled });
            flashButtonLabel(saveBtn, camera_spoof.enabled ? 'Saved' : 'Saved (off)');
            if (camera_spoof.enabled) {
                showToast('Camera limits saved. Restart Rocket League.', 'success');
            } else {
                showToast('Camera limits off.', 'success');
            }
        } catch (e) {
            showToast(String(e), 'error');
        }
    });
}

function wireReswapButton() {
    const reswapBtn = document.getElementById('reswap-btn');
    if (!reswapBtn || reswapBtn.dataset.wired === '1') return;
    reswapBtn.dataset.wired = '1';

    reswapBtn.addEventListener('click', async (e) => {
        if (isAppLoading()) return;
        const confirmed = await askConfirm(
            'This action is irreversible and should only be used if you have recently verified your game files in the Epic Games launcher.\n\nAre you sure you want to reswap all items?',
            'Reswap All Items'
        );
        if (!confirmed) return;

        const btn = e.currentTarget;
        if (btn?.dataset.busy === '1') return;
        if (btn) {
            btn.dataset.busy = '1';
            btn.disabled = true;
        }
        try {
            updateStatus('Running Reswap...', false);
            const result = await invoke('reswap_all');
            showToast(result || 'Swaps re-applied successfully.', 'success');
            refreshBackups();
            setTimeout(() => updateStatus('bitsfdb', false), 2000);
        } catch (err) {
            showToast(String(err), 'error');
            updateStatus('Error', true);
            setTimeout(() => updateStatus('bitsfdb', false), 2000);
        } finally {
            if (btn) {
                btn.disabled = false;
                delete btn.dataset.busy;
            }
        }
    });
}
let paletteBusy = false;

const PALETTE_UI_DISABLED = false;

function setPaletteUnavailable() {
    const block = document.getElementById('palette-disabled-block');
    block?.classList.add('is-disabled');
    block?.setAttribute('aria-disabled', 'true');
    const toggle = document.getElementById('rich-palette-enabled');
    const applyBtn = document.getElementById('palette-apply-btn');
    const restoreBtn = document.getElementById('palette-restore-btn');
    if (toggle) {
        toggle.disabled = true;
        toggle.checked = false;
        syncNameSpoofSwitchAria(toggle);
    }
    if (applyBtn) applyBtn.disabled = true;
    if (restoreBtn) restoreBtn.disabled = true;
    document.getElementById('palette-switch-row')?.classList.remove('is-applied');
    invoke('sync_palette_psynet_config', { forceEnabled: false }).catch(() => {});
}

function setPaletteBusy(busy) {
    paletteBusy = !!busy;
    if (!paletteBusy) return;
    const applyBtn = document.getElementById('palette-apply-btn');
    const restoreBtn = document.getElementById('palette-restore-btn');
    if (applyBtn) applyBtn.disabled = true;
    if (restoreBtn) restoreBtn.disabled = true;
}

function syncPaletteUi(applied, status) {
    if (PALETTE_UI_DISABLED) {
        setPaletteUnavailable();
        return;
    }

    const block = document.getElementById('palette-disabled-block');
    block?.classList.remove('is-disabled');
    block?.removeAttribute('aria-disabled');
    const toggle = document.getElementById('rich-palette-enabled');
    const row = document.getElementById('palette-switch-row');
    const applyBtn = document.getElementById('palette-apply-btn');
    const restoreBtn = document.getElementById('palette-restore-btn');
    const on = !!applied;
    const hasBackup = !!status?.backup_present;
    if (toggle) {
        toggle.checked = on;
        toggle.disabled = false;
        syncNameSpoofSwitchAria(toggle);
    }
    row?.classList.toggle('is-applied', on);
    if (applyBtn) applyBtn.disabled = on || paletteBusy;
    if (restoreBtn) restoreBtn.disabled = !on || !hasBackup || paletteBusy;
}

async function refreshPaletteStatus() {
    if (PALETTE_UI_DISABLED) {
        setPaletteUnavailable();
        return;
    }
    try {
        const st = await invoke('get_palette_status');
        syncPaletteUi(!!st.applied, st);
        await invoke('sync_palette_psynet_config').catch(() => {});
    } catch {
        syncPaletteUi(false);
    }
}

async function runPaletteAction(btn, command, doneLabel, fallbackMsg) {
    if (PALETTE_UI_DISABLED) return;
    if (isAppLoading() || paletteBusy) return;
    setPaletteBusy(true);
    let result = null;
    try {
        result = await invoke(command);
        showToast(result?.message || fallbackMsg, 'success');
    } catch (e) {
        showToast(String(e), 'error');
    } finally {
        setPaletteBusy(false);
        if (result) {
            syncPaletteUi(!!result.applied, result);
            flashButtonLabel(btn, doneLabel);
        } else {

            await refreshPaletteStatus();
        }
    }
}

async function wireLaunchOnStartup() {
    const toggle = document.getElementById('launch-on-startup');
    if (!toggle || toggle.dataset.wired === '1') return;
    toggle.dataset.wired = '1';
    toggle.checked = await invoke('get_launch_on_startup').catch(() => false);
    syncNameSpoofSwitchAria(toggle);
    toggle.addEventListener('change', async () => {
        try {
            await invoke('set_launch_on_startup', { enable: toggle.checked });
            showToast(toggle.checked ? 'VelocityRL will start when you log in.' : 'VelocityRL will no longer start on login.', 'success');
        } catch (e) {
            toggle.checked = await invoke('get_launch_on_startup').catch(() => false);
            syncNameSpoofSwitchAria(toggle);
            showToast(String(e), 'error');
        }
    });
}

async function wireReplayOptIn() {
    const toggle = document.getElementById('replay-opt-in');
    const hint = document.getElementById('replay-status-hint');
    if (!toggle || toggle.dataset.wired === '1') return;
    toggle.dataset.wired = '1';
    const cfg = await invoke('get_config').catch(() => ({}));
    toggle.checked = !!cfg.replay_opt_in;
    syncNameSpoofSwitchAria(toggle);
    toggle.addEventListener('change', async () => {
        const c = await invoke('get_config').catch(() => ({ game_dir: '' }));
        await invoke('save_config', { config: { ...c, replay_opt_in: toggle.checked } }).catch(() => {});
        if (hint) {
            hint.style.display = 'block';
            hint.textContent = toggle.checked
                ? 'Replay archiving is on. Your replays will be saved securely after each match.'
                : 'Replay archiving is off.';
            setTimeout(() => { hint.style.display = 'none'; }, 4000);
        }

        if (toggle.checked) {
            const status = await invoke('replay_token_status').catch(() => null);
            if (status && !status.registered) {
                await invoke('register_replay_token').catch(() => {});
            }
        }
        updateReplayVaultVisibility(toggle.checked);
        invoke('append_launch_log', { message: `replays: opt-in ${toggle.checked ? 'enabled' : 'disabled'}` }).catch(() => {});
    });

    wireReplayVault();
    updateReplayVaultVisibility(toggle.checked);
}

function updateReplayVaultVisibility(enabled) {
    const vault = document.getElementById('replay-vault');
    if (vault) vault.style.display = enabled ? 'block' : 'none';
}

function wireReplayVault() {
    const refreshBtn = document.getElementById('replay-vault-refresh');
    if (!refreshBtn || refreshBtn.dataset.wired === '1') return;
    refreshBtn.dataset.wired = '1';
    refreshBtn.addEventListener('click', async () => {
        refreshBtn.disabled = true;
        const oldLabel = refreshBtn.textContent;
        refreshBtn.textContent = 'Loading…';
        try {
            let data = await invoke('get_my_replays');
            if (!data.registered) {

                await invoke('register_replay_token').catch(() => {});
                data = await invoke('get_my_replays').catch(() => ({ registered: false, total: 0, replays: [] }));
            }
            renderReplayVault(data);
        } catch (e) {
            renderReplayVaultError(String(e));
        } finally {
            refreshBtn.disabled = false;
            refreshBtn.textContent = oldLabel;
        }
    });
}

function renderReplayVault(data) {
    const list = document.getElementById('replay-vault-list');
    const count = document.getElementById('replay-vault-count');
    if (!list) return;
    list.style.display = 'block';
    list.innerHTML = '';
    if (count) {
        count.style.display = 'inline';
        count.textContent = `${data.total} saved`;
    }
    if (!data.replays || data.replays.length === 0) {
        list.innerHTML = '<p class="field-hint">No replays saved yet. Play a match with archiving on and it will show up here.</p>';
        return;
    }
    for (const r of data.replays) {
        const row = document.createElement('div');
        row.style.cssText = 'display:flex; align-items:center; justify-content:space-between; gap:10px; padding:7px 10px; border:1px solid var(--border-color, #333); border-radius:8px; margin-bottom:6px;';
        const label = document.createElement('span');
        label.style.cssText = 'min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;';
        const when = r.uploaded_at ? new Date(r.uploaded_at).toLocaleString() : '';
        label.textContent = [r.map || r.filename, r.match_type, when].filter(Boolean).join(' — ');
        const link = document.createElement('a');
        link.href = '#';
        link.textContent = 'Download';
        link.style.cssText = 'flex:none; color:var(--accent-blue, #4af);';
        link.addEventListener('click', (ev) => {
            ev.preventDefault();
            const url = r.download_url;
            if (url) window.__TAURI__?.core?.invoke('plugin:shell|open', { path: url });
        });
        row.appendChild(label);
        row.appendChild(link);
        list.appendChild(row);
    }
}

function renderReplayVaultError(msg) {
    const list = document.getElementById('replay-vault-list');
    if (!list) return;
    list.style.display = 'block';
    list.innerHTML = `<p class="field-hint">Could not load replays: ${escHtml(msg)}</p>`;
    const count = document.getElementById('replay-vault-count');
    if (count) count.style.display = 'none';
}

function initMiscTab() {
    refreshPaletteStatus();

    const paletteApply = document.getElementById('palette-apply-btn');
    const paletteRestore = document.getElementById('palette-restore-btn');
    const paletteToggle = document.getElementById('rich-palette-enabled');

    if (initMiscTab._wired) return;
    initMiscTab._wired = true;

    paletteToggle?.addEventListener('change', () => {
        if (paletteToggle.checked) {
            runPaletteAction(paletteApply, 'apply_rich_palette', 'Applied', 'Color palette on. Restart Rocket League.');
        } else {
            runPaletteAction(paletteRestore, 'restore_rich_palette', 'Restored', 'Color palette off.');
        }
    });

    paletteApply?.addEventListener('click', () => runPaletteAction(
        paletteApply, 'apply_rich_palette', 'Applied', 'Color palette on. Restart Rocket League.',
    ));
    paletteRestore?.addEventListener('click', () => runPaletteAction(
        paletteRestore, 'restore_rich_palette', 'Restored', 'Color palette off.',
    ));

    const enabledEl = document.getElementById('logo-spoof-enabled');
    const urlEl = document.getElementById('logo-spoof-url');
    const saveBtn = document.getElementById('logo-spoof-save-btn');
    const defaultLink = document.getElementById('logo-spoof-default');
    const blogEnabledEl = document.getElementById('blog-spoof-enabled');
    const blogMotdEl = document.getElementById('blog-spoof-motd');
    const blogSaveBtn = document.getElementById('blog-spoof-save-btn');

    if (enabledEl && urlEl && saveBtn) {
        let saved = {};
        try { saved = JSON.parse(localStorage.getItem(LOGO_SPOOF_KEY) || '{}'); } catch { }
        const ls = saved.logo_spoof || {};
        enabledEl.checked = !!ls.enabled;
        syncNameSpoofSwitchAria(enabledEl);
        urlEl.value = ls.logo_url || saved.logo_url || DEFAULT_SEASON23_LOGO_URL;

        let blogSaved = {};
        try { blogSaved = JSON.parse(localStorage.getItem(BLOG_SPOOF_KEY) || '{}'); } catch { }
        const bs = blogSaved.blog_spoof || {};
        if (blogEnabledEl) {
            blogEnabledEl.checked = !!bs.enabled;
            syncNameSpoofSwitchAria(blogEnabledEl);
        }
        if (blogMotdEl) blogMotdEl.value = bs.motd || blogSaved.motd || DEFAULT_BLOG_MOTD;

        enabledEl.addEventListener('change', () => syncNameSpoofSwitchAria(enabledEl));
        blogEnabledEl?.addEventListener('change', () => syncNameSpoofSwitchAria(blogEnabledEl));
        defaultLink?.addEventListener('click', (e) => {
            e.preventDefault();
            urlEl.value = DEFAULT_SEASON23_LOGO_URL;
            showToast('Default logo set.', 'success');
        });
    }
    saveBtn.addEventListener('click', async () => {
        if (isAppLoading() || spoofSaveInFlight) return;
        const logo_url = urlEl.value.trim();
        const enabled = !!enabledEl.checked;
        if (enabled && !logo_url) {
            showToast('Enter a logo URL or turn off.', 'error');
            return;
        }
        const logo_spoof = { enabled, logo_url };
        try {
            localStorage.setItem(LOGO_SPOOF_KEY, JSON.stringify({ logo_spoof }));
            invoke('append_launch_log', {
                message: `psynet: season logo save enabled=${enabled} url_len=${logo_url.length}`,
            }).catch(() => {});
            await runToolSave(saveBtn, 'season logo', { logo_spoof }, { enabled });
            flashButtonLabel(saveBtn, enabled ? 'Saved' : 'Saved (off)');
            if (!enabled) showToast('Season logo off.', 'success');
        } catch (e) {
            showToast(String(e), 'error');
        }
    });
    blogSaveBtn?.addEventListener('click', async () => {
        if (isAppLoading() || spoofSaveInFlight) return;
        const motd = (blogMotdEl?.value || '').trim();
        const enabled = !!(blogEnabledEl && blogEnabledEl.checked);
        if (enabled && !motd) {
            showToast('Enter text or turn off.', 'error');
            return;
        }
        const blog_spoof = { enabled, motd };
        try {
            localStorage.setItem(BLOG_SPOOF_KEY, JSON.stringify({ blog_spoof }));
            await runToolSave(blogSaveBtn, 'MotD', { blog_spoof }, { enabled });
            flashButtonLabel(blogSaveBtn, enabled ? 'Saved' : 'Saved (off)');
            if (!enabled) showToast('Main-menu MotD off.', 'success');
        } catch (e) {
            showToast(String(e), 'error');
        }
    });
}

function paintLabel(id) {
    return PAINT_NAMES[id] || PAINT_NAMES[String(id)] || `Paint ${id}`;
}

function wirePaintSwatches(swatchId, selectId, selectedLabelId) {
    const wrap = document.getElementById(swatchId);
    const select = document.getElementById(selectId);
    const selectedEl = document.getElementById(selectedLabelId);
    if (!wrap || wrap.dataset.wired === '1') return;
    wrap.dataset.wired = '1';

    if (!wrap.querySelector('.paint-swatch')) {
        wrap.innerHTML = Object.entries(PAINT_NAMES).map(([id, name]) => (
            `<button type="button" class="paint-swatch${id === '0' ? ' is-active' : ''}" data-paint="${id}" aria-label="${escHtml(name)}" aria-checked="${id === '0' ? 'true' : 'false'}"></button>`
        )).join('');
    }

    const setPaint = (id) => {
        const sid = String(id);
        wrap.querySelectorAll('.paint-swatch').forEach((btn) => {
            const on = btn.dataset.paint === sid;
            btn.classList.toggle('is-active', on);
            btn.setAttribute('aria-checked', on ? 'true' : 'false');
        });
        if (select) select.value = sid;
        if (selectedEl) selectedEl.textContent = paintLabel(sid);
    };

    wrap.querySelectorAll('.paint-swatch').forEach((btn) => {
        btn.addEventListener('click', () => {
            if (btn.disabled || wrap.closest('.spawn-paint-block')?.classList.contains('is-disabled')) return;
            setPaint(btn.dataset.paint);
        });
    });
    select?.addEventListener('change', () => {
        if (wrap.closest('.spawn-paint-block')?.classList.contains('is-disabled')) {
            select.value = '0';
            return;
        }
        setPaint(select.value);
    });
    setPaint(select?.value || '0');
}

async function initTitlesTab() {
    if (!titlesTabReady) {
        titlesTabReady = true;
        wireTitleSpoofControls();
        await loadTitlesDatabase();
        loadTitleSpoofForm();
        renderDonorList('');
        renderDisplayList('');
        updateTitlePreview();
        attachCloseGuard();
    }
    await refreshProxyStatus();
}

function findTitleById(id) {
    if (!id || !titlesDb.titles?.length) return null;
    const raw = String(id).trim();
    if (!raw) return null;
    const exact = titlesDb.titles.find(x => (x.id || x.Id) === raw);
    if (exact) return exact;
    const lower = raw.toLowerCase();
    return titlesDb.titles.find(x => (x.id || x.Id || '').toLowerCase() === lower) || null;
}

function loadTitleSpoofForm() {
    ensureTitleSwapsLoaded();
    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(TITLE_SPOOF_KEY) || '{}'); } catch {  }
    const last = titleSwaps[titleSwaps.length - 1];
    const equipId = last?.equip_title_id || saved.equip_title_id || 'Team_Iraq_World_Cup_2026';
    const displayId = last?.display_title_id || saved.display_title_id || '';
    const donor = findTitleById(equipId) || {
        id: equipId,
        text: equipId.replace(/_/g, ' '),
        category: '',
    };
    selectDonor(donor, false);

    const customText = last?.custom_text || saved.custom_text || '';
    const category = last?.category || saved.category || '';
    const catalogDisplay = displayId && displayId !== 'custom' ? findTitleById(displayId) : null;
    const catalogText = catalogDisplay
        ? (catalogDisplay.text || catalogDisplay.Text || '')
        : '';
    if (catalogDisplay) {

        userEditedCustomText = false;
        selectDisplay({
            ...catalogDisplay,
            text: customText || catalogText,

            category: catalogDisplay.category || catalogDisplay.Category || category,
        }, false);
        userEditedCustomText = !!(customText && customText !== catalogText);
    } else {
        userEditedCustomText = !!customText;
        const set = (eid, v) => { const el = document.getElementById(eid); if (el) el.value = v; };
        set('title-display-id', displayId === 'custom' ? 'custom' : '');
        set('title-custom-text', customText);

        displayPick = (customText || category) ? {
            id: displayId || 'custom',
            text: customText,
            category: category || '',
        } : null;
        setSelectedSlot('display-selected', displayPick, 'Search below - or type custom text');
        renderDisplayList(document.getElementById('display-search')?.value || '');
    }
    setTitleColorForm(last?.title_color || null);
    renderTitleSwapList();
    updateTitlePreview();
}

function trySelectTitleByTypedId(raw, side) {
    const t = findTitleById(raw);
    if (!t) return;
    if (side === 'donor') selectDonor(t, false);
    else selectDisplay(t, false);
}

function wireTitleSpoofControls() {
    document.getElementById('title-apply-btn')?.addEventListener('click', saveTitleSpoof);
    document.getElementById('title-restore-all-btn')?.addEventListener('click', restoreAllTitleSwaps);
    wireTitleColorInputs();

    const onDonorSearchId = debounce((q) => trySelectTitleByTypedId(q, 'donor'), 175);
    const onDisplaySearchId = debounce((q) => trySelectTitleByTypedId(q, 'display'), 175);

    document.getElementById('donor-search')?.addEventListener('input', (e) => {
        const q = e.target.value || '';
        renderDonorList(q);
        onDonorSearchId(q);
    });
    document.getElementById('display-search')?.addEventListener('input', (e) => {
        const q = e.target.value || '';
        renderDisplayList(q);
        onDisplaySearchId(q);
    });
    document.getElementById('title-custom-text')?.addEventListener('input', () => {
        const input = document.getElementById('title-custom-text');

        userEditedCustomText = !!(input && input.value.trim());

        updateTitlePreview();
    });
}

function flagEmoji(cc) {
    const code = String(cc || '').toLowerCase();
    if (!/^[a-z]{2}$/.test(code)) return '';
    const A = 0x1f1e6;
    return String.fromCodePoint(A + code.charCodeAt(0) - 97, A + code.charCodeAt(1) - 97);
}

function regionalIndicatorsToCc(ch0, ch1) {
    if (!ch0 || !ch1) return '';
    const a = ch0.codePointAt(0);
    const b = ch1.codePointAt(0);
    if (a < 0x1f1e6 || a > 0x1f1ff || b < 0x1f1e6 || b > 0x1f1ff) return '';
    return String.fromCharCode(97 + (a - 0x1f1e6), 97 + (b - 0x1f1e6));
}

const APPLE_FLAG_PNG = new Set([
    'ar', 'at', 'au', 'ba', 'be', 'br', 'ca', 'cd', 'ch', 'ci', 'co', 'cv', 'cw', 'cz',
    'de', 'dz', 'ec', 'eg', 'es', 'fr', 'gh', 'hr', 'ht', 'iq', 'ir', 'jo', 'jp', 'kr',
    'ma', 'mx', 'nl', 'no', 'nz', 'pa', 'pt', 'py', 'qa', 'sa', 'sc', 'se', 'sn', 'tn',
    'tr', 'us', 'uy', 'uz', 'za',
]);

function flagImgHtml(cc) {
    const code = String(cc || '').toLowerCase();
    if (!APPLE_FLAG_PNG.has(code)) return '';
    const alt = flagEmoji(code);
    const src = `${API_BASE}/thumbnails/flags/flag_${code}.png`;
    return `<img class="title-flag" src="${src}" alt="${alt}" title="${code.toUpperCase()}" width="18" height="18" draggable="false" loading="lazy">`;
}

function formatTitleText(text) {
    return String(text || '')
        .replace(/\{flag_([a-z]{2})\}/gi, (m, cc) => (APPLE_FLAG_PNG.has(String(cc).toLowerCase()) ? flagEmoji(cc) : m))
        .replace(/\bFLAG_([A-Z]{2})\b/gi, (m, cc) => (APPLE_FLAG_PNG.has(String(cc).toLowerCase()) ? flagEmoji(cc) : m));
}

function formatTitleHtml(text) {
    const plain = formatTitleText(text);
    const chars = [...plain];
    let out = '';
    for (let i = 0; i < chars.length; i++) {
        const cc = i + 1 < chars.length ? regionalIndicatorsToCc(chars[i], chars[i + 1]) : '';
        if (cc) {
            out += flagImgHtml(cc) || escHtml(chars[i] + chars[i + 1]);
            i++;
            continue;
        }
        out += escHtml(chars[i]);
    }
    return out;
}

function normalizeHexColor(raw) {
    if (!raw || typeof raw !== 'string') return '';
    const s = raw.trim();
    if (!s || s.toLowerCase() === 'transparent') return '';
    return s.startsWith('#') ? s : `#${s}`;
}

function findCategoryEntry(catId) {
    if (!catId) return null;
    const cats = titlesDb.categories || {};
    if (cats[catId]) return cats[catId];
    const lower = catId.toLowerCase();
    const key = Object.keys(cats).find((k) => k.toLowerCase() === lower);
    return key ? cats[key] : null;
}

function categoryColorsFromTitles(catId) {
    if (!catId) return null;
    const lower = catId.toLowerCase();
    const t = (titlesDb.titles || []).find((x) => {
        const cid = x.category || x.Category || '';
        return cid && cid.toLowerCase() === lower;
    });
    if (!t) return null;
    return {
        Color: t.color || t.Color || '',
        GlowColor: t.glow || t.GlowColor || t.glow_color || '',
    };
}

function categoryColors(catId) {
    if (!catId) return { color: '#c8c8c8', glow: '' };
    const c = findCategoryEntry(catId) || categoryColorsFromTitles(catId);
    if (!c) return { color: '#c8c8c8', glow: '' };
    const color = normalizeHexColor(c.Color || c.color || '') || '#c8c8c8';
    const glow = normalizeHexColor(c.GlowColor || c.glow_color || c.glow || '');
    return { color, glow };
}

function titleCategoryId(title) {
    return String(title?.category || title?.Category || '').trim();
}

function titleGlowHex(title) {
    return titleColors(title).glow || '';
}

function titleGlowLabel(title) {
    return titleGlowHex(title) ? 'glow' : 'no glow';
}

function formatTitlePickMeta(title) {
    const cat = titleCategoryId(title) || '-';
    return `${cat} · ${titleGlowLabel(title)}`;
}

function setSelectedSlot(elId, title, emptyHint) {
    const el = document.getElementById(elId);
    if (!el) return;
    if (!title) {
        el.classList.add('is-empty');
        el.innerHTML = `<span class="title-selected-label">Not picked</span><span class="title-selected-meta">${escHtml(emptyHint)}</span>`;
        return;
    }
    el.classList.remove('is-empty');
    const text = title.text || title.Text || title.id || '';
    const style = titleInlineStyle(title);
    el.innerHTML = `<span class="title-selected-label" style="${style}">${formatTitleHtml(text)}</span><span class="title-selected-meta">${escHtml(formatTitlePickMeta(title))}</span>`;
}

function selectDonor(title, toast) {
    donorPick = title;
    const id = title?.id || title?.Id || '';
    const equip = document.getElementById('title-equip-id');
    if (equip) equip.value = id;
    setSelectedSlot('donor-selected', title, 'Search below');
    renderDonorList(document.getElementById('donor-search')?.value || '');
    updateTitlePreview();
    if (toast && id) showToast(`Donor set: ${id}`, 'success');
}

function selectDisplay(title, toast) {
    displayPick = title;
    const id = title?.id || title?.Id || '';
    const text = title?.text || title?.Text || '';
    const set = (eid, v) => { const el = document.getElementById(eid); if (el) el.value = v; };
    const customEl = document.getElementById('title-custom-text');
    const hasCustom = !!(customEl?.value?.trim());
    set('title-display-id', id);

    if (!hasCustom) set('title-custom-text', text);
    setSelectedSlot('display-selected', title, 'Search below - or type custom text');
    renderDisplayList(document.getElementById('display-search')?.value || '');
    updateTitlePreview();
    if (toast && id) {
        const shown = (customEl?.value?.trim()) || text || id;
        showToast(`Look set: ${formatTitleText(shown)}`, 'success');
    }
}

async function loadBundledCategories() {
    try {
        const res = await fetch('categories.json', { cache: 'no-store' });
        if (!res.ok) return {};
        return categoriesMapFromPayload(await res.json());
    } catch {
        return {};
    }
}

function categoriesMapFromPayload(data) {
    if (!data) return {};
    let categories = data.categories || data.Categories || data;
    if (Array.isArray(categories)) {
        const map = {};
        for (const c of categories) {
            const id = c.ID || c.Id || c.id;
            if (id) map[id] = c;
        }
        return map;
    }
    if (categories && typeof categories === 'object') return { ...categories };
    return {};
}

async function loadTitlesDatabase() {
    const lists = [document.getElementById('donor-list'), document.getElementById('display-list')];
    const fail = (msg) => lists.forEach(list => { if (list) list.innerHTML = `<div class="backup-empty">${escHtml(msg)}</div>`; });
    const bundledCats = await loadBundledCategories();
    const apply = (raw) => {
        titlesDb = normalizeTitlesPayload(raw);

        titlesDb.categories = { ...titlesDb.categories, ...bundledCats };
    };
    try {
        const res = await fetch(`${API_BASE}/v2/rl/titles`, { cache: 'no-store' });
        if (res.ok) {
            apply(await res.json());
            return;
        }
    } catch {  }
    try {
        const res = await fetch('https://raw.githubusercontent.com/bitsfdb/VelocityRL/main/tools/psynet_proxy/titles.json', { cache: 'no-store' });
        if (res.ok) {
            apply(await res.json());
            return;
        }
    } catch {  }
    fail('Could not load titles DB from api.velocityrl.tech.');
}

function categoriesFromTitles(titles) {
    const map = {};
    for (const t of titles || []) {
        const id = t.category || t.Category;
        if (!id || map[id]) continue;
        const color = t.color || t.Color || '';
        const glow = t.glow || t.GlowColor || t.glow_color || '';
        if (!color && !glow) continue;
        map[id] = { ID: id, Color: color, GlowColor: glow };
    }
    return map;
}

function normalizeTitlesPayload(data) {
    if (!data) return { titles: [], categories: {} };
    if (Array.isArray(data)) {
        const titles = data;
        return { titles, categories: categoriesFromTitles(titles) };
    }
    const titles = data.titles || data.Titles || [];
    let categories = categoriesMapFromPayload(data);

    if (!categories || !Object.keys(categories).length) {
        categories = categoriesFromTitles(titles);
    }
    return { titles, categories };
}

function updateTitlePreview() {
    const donorChip = document.getElementById('donor-preview');
    const chip = document.getElementById('title-preview');
    if (donorChip) {
        donorChip.innerHTML = formatTitleHtml(donorPick?.text || donorPick?.Text || '-');

        if (donorPick) {
            const { color, glow } = titleColors(donorPick);
            donorChip.style.color = color || '#c8c8c8';
            donorChip.style.textShadow = titleTextShadow(glow);
        } else {

            donorChip.style.color = '';
            donorChip.style.textShadow = '';
        }
    }
    if (!chip) return;
    const text = document.getElementById('title-custom-text')?.value?.trim() || '-';
    chip.innerHTML = formatTitleHtml(text);

    let color = '#c8c8c8';
    let glow = '';
    const customTc = readTitleColorFromForm();
    if (customTc) {
        color = `#${customTc.color}`;
        glow = customTc.glow_color ? `#${customTc.glow_color}` : '';
    } else if (displayPick) {
        ({ color, glow } = titleColors(displayPick));
    } else {
        ({ color, glow } = categoryColors(lookCategory()));
    }

    chip.style.color = color || '#c8c8c8';
    chip.style.textShadow = titleTextShadow(glow);
}

function filterTitles(q) {
    const query = (q || '').toLowerCase().trim();
    return (titlesDb.titles || []).filter(t => {
        const id = (t.id || t.Id || '').toLowerCase();
        const text = (t.text || t.Text || '').toLowerCase();
        const cat = (t.category || t.Category || '').toLowerCase();
        const pretty = formatTitleText(t.text || t.Text || '').toLowerCase();
        if (!query) return true;
        return id.includes(query) || text.includes(query) || cat.includes(query) || pretty.includes(query);
    }).slice(0, 200);
}

function renderTitleRows(listEl, rows, activeId, onPick) {
    if (!listEl) return;
    if (!rows.length) {
        listEl.innerHTML = '<div class="backup-empty">No titles match.</div>';
        return;
    }
    listEl.innerHTML = rows.map(t => {
        const id = t.id || t.Id || '';
        const text = t.text || t.Text || id;
        const meta = formatTitlePickMeta(t);
        const active = id && id === activeId ? ' is-active' : '';
        const hasGlow = !!titleGlowHex(t);
        const style = `flex:1;${titleInlineStyle(t)}`;
        return `<div class="title-row${active}${hasGlow ? ' has-glow' : ''}" data-id="${escHtml(id)}">
            <span class="title-row-text" style="${style}">${formatTitleHtml(text)}</span>
            <span class="title-row-meta">${escHtml(meta)}</span>
        </div>`;
    }).join('');

    if (!listEl.dataset.delegated) {
        listEl.dataset.delegated = 'true';
        listEl.addEventListener('click', (e) => {
            const row = e.target.closest('.title-row');
            if (row) {
                const t = findTitleById(row.dataset.id);
                if (t) onPick(t, true);
            }
        });
    }
}

function renderDonorList(q) {
    const active = document.getElementById('title-equip-id')?.value || '';
    renderTitleRows(document.getElementById('donor-list'), filterTitles(q), active, selectDonor);
}

function renderDisplayList(q) {
    const active = document.getElementById('title-display-id')?.value || '';
    renderTitleRows(document.getElementById('display-list'), filterTitles(q), active, selectDisplay);
}

async function saveTitleSpoof() {
    if (isAppLoading() || spoofSaveInFlight) return;
    const applyBtn = document.getElementById('title-apply-btn');
    if (applyBtn?.dataset.labelFlashing === '1' || applyBtn?.dataset.saving === '1') return;
    const entry = pickerSwapEntry();
    if (!entry?.equip_title_id) {
        showToast('Pick a donor title first.', 'error');
        return;
    }
    if (!entry.custom_text) {
        showToast('Enter custom text (or pick a catalog look to fill it).', 'error');
        return;
    }
    if (/[\x00-\x1f]/.test(entry.custom_text)) {
        showToast('Custom text cannot include control characters.', 'error');
        return;
    }
    if (entry.category && /["\\\x00-\x1f]/.test(entry.category)) {
        showToast('Category has illegal characters.', 'error');
        return;
    }
    if (document.getElementById('title-color-custom')?.checked && !entry.title_color) {
        showToast('Custom colors need valid 6-digit hex (e.g. AEF7FF).', 'error');
        return;
    }
    ensureTitleSwapsLoaded();
    titleSwaps = titleSwaps.filter((s) => s.equip_title_id !== entry.equip_title_id);
    titleSwaps.unshift(entry);
    try {
        persistTitleSpoofLocal();
        await runToolSave(applyBtn, 'titles', titleSpoofPayload(), { enabled: true });
        renderTitleSwapList();
        flashButtonLabel(applyBtn, 'Saved!');
        showToast('Title remap saved. Restart RL if already in-menu.', 'success');
    } catch (e) {
        showToast(String(e), 'error');
    }
}

async function restoreTitleSwap(index) {
    if (isAppLoading()) return;
    ensureTitleSwapsLoaded();
    if (index < 0 || index >= titleSwaps.length) return;
    const list = document.getElementById('title-swap-list');
    const btn = list?.querySelector(`[data-restore-index="${index}"]`);
    if (btn?.dataset.labelFlashing === '1') return;
    titleSwaps.splice(index, 1);
    try {
        await writeTitleSpoofConfig();

        if (btn) {
            list?.querySelectorAll('[data-restore-index]').forEach((b) => {
                b.style.pointerEvents = 'none';
            });
            const allBtn = document.getElementById('title-restore-all-btn');
            if (allBtn) allBtn.style.pointerEvents = 'none';
            flashButtonLabel(btn, 'Restored!', 1500, () => {
                if (allBtn) allBtn.style.pointerEvents = '';
                renderTitleSwapList();
            });
        } else {
            renderTitleSwapList();
        }
        showToast('Title remap restored.', 'success');
    } catch (e) {
        showToast(String(e), 'error');
    }
}

async function restoreAllTitleSwaps() {
    if (isAppLoading()) return;
    const btn = document.getElementById('title-restore-all-btn');
    if (btn?.dataset.labelFlashing === '1') return;
    titleSwapsLoaded = true;
    titleSwaps = [];
    try {
        await writeTitleSpoofConfig();
        const list = document.getElementById('title-swap-list');
        if (list) {
            list.innerHTML = '<div class="backup-empty">No title remaps yet. Pick a donor and a look (or custom text), then Add swap.</div>';
        }

        if (btn) {
            btn.hidden = false;
            flashButtonLabel(btn, 'Restored!', 1500, () => renderTitleSwapList());
        } else {
            renderTitleSwapList();
        }
        showToast('All title remaps restored.', 'success');
    } catch (e) {
        showToast(String(e), 'error');
    }
}

const CHANGELOG_CACHE_KEY = 'velocityrl_changelog_cache';
const CHANGELOG_CACHE_TTL = 30 * 60 * 1000;

function renderChangelog(releases) {
    const body = document.getElementById('changelog-body');
    const list = Array.isArray(releases) ? releases : (releases?.releases || []);
    if (!list.length) { body.innerHTML = '<div style="color:var(--text-secondary);padding:20px;">No releases found.</div>'; return; }
    body.innerHTML = list.map(r => {
        const tag = r.tag_name || r.name || 'Release';
        const date = r.published_at ? new Date(r.published_at).toLocaleDateString('en-US', { year:'numeric', month:'long', day:'numeric' }) : '';
        return `
            <div class="changelog-release" style="margin-bottom:20px;padding-bottom:16px;border-bottom:1px solid rgba(255,255,255,0.08);">
                <div style="display:flex;align-items:baseline;justify-content:space-between;margin-bottom:6px;">
                    <div class="changelog-release-tag" style="font-size:15px;font-weight:700;color:var(--accent);">${escHtml(tag)}</div>
                    <div class="changelog-release-date" style="font-size:12px;color:var(--muted);">${escHtml(date)}</div>
                </div>
                <div class="changelog-release-body" style="font-size:13px;line-height:1.5;">${formatChangelogNotes(r.body || '')}</div>
            </div>`;
    }).join('');
}

async function openChangelog(forceRefresh = false) {
    document.getElementById('changelog-modal').classList.add('active');
    invoke('get_config').catch(() => ({})).then(cfg => {
        const btn = document.getElementById('toggle-changelog-startup');
        if (btn) btn.textContent = cfg.changelog_on_startup === false ? 'Show on startup' : "Don't show on startup";
    });
    const body = document.getElementById('changelog-body');

    if (!forceRefresh) {
        try {
            const cached = JSON.parse(localStorage.getItem(CHANGELOG_CACHE_KEY) || 'null');
            if (cached && cached.releases && (Date.now() - cached.ts) < CHANGELOG_CACHE_TTL) {
                renderChangelog(cached.releases);
                return;
            }
        } catch {}
    }

    try {
        let releases = null;
        try {
            const res = await fetch('https://api.velocityrl.tech/v2/changelog');
            if (res.ok) {
                const data = await res.json();
                if (Array.isArray(data)) releases = data;
                else if (Array.isArray(data?.releases)) releases = data.releases;
            }
        } catch (_) {}

        if (!releases) {
            const ghRes = await fetch('https://api.github.com/repos/bitsfdb/VelocityRL/releases?per_page=50');
            if (ghRes.ok) releases = await ghRes.json();
        }

        if (releases && Array.isArray(releases) && releases.length) {
            try {
                localStorage.setItem(CHANGELOG_CACHE_KEY, JSON.stringify({ ts: Date.now(), releases }));
            } catch {}
            renderChangelog(releases);
            return;
        }
        throw new Error('No release data found');
    } catch (err) {
        try {
            const cached = JSON.parse(localStorage.getItem(CHANGELOG_CACHE_KEY) || 'null');
            if (cached && cached.releases) {
                renderChangelog(cached.releases);
                return;
            }
        } catch {}

        body.innerHTML = `
            <div class="changelog-release">
                <div class="changelog-release-tag">v2.0.0-alpha.1</div>
                <div class="changelog-release-date">${new Date().toLocaleDateString('en-US', { year:'numeric', month:'long', day:'numeric' })}</div>
                <div class="changelog-release-body">Item swapping, palette support, titles, fake ranks, camera limits, season logo, and PsyNet proxy.</div>
            </div>
            <div style="color:var(--text-secondary);padding:12px 0;font-size:12px;">Could not load changelog. <a href="#" onclick="window.__TAURI__.core.invoke('plugin:shell|open', { path: 'https://api.velocityrl.tech/v2/changelog' }); return false;" style="color:var(--accent-blue);">View on website</a>.</div>`;
    }
}

let workshopCatalogPage = 1;
let workshopCatalogTotalPages = 1;
let workshopCatalogQuery = '';
let isCatalogLoading = false;

async function loadWorkshopCatalog(page = 1, query = '') {
    const grid = document.getElementById('workshop-catalog-grid');
    const pageInfo = document.getElementById('workshop-catalog-page-info');
    const prevBtn = document.getElementById('workshop-catalog-prev-btn');
    const nextBtn = document.getElementById('workshop-catalog-next-btn');
    if (!grid) return;

    workshopCatalogPage = Number(page) || 1;
    workshopCatalogQuery = String(query || '').trim();
    isCatalogLoading = true;

    grid.innerHTML = `
        <div style="grid-column: 1 / -1; text-align: center; padding: 48px 20px; color: var(--text-secondary);">
            <div class="spinner" style="width: 28px; height: 28px; margin: 0 auto 12px; border: 3px solid rgba(255,255,255,0.1); border-top-color: var(--accent-blue); border-radius: 50%; animation: spin 1s linear infinite;"></div>
            <div>Loading maps…</div>
        </div>
    `;

    try {
        const timeoutPromise = new Promise((_, reject) =>
            setTimeout(() => reject(new Error('BakkesPlugins request timed out after 15s. Check internet connection.')), 15000)
        );

        const fetchPromise = invoke('workshop_fetch_bakkes_maps', {
            page: workshopCatalogPage,
            query: workshopCatalogQuery || null,
        });

        const data = await Promise.race([fetchPromise, timeoutPromise]);

        const items = data.items || [];
        workshopCatalogTotalPages = data.totalPages || 1;

        if (pageInfo) pageInfo.textContent = `Page ${data.page || workshopCatalogPage} of ${workshopCatalogTotalPages}`;
        if (prevBtn) prevBtn.disabled = (data.page || workshopCatalogPage) <= 1;
        if (nextBtn) nextBtn.disabled = !data.hasNextPage;

        if (items.length === 0) {
            grid.innerHTML = `
                <div style="grid-column: 1 / -1; text-align: center; padding: 48px 20px; color: var(--text-secondary);">
                    <div style="font-size: 15px; font-weight: 700; color: #fff; margin-bottom: 6px;">No maps found</div>
                    <div style="font-size: 12px;">Try searching for different keywords (e.g. "Rings", "Dribble", "Aerial")</div>
                </div>
            `;
            return;
        }

        grid.innerHTML = '';
        items.forEach(map => {
            const card = document.createElement('div');
            card.className = 'map-catalog-card';

            const sizeMb = map.latestVersionFileSizeBytes ? (map.latestVersionFileSizeBytes / (1024 * 1024)).toFixed(1) + ' MB' : '';
            const banner = map.bannerUrl || '';
            if (banner) {
                try {
                    localStorage.setItem('vrl_map_thumb_' + map.name.toLowerCase().trim(), banner);
                    if (map.id) localStorage.setItem('vrl_map_thumb_id_' + map.id, banner);
                } catch {}
            }
            const author = (map.member && map.member.displayName) ? map.member.displayName : 'Community';
            const desc = map.shortDescription || '';
            const tags = (map.tags || []).slice(0, 3).map(t => `<span class="tracker-badge neutral" style="font-size:10px; padding:2px 8px;">${escHtml(t.shortName || t.key)}</span>`).join('');

            card.innerHTML = `
                <div class="map-catalog-thumb">
                    <span class="map-catalog-thumb-placeholder">MAP</span>
                    ${banner ? `<img src="${escHtml(banner)}" alt="${escHtml(map.name)}" onerror="this.style.opacity='0';">` : ''}
                    ${sizeMb ? `<span class="map-catalog-size-badge">${sizeMb}</span>` : ''}
                </div>
                <div class="map-catalog-content">
                    <div class="map-catalog-title" title="${escHtml(map.name)}">${escHtml(map.name)}</div>
                    <div class="map-catalog-author">by ${escHtml(author)}</div>
                    <div class="map-catalog-desc">${escHtml(desc || 'Rocket League custom workshop map.')}</div>
                    <div class="map-catalog-tags">
                        ${tags}
                    </div>
                    <button type="button" class="action-btn map-catalog-install-btn" data-id="${map.id}" data-name="${escHtml(map.name)}" style="margin-top:8px; width:100%; font-size:12px; padding:7px 0; font-weight:600;">Download & Install</button>
                </div>
            `;
            grid.appendChild(card);
        });

        // Wire install buttons
        grid.querySelectorAll('.map-catalog-install-btn').forEach(btn => {
            btn.onclick = async (e) => {
                e.preventDefault();
                const mapId = btn.dataset.id;
                const mapName = btn.dataset.name;
                const mapItem = items.find(m => String(m.id) === String(mapId));
                const banner = mapItem?.bannerUrl || '';
                btn.disabled = true;
                btn.textContent = 'Fetching version…';

                const progressWrap = document.getElementById('workshop-url-progress');
                const progressFill = document.getElementById('workshop-url-progress-fill');
                const progressText = document.getElementById('workshop-url-progress-text');

                if (progressWrap) {
                    progressWrap.style.display = 'flex';
                    if (progressFill) { progressFill.classList.add('indeterminate'); progressFill.style.width = '100%'; }
                    if (progressText) progressText.textContent = `Resolving download for ${mapName}…`;
                }

                try {
                    const versions = await invoke('workshop_fetch_bakkes_versions', {
                        mapId: parseInt(mapId, 10),
                    });

                    let edgeUrl = null;
                    if (Array.isArray(versions) && versions.length > 0) {
                        edgeUrl = versions[0].edgeUrl || versions[0].url;
                    } else if (versions && typeof versions === 'object') {
                        edgeUrl = versions.edgeUrl || versions.url;
                    }

                    if (!edgeUrl) {
                        throw new Error('No valid download files found for this map.');
                    }

                    btn.textContent = 'Downloading…';
                    if (progressText) progressText.textContent = `Downloading ${mapName}…`;

                    if (edgeUrl.toLowerCase().endsWith('.zip')) {
                        const imported = await invoke('workshop_import_bakkes_zip', { zipUrl: edgeUrl });
                        await invoke('workshop_install_custom_map', {
                            sourcePath: imported.path,
                            name: mapName,
                        });
                        if (banner) {
                            try {
                                localStorage.setItem('vrl_map_thumb_' + mapName.toLowerCase().trim(), banner);
                                localStorage.setItem('vrl_map_thumb_path_' + imported.path, banner);
                            } catch {}
                            await invoke('workshop_set_map_thumbnail', { pathOrName: imported.path, thumbnailUrl: banner }).catch(() => {});
                        }
                    } else {
                        await invoke('workshop_install_map_from_url', {
                            url: edgeUrl,
                            name: mapName,
                        });
                        if (banner) {
                            try {
                                localStorage.setItem('vrl_map_thumb_' + mapName.toLowerCase().trim(), banner);
                            } catch {}
                            await invoke('workshop_set_map_thumbnail', { pathOrName: mapName, thumbnailUrl: banner }).catch(() => {});
                        }
                    }

                    showToast(`"${mapName}" installed! In Rocket League: select Underpass in Free Play or Exhibition to play.`, 'success');
                    btn.textContent = 'Installed ✓';
                    btn.style.background = '#1b5e20';
                    await refreshWorkshopInstalled();
                } catch (err) {
                    showToast(`Download failed: ${err.message || err}`, 'error');
                    btn.disabled = false;
                    btn.textContent = 'Download & Install';
                } finally {
                    if (progressWrap) {
                        setTimeout(() => { progressWrap.style.display = 'none'; }, 2000);
                    }
                }
            };
        });

    } catch (e) {
        grid.innerHTML = `
            <div style="grid-column: 1 / -1; text-align: center; padding: 48px 20px; color: var(--text-secondary);">
                <div style="font-size: 14px; color: #ff5252; margin-bottom: 8px;">Failed to load BakkesPlugins map catalog</div>
                <div style="font-size: 12px; margin-bottom: 12px;">${escHtml(e.message || e)}</div>
                <button type="button" class="action-btn action-btn-secondary" id="workshop-catalog-retry-btn">Retry</button>
            </div>
        `;
        document.getElementById('workshop-catalog-retry-btn')?.addEventListener('click', () => loadWorkshopCatalog(1, ''));
    } finally {
        isCatalogLoading = false;
    }
}

async function initWorkshopTab() {
    if (!initWorkshopTab._wired) {
        initWorkshopTab._wired = true;

        const searchInput = document.getElementById('workshop-catalog-search');
        const searchBtn = document.getElementById('workshop-catalog-search-btn');
        const prevBtn = document.getElementById('workshop-catalog-prev-btn');
        const nextBtn = document.getElementById('workshop-catalog-next-btn');

        if (searchBtn && searchInput) {
            const doSearch = () => loadWorkshopCatalog(1, searchInput.value);
            searchBtn.onclick = doSearch;
            searchInput.onkeydown = (e) => { if (e.key === 'Enter') doSearch(); };
        }

        if (prevBtn) {
            prevBtn.onclick = () => {
                if (workshopCatalogPage > 1) {
                    loadWorkshopCatalog(workshopCatalogPage - 1, workshopCatalogQuery);
                }
            };
        }

        if (nextBtn) {
            nextBtn.onclick = () => {
                if (workshopCatalogPage < workshopCatalogTotalPages) {
                    loadWorkshopCatalog(workshopCatalogPage + 1, workshopCatalogQuery);
                }
            };
        }

        const progressWrap = document.getElementById('workshop-url-progress');
        const progressFill = document.getElementById('workshop-url-progress-fill');
        const progressText = document.getElementById('workshop-url-progress-text');

        window.__TAURI__?.event?.listen('map-download-progress', (evt) => {
            const p = evt?.payload || {};
            if (!progressWrap) return;
            progressWrap.style.display = 'flex';
            const indeterminate = !(typeof p.percent === 'number' && p.percent >= 0);
            if (progressFill) {
                progressFill.classList.toggle('indeterminate', indeterminate);
                progressFill.style.width = indeterminate ? '100%' : `${p.percent}%`;
            }
            const pctEl = document.getElementById('workshop-url-progress-pct');
            if (progressText) {
                if (p.phase === 'install') {
                    progressText.textContent = 'Installing map…';
                    if (pctEl) pctEl.textContent = '';
                    return;
                }
                const mb = (p.downloaded / (1024 * 1024)).toFixed(1);
                progressText.textContent = indeterminate
                    ? `Downloading map… ${mb} MB`
                    : p.total > 0
                        ? `Downloading map… ${p.percent}% — ${(p.downloaded / (1024 * 1024)).toFixed(1)} / ${(p.total / (1024 * 1024)).toFixed(1)} MB`
                        : `Downloading map… ${mb} MB`;
            }
            if (pctEl) pctEl.textContent = indeterminate ? '' : `${p.percent}%`;
        });

        const urlInstall = async () => {
            const inp = document.getElementById('workshop-url-input');
            const url = (inp?.value || '').trim();
            if (!url) { showToast('Paste a direct .upk or .zip URL first.', 'error'); return; }
            const btn = document.getElementById('workshop-url-btn');
            if (btn) { btn.disabled = true; btn.textContent = 'Downloading…'; }
            if (progressWrap) {
                progressWrap.style.display = 'flex';
                if (progressFill) progressFill.style.width = '0%';
                if (progressText) progressText.textContent = 'Starting…';
            }
            try {
                if (url.toLowerCase().endsWith('.zip')) {
                    const imported = await invoke('workshop_import_bakkes_zip', { zipUrl: url });
                    await invoke('workshop_install_custom_map', {
                        sourcePath: imported.path,
                        name: null,
                    });
                } else {
                    await invoke('workshop_install_map_from_url', { url, name: null });
                }
                showToast('Map loaded! Join Underpass in Rocket League (Free Play or Exhibition) to play.', 'success');
                if (inp) inp.value = '';
                await refreshWorkshopInstalled();
                await refreshWorkshopLibrary();
            } catch (e) {
                showToast(String(e), 'error');
            } finally {
                if (btn) { btn.disabled = false; btn.textContent = 'Install from URL'; }
                setTimeout(() => { if (progressWrap) progressWrap.style.display = 'none'; }, 1500);
            }
        };

        document.getElementById('workshop-url-btn')?.addEventListener('click', urlInstall);
        document.getElementById('workshop-url-input')?.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') urlInstall();
        });

        document.getElementById('workshop-restore-btn')?.addEventListener('click', async () => {
            try {
                await invoke('workshop_restore_original_map');
                showToast('Original Underpass map restored.', 'success');
                await refreshWorkshopInstalled();
            } catch (e) {
                showToast(String(e), 'error');
            }
        });

        const dropArea = document.getElementById('workshop-drop-area');
        const pickAndInstall = async () => {
            const picked = await open({
                multiple: false,
                directory: false,
                title: 'Pick a map .upk file',
                filters: [{ name: 'Unreal package', extensions: ['upk'] }],
            });
            if (!picked) return;
            const name = await appDialog({
                title: 'Name this map',
                message: 'What should this map show as in VelocityRL?',
                input: picked.split(/[\\/]/).pop().replace(/\.upk$/i, ''),
                okLabel: 'Install',
            });

            if (name === null) return;
            try {
                const inst = await invoke('workshop_install_custom_map', {
                    sourcePath: picked,
                    name: (name || '').trim() || null,
                });
                showToast(`"${inst.map_name}" loaded! Join Underpass in Rocket League (Free Play or Exhibition) to play.`, 'success');
                await refreshWorkshopInstalled();
                await refreshWorkshopLibrary();
            } catch (e) {
                showToast(String(e), 'error');
            }
        };
        dropArea?.addEventListener('click', pickAndInstall);

        document.getElementById('workshop-add-map-btn')?.addEventListener('click', pickAndInstall);

        document.getElementById('workshop-preset-save-btn')?.addEventListener('click', async () => {
            const name = await appDialog({ title: 'Save map preset', message: 'Map preset name:', input: 'My map', okLabel: 'Save' });
            if (!name || !name.trim()) return;
            try {
                const preset = await invoke('workshop_save_map_preset', { name: name.trim() });
                showToast(`Map preset <strong>${escHtml(preset.name)}</strong> saved.`, 'success');
                refreshWorkshopPresets();
            } catch (e) {
                showToast(String(e), 'error');
            }
        });

        window.__TAURI__?.event?.listen('bakkes-map-imported', async (evt) => {
            const p = evt?.payload || {};
            const progressWrap = document.getElementById('workshop-url-progress');
            const progressFill = document.getElementById('workshop-url-progress-fill');
            if (progressWrap) {
                if (progressFill) progressFill.classList.remove('indeterminate');
                setTimeout(() => { progressWrap.style.display = 'none'; }, 1200);
            }
            if (p.error) showToast(`Map import failed: ${p.error}`, 'error');
            else showToast(`Map "${p.name}" added to your custom maps.`, 'success');
            await refreshWorkshopLibrary();
        });

        window.__TAURI__?.event?.listen('workshop-map-restored', async () => {
            showToast('Game closed — original Underpass restored automatically.', 'info');
            await refreshWorkshopInstalled();
        });
    }

    loadWorkshopCatalog(workshopCatalogPage, workshopCatalogQuery);
    await refreshWorkshopLibrary();
    await refreshWorkshopInstalled();
    await refreshWorkshopPresets();
}

function resolveThumbnailSrc(src) {
    if (!src) return '';
    if (src.startsWith('http://') || src.startsWith('https://') || src.startsWith('data:')) return src;
    if (window.__TAURI__?.core?.convertFileSrc) {
        return window.__TAURI__.core.convertFileSrc(src);
    }
    return src;
}

function getMapThumbnailUrl(name, path, explicitThumb) {
    if (explicitThumb) return explicitThumb;
    if (path) {
        const pThumb = localStorage.getItem('vrl_map_thumb_path_' + path);
        if (pThumb) return pThumb;
    }
    if (name) {
        const cleanName = name.toLowerCase().trim();
        const nThumb = localStorage.getItem('vrl_map_thumb_' + cleanName);
        if (nThumb) return nThumb;
        for (let i = 0; i < localStorage.length; i++) {
            const k = localStorage.key(i);
            if (k && k.startsWith('vrl_map_thumb_')) {
                const sub = k.replace('vrl_map_thumb_', '');
                if (sub && (cleanName.includes(sub) || sub.includes(cleanName.slice(0, 10)))) {
                    return localStorage.getItem(k);
                }
            }
        }
    }
    return '';
}

async function autoFetchMissingThumbnail(mapName) {
    if (!mapName) return null;
    const clean = mapName.toLowerCase().trim();
    if (localStorage.getItem('vrl_map_thumb_' + clean)) return localStorage.getItem('vrl_map_thumb_' + clean);
    try {
        const firstWord = mapName.replace(/[^a-zA-Z0-9 ]/g, ' ').trim().split(' ')[0];
        const res = await invoke('workshop_fetch_bakkes_maps', { page: 1, query: firstWord });
        const items = res?.items || [];
        const match = items.find(m => m.name.toLowerCase().includes(clean) || clean.includes(m.name.toLowerCase()) || m.name.toLowerCase().startsWith(clean.slice(0, 8)));
        if (match && match.bannerUrl) {
            localStorage.setItem('vrl_map_thumb_' + clean, match.bannerUrl);
            return match.bannerUrl;
        }
    } catch {}
    return null;
}

async function refreshWorkshopLibrary() {
    const list = document.getElementById('workshop-library-list');
    if (!list) return;
    list.innerHTML = '';

    const guideBanner = document.createElement('div');
    guideBanner.className = 'map-how-to-play-banner';
    guideBanner.innerHTML = `
        <div class="map-how-to-play-icon">ℹ️</div>
        <div class="map-how-to-play-text">
            <strong>How to play your custom map in Rocket League:</strong><br>
            Custom maps replace the <em>Underpass</em> arena. After loading a map below, start Rocket League and join:
            <span style="display:block;margin-top:2px;">• <strong>Play → Training → Free Play → Underpass</strong></span>
            <span style="display:block;">• Or <strong>Play → Custom Games → Exhibition Match → Arena: Underpass</strong></span>
        </div>
    `;
    list.appendChild(guideBanner);

    let installed = null;
    try { installed = await invoke('workshop_get_installed'); } catch {}

    const replacedWrap = document.createElement('div');
    replacedWrap.style.cssText = 'margin-bottom:14px;';
    const replacedTitle = document.createElement('p');
    replacedTitle.className = 'switch-title';
    replacedTitle.style.cssText = 'margin:0 0 6px 0;font-weight:600;font-size:12px;';
    replacedTitle.textContent = 'Replaced in-game';
    replacedWrap.appendChild(replacedTitle);

    if (installed) {
        const row = document.createElement('div');
        row.className = 'backup-item';
        row.style.cssText = 'display:flex;align-items:center;gap:12px;border:1px solid var(--accent-blue);padding:8px 12px;border-radius:6px;background:#181818;';

        const thumbWrap = document.createElement('div');
        thumbWrap.className = 'map-item-thumb-wrap';
        let thumbUrl = getMapThumbnailUrl(installed.map_name, installed.source_path, installed.thumbnail_url);
        if (thumbUrl) {
            const img = document.createElement('img');
            img.src = resolveThumbnailSrc(thumbUrl);
            img.className = 'map-item-thumb';
            img.alt = installed.map_name;
            img.onerror = () => { img.style.display = 'none'; fallback.style.display = 'flex'; };
            thumbWrap.appendChild(img);
        }
        const fallback = document.createElement('div');
        fallback.className = 'map-item-thumb-fallback';
        fallback.textContent = 'MAP';
        if (thumbUrl) fallback.style.display = 'none';
        thumbWrap.appendChild(fallback);
        row.appendChild(thumbWrap);

        if (!thumbUrl && installed.map_name) {
            autoFetchMissingThumbnail(installed.map_name).then(fetched => {
                if (fetched) {
                    fallback.style.display = 'none';
                    let img = thumbWrap.querySelector('img');
                    if (!img) {
                        img = document.createElement('img');
                        img.className = 'map-item-thumb';
                        img.alt = installed.map_name;
                        thumbWrap.appendChild(img);
                    }
                    img.src = resolveThumbnailSrc(fetched);
                    img.style.display = 'block';
                }
            });
        }

        const info = document.createElement('div');
        info.style.cssText = 'flex:1;min-width:0;';
        const nm = document.createElement('p');
        nm.className = 'switch-title';
        nm.style.cssText = 'margin:0;font-weight:700;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;display:flex;align-items:center;';
        nm.textContent = installed.map_name;

        const activeBadge = document.createElement('span');
        activeBadge.className = 'map-active-pill';
        activeBadge.textContent = 'ACTIVE';
        nm.appendChild(activeBadge);

        const sub = document.createElement('p');
        sub.className = 'field-hint';
        sub.style.cssText = 'margin:2px 0 0 0;font-size:11px;color:var(--text-secondary);';
        sub.innerHTML = '<span style="color:var(--accent-blue);font-weight:600;">Active in-game:</span> Join <strong style="color:#fff;">Underpass</strong> in Free Play or Exhibition to play.';
        info.appendChild(nm);
        info.appendChild(sub);

        const restoreBtn = document.createElement('button');
        restoreBtn.className = 'action-btn action-btn-secondary';
        restoreBtn.type = 'button';
        restoreBtn.textContent = 'Restore Original';
        restoreBtn.addEventListener('click', async () => {
            restoreBtn.disabled = true;
            try {
                await invoke('workshop_restore');
                showToast('Original Underpass restored.', 'success');
                await refreshWorkshopLibrary();
                await refreshWorkshopInstalled();
            } catch (e) {
                showToast(String(e), 'error');
                restoreBtn.disabled = false;
            }
        });
        row.appendChild(info);
        row.appendChild(restoreBtn);
        replacedWrap.appendChild(row);
    } else {
        const empty = document.createElement('div');
        empty.className = 'backup-empty';
        empty.textContent = 'Nothing replaced — the game is using the original Underpass.';
        replacedWrap.appendChild(empty);
    }
    list.appendChild(replacedWrap);

    const dlTitle = document.createElement('p');
    dlTitle.className = 'switch-title';
    dlTitle.style.cssText = 'margin:0 0 6px 0;font-weight:600;font-size:12px;';
    dlTitle.textContent = 'Downloaded maps';
    list.appendChild(dlTitle);

    let lib = [];
    try { lib = await invoke('workshop_get_map_library'); } catch (e) {
        const err = document.createElement('div');
        err.className = 'backup-empty';
        err.textContent = `Could not load library: ${String(e)}`;
        list.appendChild(err);
        return;
    }
    if (!lib.length) {
        const empty = document.createElement('div');
        empty.className = 'backup-empty';
        empty.textContent = 'No custom maps yet. Grab one from the Maps catalog or click "Add map file".';
        list.appendChild(empty);
        return;
    }
    for (const entry of lib) {
        const row = document.createElement('div');
        row.className = 'backup-item';
        row.style.cssText = 'display:flex;align-items:center;gap:12px;padding:8px 12px;border-radius:6px;border:1px solid var(--border);background:#181818;margin-bottom:6px;';

        const thumbWrap = document.createElement('div');
        thumbWrap.className = 'map-item-thumb-wrap';
        let thumbUrl = getMapThumbnailUrl(entry.name, entry.path, entry.thumbnail_url);
        if (thumbUrl) {
            const img = document.createElement('img');
            img.src = resolveThumbnailSrc(thumbUrl);
            img.className = 'map-item-thumb';
            img.alt = entry.name;
            img.onerror = () => { img.style.display = 'none'; fallback.style.display = 'flex'; };
            thumbWrap.appendChild(img);
        }
        const fallback = document.createElement('div');
        fallback.className = 'map-item-thumb-fallback';
        fallback.textContent = 'MAP';
        if (thumbUrl) fallback.style.display = 'none';
        thumbWrap.appendChild(fallback);
        row.appendChild(thumbWrap);

        if (!thumbUrl && entry.name) {
            autoFetchMissingThumbnail(entry.name).then(fetched => {
                if (fetched) {
                    fallback.style.display = 'none';
                    let img = thumbWrap.querySelector('img');
                    if (!img) {
                        img = document.createElement('img');
                        img.className = 'map-item-thumb';
                        img.alt = entry.name;
                        thumbWrap.appendChild(img);
                    }
                    img.src = resolveThumbnailSrc(fetched);
                    img.style.display = 'block';
                }
            });
        }

        const info = document.createElement('div');
        info.style.cssText = 'flex:1;min-width:0;';
        const nm = document.createElement('p');
        nm.className = 'switch-title';
        nm.style.cssText = 'margin:0;font-weight:600;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;display:flex;align-items:center;';
        nm.textContent = entry.name;
        nm.title = entry.path;

        const active = installed && installed.source_path &&
            installed.source_path.toLowerCase() === String(entry.path).toLowerCase();
        if (active) {
            const badge = document.createElement('span');
            badge.className = 'map-active-pill';
            badge.textContent = 'LOADED (UNDERPASS)';
            nm.appendChild(badge);
        }
        info.appendChild(nm);
        const loadBtn = document.createElement('button');
        loadBtn.className = 'action-btn';
        loadBtn.type = 'button';
        loadBtn.textContent = active ? 'Reload' : 'Load';
        loadBtn.addEventListener('click', async () => {
            loadBtn.disabled = true;
            try {
                const inst = await invoke('workshop_install_from_library', { path: entry.path });
                showToast(`"${inst.map_name}" loaded! Join Underpass in Rocket League (Free Play or Exhibition) to play.`, 'success');
                await refreshWorkshopLibrary();
                await refreshWorkshopInstalled();
            } catch (e) {
                showToast(String(e), 'error');
            } finally {
                loadBtn.disabled = false;
            }
        });
        const delBtn = document.createElement('button');
        delBtn.className = 'action-btn action-btn-secondary';
        delBtn.type = 'button';
        delBtn.textContent = 'Remove';
        delBtn.addEventListener('click', async () => {
            try {
                await invoke('workshop_remove_from_library', { path: entry.path });
                refreshWorkshopLibrary();
            } catch (e) {
                showToast(String(e), 'error');
            }
        });
        row.appendChild(info);
        row.appendChild(loadBtn);
        row.appendChild(delBtn);
        list.appendChild(row);
    }
}

async function refreshWorkshopPresets() {
    const list = document.getElementById('workshop-preset-list');
    if (!list) return;
    try {
        const presets = await invoke('workshop_get_map_presets');
        if (!presets.length) {
            list.innerHTML = '<div class="backup-empty">No map presets yet. Install a map, then click "Save loaded map as preset".</div>';
            return;
        }
        list.innerHTML = '';
        for (const p of presets) {
            const row = document.createElement('div');
            row.className = 'backup-item';
            row.style.cssText = 'display:flex;align-items:center;gap:10px;';
            const info = document.createElement('div');
            info.style.cssText = 'flex:1;min-width:0;';
            const nm = document.createElement('p');
            nm.className = 'switch-title';
            nm.style.cssText = 'margin:0;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
            nm.textContent = p.name;
            nm.title = p.name;
            const sub = document.createElement('p');
            sub.className = 'field-hint';
            sub.style.cssText = 'margin:0;font-size:11px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
            sub.textContent = p.map_name;
            sub.title = p.map_name;
            info.appendChild(nm);
            info.appendChild(sub);
            const applyBtn = document.createElement('button');
            applyBtn.className = 'action-btn';
            applyBtn.type = 'button';
            applyBtn.textContent = 'Load';
            applyBtn.addEventListener('click', async () => {
                applyBtn.disabled = true;
                try {
                    const msg = await invoke('workshop_apply_map_preset', { id: p.id });
                    showToast(msg, 'success');
                    await refreshWorkshopInstalled();
                } catch (e) {
                    showToast(String(e), 'error');
                } finally {
                    applyBtn.disabled = false;
                }
            });
            const delBtn = document.createElement('button');
            delBtn.className = 'action-btn action-btn-secondary';
            delBtn.type = 'button';
            delBtn.textContent = 'Delete';
            delBtn.addEventListener('click', async () => {
                try {
                    await invoke('workshop_delete_map_preset', { id: p.id });
                    refreshWorkshopPresets();
                } catch (e) {
                    showToast(String(e), 'error');
                }
            });
            row.appendChild(info);
            row.appendChild(applyBtn);
            row.appendChild(delBtn);
            list.appendChild(row);
        }
    } catch (e) {
        list.innerHTML = `<div class="backup-empty">Failed to load map presets: ${escHtml(String(e))}</div>`;
    }
}

async function refreshWorkshopInstalled() {
    const status = document.getElementById('workshop-installed-status');
    if (!status) return;
    let inst = null;
    try { inst = await invoke('workshop_get_installed'); } catch {}
    if (inst) {
        status.style.display = 'block';
        status.textContent = `Loaded map: ${inst.map_name}`;
    } else {
        status.style.display = 'none';
    }
}

async function searchWorkshopMaps() {
    const input = document.getElementById('workshop-search-input');
    const results = document.getElementById('workshop-results');
    if (!input || !results) return;
    const q = input.value.trim();
    results.innerHTML = '<p class="field-hint">Searching…</p>';
    try {
        const maps = await invoke('workshop_search_maps', { query: q });
        results.innerHTML = '';
        if (!maps.length) {
            results.innerHTML = '<p class="field-hint">No maps found.</p>';
            return;
        }
        const grid = document.createElement('div');
        grid.style.cssText = 'display:grid;grid-template-columns:repeat(3, minmax(0, 1fr));gap:10px;';
        for (const m of maps) {
            const card = document.createElement('div');
            card.style.cssText = 'border:1px solid var(--border);border-radius:10px;overflow:hidden;display:flex;flex-direction:column;background:rgba(255,255,255,0.02);';
            if (m.preview_url) {
                const img = document.createElement('img');
                img.src = m.preview_url;
                img.alt = m.name;
                img.loading = 'lazy';
                img.style.cssText = 'width:100%;aspect-ratio:16/9;object-fit:cover;display:block;background:#1a1a1a;';
                img.onerror = () => img.remove();
                card.appendChild(img);
            }
            const body = document.createElement('div');
            body.style.cssText = 'padding:8px 10px 10px;display:flex;flex-direction:column;gap:2px;flex:1;min-width:0;';
            const nm = document.createElement('div');
            nm.style.cssText = 'font-weight:600;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
            nm.textContent = m.name;
            nm.title = m.name;
            const sub = document.createElement('div');
            sub.className = 'field-hint';
            sub.style.cssText = 'margin:0;font-size:11px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
            const sizeTxt = m.size_bytes ? ` — ${(m.size_bytes / (1024 * 1024)).toFixed(1)} MB` : '';
            sub.textContent = [m.author, m.id + sizeTxt].filter(Boolean).join(' — ');
            sub.title = sub.textContent;
            const btn = document.createElement('button');
            btn.className = 'action-btn';
            btn.type = 'button';
            btn.style.cssText = 'margin-top:8px;width:100%;';
            btn.textContent = 'Install';
            btn.addEventListener('click', async () => {
                btn.disabled = true;
                btn.textContent = 'Installing…';
                try {
                    await invoke('workshop_install_map', { mapId: m.id, mapName: m.name });
                    showToast(`"${m.name}" loaded. Start an Underpass match in-game.`, 'success');
                    await refreshWorkshopInstalled();
                } catch (e) {
                    showToast(String(e), 'error');
                } finally {
                    btn.disabled = false;
                    btn.textContent = 'Install';
                }
            });
            body.appendChild(nm);
            body.appendChild(sub);
            body.appendChild(btn);
            card.appendChild(body);
            grid.appendChild(card);
        }
        results.appendChild(grid);
    } catch (e) {
        results.innerHTML = '';
        const msg = String(e);
        results.innerHTML = `<p class="field-hint">${escHtml(msg)}</p>`;
    }
}

let trackerSessionCache = null;
let isOverlayLocked = true;

async function initTrackerModule() {
    try {
        const config = await invoke('get_config').catch(() => ({ game_dir: '' }));
        if (config && config.game_dir) {
            invoke('tracker_ensure_stats_api', { gameDir: config.game_dir }).catch(() => {});
        }
        const session = await invoke('tracker_load_session');
        trackerSessionCache = session;
        if (typeof session.is_locked === 'boolean') {
            isOverlayLocked = session.is_locked;
        }
        bindTrackerEvents();
    } catch (e) {
        console.warn('initTrackerModule error:', e);
    }
}

async function initTrackerTab() {
    try {
        const session = await invoke('tracker_load_session');
        trackerSessionCache = session;
        if (typeof session.is_locked === 'boolean') {
            isOverlayLocked = session.is_locked;
        }

        const masterSw = document.getElementById('tracker-master-enabled');
        const posSel = document.getElementById('tracker-position-select');
        const scaleSlider = document.getElementById('tracker-scale-slider');
        const scaleInput = document.getElementById('tracker-scale-input');
        const opacitySlider = document.getElementById('tracker-opacity-slider');
        const opacityInput = document.getElementById('tracker-opacity-input');
        const lockBtn = document.getElementById('tracker-toggle-lock-btn');

        const winDeltaInput = document.getElementById('tracker-win-delta-input');
        const lossDeltaInput = document.getElementById('tracker-loss-delta-input');

        if (masterSw) masterSw.checked = session.master_enabled !== false;
        if (posSel) posSel.value = session.position.startsWith('custom:') ? 'custom' : (session.position || 'top-right');
        if (scaleSlider) scaleSlider.value = session.scale || 100;
        if (scaleInput) scaleInput.value = session.scale || 100;
        if (opacitySlider) opacitySlider.value = session.opacity || 85;
        if (opacityInput) opacityInput.value = session.opacity || 85;
        if (winDeltaInput) winDeltaInput.value = session.win_delta || 9;
        if (lossDeltaInput) lossDeltaInput.value = session.loss_delta || 9;
        if (lockBtn) {
            lockBtn.textContent = isOverlayLocked ? 'Unlock Position to Move' : 'Lock Overlay Position';
            lockBtn.className = isOverlayLocked ? 'action-btn action-btn-secondary' : 'action-btn';
        }

        const activeStyle = session.overlay_style || 'circle';
        document.querySelectorAll('.theme-style-btn, .tracker-style-opt').forEach(btn => {
            btn.classList.toggle('active', btn.dataset.style === activeStyle);
            btn.classList.toggle('is-active', btn.dataset.style === activeStyle);
        });

        bindTrackerEvents();
    } catch (e) {
        console.warn('initTrackerTab error:', e);
    }
}

function bindTrackerEvents() {
    if (bindTrackerEvents._wired) return;
    bindTrackerEvents._wired = true;

    const masterSw = document.getElementById('tracker-master-enabled');
    const posSel = document.getElementById('tracker-position-select');
    const lockBtn = document.getElementById('tracker-toggle-lock-btn');
    const testBtn = document.getElementById('tracker-test-preview-btn');
    const centerBtn = document.getElementById('tracker-center-overlay-btn');
    const closeBtn = document.getElementById('tracker-close-overlay-btn');
    const resetSessionBtn = document.getElementById('tracker-reset-session-btn');

    if (resetSessionBtn) {
        resetSessionBtn.onclick = async (e) => {
            e.preventDefault();
            try {
                await invoke('reset_session');
                trackerSessionCache = await invoke('tracker_load_session').catch(() => null);
                showToast('Tracker session stats reset (MMR ±0, Wins 0, Losses 0, Streak 0).', 'success');
            } catch (err) {
                showToast(`Could not reset session: ${err.message || err}`, 'error');
            }
        };
    }

    if (lockBtn) {
        lockBtn.onclick = async (e) => {
            e.preventDefault();
            isOverlayLocked = !isOverlayLocked;
            lockBtn.textContent = isOverlayLocked ? 'Unlock Position to Move' : 'Lock Overlay Position';
            lockBtn.className = isOverlayLocked ? 'action-btn action-btn-secondary' : 'action-btn';
            if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
            if (trackerSessionCache.master_enabled) {
                await invoke('tracker_open_overlay_window', { style: trackerSessionCache.overlay_style || 'circle' }).catch(() => {});
            }
            await invoke('tracker_set_overlay_locked', { locked: isOverlayLocked }).catch(() => {});
            showToast(isOverlayLocked ? 'Overlay locked in position for gameplay.' : 'Overlay unlocked! Drag it anywhere on your screen.', 'success');
        };

        window.__TAURI__?.event?.listen('tracker-overlay-locked', (evt) => {
            if (typeof evt?.payload === 'boolean') {
                isOverlayLocked = evt.payload;
                lockBtn.textContent = isOverlayLocked ? 'Unlock Position to Move' : 'Lock Overlay Position';
                lockBtn.className = isOverlayLocked ? 'action-btn action-btn-secondary' : 'action-btn';
            }
        });
    }

    if (testBtn) {
        testBtn.onclick = async (e) => {
            e.preventDefault();
            try {
                if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
                await invoke('tracker_open_overlay_window', { style: trackerSessionCache.overlay_style || 'circle' });
                await invoke('tracker_set_overlay_locked', { locked: false });
                showToast('Tracker overlay opened on screen in position mode.', 'success');
            } catch (err) {
                console.error('testBtn overlay launch failed:', err);
                showToast(`Could not open overlay: ${err.message || err}`, 'error');
            }
        };
    }

    if (centerBtn) {
        centerBtn.onclick = async (e) => {
            e.preventDefault();
            if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
            await invoke('tracker_open_overlay_window', { style: trackerSessionCache.overlay_style || 'circle' }).catch(() => {});
            await invoke('tracker_center_overlay').catch(() => {});
            showToast('Tracker overlay centered on primary screen.', 'success');
        };
    }

    if (closeBtn) {
        closeBtn.onclick = async (e) => {
            e.preventDefault();
            await invoke('tracker_close_overlay_window').catch(() => {});
            showToast('Tracker overlay hidden.', 'info');
        };
    }

    if (masterSw) {
        masterSw.onchange = async () => {
            if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
            trackerSessionCache.master_enabled = masterSw.checked;
            await saveTrackerSession(trackerSessionCache);
            if (trackerSessionCache.master_enabled) {
                await invoke('tracker_open_overlay_window', { style: trackerSessionCache.overlay_style || 'circle' }).catch(() => {});

                try {
                    const config = await invoke('get_config').catch(() => ({ game_dir: '' }));
                    if (config && config.game_dir) {
                        const rewritten = await invoke('tracker_ensure_stats_api', { gameDir: config.game_dir });
                        if (rewritten) {
                            showToast('Stats API files updated — restart Rocket League for changes to apply.', 'success');
                        } else {
                            showToast('Stats API already configured ✓', 'info');
                        }
                    } else {
                        showToast('Set your Rocket League path in Settings so Stats API can be configured.', 'error');
                    }
                } catch (err) {
                    showToast(`Stats API check failed: ${err.message || err}`, 'error');
                }
            } else {
                await invoke('tracker_close_overlay_window').catch(() => {});
            }
        };
    }

    if (posSel) {
        posSel.onchange = async () => {
            if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
            trackerSessionCache.position = posSel.value;
            await saveTrackerSession(trackerSessionCache);
            await invoke('tracker_position_overlay', { pos: posSel.value }).catch(() => {});
        };
    }

    const scaleSlider = document.getElementById('tracker-scale-slider');
    const scaleInput = document.getElementById('tracker-scale-input');
    const opacitySlider = document.getElementById('tracker-opacity-slider');
    const opacityInput = document.getElementById('tracker-opacity-input');

    const updateScale = async (val) => {
        const num = Math.max(1, Math.min(250, parseInt(val, 10) || 100));
        if (scaleSlider) scaleSlider.value = num;
        if (scaleInput) scaleInput.value = num;
        if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
        trackerSessionCache.scale = num;
        invoke('tracker_apply_scale_opacity', {
            scale: num,
            opacity: trackerSessionCache.opacity ?? 85
        }).catch(() => {});
        await saveTrackerSession(trackerSessionCache);
    };

    const updateOpacity = async (val) => {
        const num = Math.max(10, Math.min(100, parseInt(val, 10) || 85));
        if (opacitySlider) opacitySlider.value = num;
        if (opacityInput) opacityInput.value = num;
        if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
        trackerSessionCache.opacity = num;
        invoke('tracker_apply_scale_opacity', {
            scale: trackerSessionCache.scale ?? 100,
            opacity: num
        }).catch(() => {});
        await saveTrackerSession(trackerSessionCache);
    };

    if (scaleSlider) scaleSlider.oninput = (e) => updateScale(e.target.value);
    if (scaleInput) {
        scaleInput.oninput = (e) => updateScale(e.target.value);
        scaleInput.onchange = (e) => updateScale(e.target.value);
    }
    document.querySelectorAll('.scale-preset-btn').forEach(btn => {
        btn.onclick = () => updateScale(btn.dataset.scale);
    });

    if (opacitySlider) opacitySlider.oninput = (e) => updateOpacity(e.target.value);
    if (opacityInput) {
        opacityInput.oninput = (e) => updateOpacity(e.target.value);
        opacityInput.onchange = (e) => updateOpacity(e.target.value);
    }
    document.querySelectorAll('.opacity-preset-btn').forEach(btn => {
        btn.onclick = () => updateOpacity(btn.dataset.opacity);
    });

    const winDeltaInput = document.getElementById('tracker-win-delta-input');
    const lossDeltaInput = document.getElementById('tracker-loss-delta-input');
    const updateDeltas = async () => {
        if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
        if (winDeltaInput) trackerSessionCache.win_delta = Math.max(1, parseInt(winDeltaInput.value, 10) || 9);
        if (lossDeltaInput) trackerSessionCache.loss_delta = Math.max(1, parseInt(lossDeltaInput.value, 10) || 9);
        await saveTrackerSession(trackerSessionCache);
    };
    if (winDeltaInput) winDeltaInput.onchange = updateDeltas;
    if (lossDeltaInput) lossDeltaInput.onchange = updateDeltas;

    const styleOpts = document.querySelectorAll('.theme-style-btn, .tracker-style-opt');
    styleOpts.forEach(opt => {
        opt.onclick = async (e) => {
            e.preventDefault();
            e.stopPropagation();
            if (!trackerSessionCache) trackerSessionCache = await invoke('tracker_load_session');
            styleOpts.forEach(o => {
                o.classList.remove('active');
                o.classList.remove('is-active');
            });
            opt.classList.add('active');
            opt.classList.add('is-active');
            trackerSessionCache.overlay_style = opt.dataset.style;
            await saveTrackerSession(trackerSessionCache);
            window.__TAURI__?.event?.emit('tracker-style-changed', opt.dataset.style);
            await invoke('tracker_open_overlay_window', { style: opt.dataset.style }).catch(() => {});
        };
    });
}

function sanitizeTrackerName(name, playerId) {
    if (name && !name.startsWith('Epic|') && name.trim()) return name.trim();
    if (playerId && !playerId.startsWith('Epic|') && playerId.trim()) return playerId.trim();
    return 'Player';
}

async function saveTrackerSession(session) {
    try {
        await invoke('tracker_save_session', { session });
    } catch (e) {
        console.warn('saveTrackerSession failed:', e);
    }
}
document.addEventListener('contextmenu', (e) => e.preventDefault());

window.addEventListener('DOMContentLoaded', async () => {

    setTimeout(() => {
        if (appLoading) {
            console.warn('App loading safety watchdog triggered — releasing overlay');
            releaseAppLoading();
        }
    }, 3500);

    document.getElementById('privacy-link')?.addEventListener('click', (e) => {
        e.preventDefault();
        window.__TAURI__?.core?.invoke('open_external_url', { url: PRIVACY_POLICY_URL });
    });

    try {
        await init();
    } catch (err) {
        console.error('Fatal error during init():', err);
        releaseAppLoading();
    }

    initVersionBadge();

    try {
        await initFeatures();
    } catch (err) {
        console.warn('initFeatures non-fatal error:', err);
    }

    try {
        await initTrackerModule();
    } catch (err) {
        console.warn('initTrackerModule non-fatal error:', err);
    }
});

async function initFeatures() {
    try {
        const feat = await invoke('get_features');
        if (!feat) return;

        if (feat.build_outdated) {
            showOutdatedBuildModal(feat);
            return;
        }

        const mainWrap = document.querySelector('.main-wrap');

        // Cleanup any legacy banners erroneously attached directly to body
        document.body.querySelectorAll(':scope > #maintenance-banner, :scope > #announcement-banner').forEach(el => el.remove());

        const existingMBanner = document.getElementById('maintenance-banner');
        if (feat.maintenance?.enabled) {
            let mBanner = existingMBanner;
            if (!mBanner) {
                mBanner = document.createElement('div');
                mBanner.id = 'maintenance-banner';
                mBanner.style.cssText = 'background: rgba(220, 38, 38, 0.95); color: #fff; text-align: center; padding: 10px 16px; font-weight: 600; font-size: 13px; z-index: 9999; display: flex; align-items: center; justify-content: center; gap: 8px; width: 100%; border-bottom: 1px solid rgba(255,255,255,0.15);';
                if (mainWrap) mainWrap.prepend(mBanner);
            }
            mBanner.textContent = feat.maintenance.message || 'VelocityRL servers undergoing routine maintenance.';
        } else if (existingMBanner) {
            existingMBanner.remove();
        }

        const existingABanner = document.getElementById('announcement-banner');
        if (feat.announcement?.active && feat.announcement?.text) {
            let aBanner = existingABanner;
            if (!aBanner) {
                aBanner = document.createElement('div');
                aBanner.id = 'announcement-banner';
                aBanner.style.cssText = 'background: rgba(37, 99, 235, 0.9); color: #fff; text-align: center; padding: 8px 16px; font-weight: 500; font-size: 12px; z-index: 9998; display: flex; align-items: center; justify-content: center; gap: 8px; width: 100%; border-bottom: 1px solid rgba(255,255,255,0.1);';
                if (mainWrap) mainWrap.prepend(aBanner);
            }
            aBanner.textContent = feat.announcement.text;
        } else if (existingABanner) {
            existingABanner.remove();
        }

        if (feat.flags) {
            // Fake Ranks
            const ranksNav = document.querySelector('.nav-item[data-tab="ranks-tab"]');
            if (ranksNav) {
                ranksNav.style.display = feat.flags.fake_ranks === false ? 'none' : '';
            }
            if (feat.flags.fake_ranks === false) {
                const ranksTab = document.getElementById('ranks-tab');
                if (ranksTab && ranksTab.classList.contains('active')) {
                    document.querySelector('.nav-item[data-tab="swapper-tab"]')?.click();
                }
            }

            // Custom Titles
            const titlesNav = document.querySelector('.nav-item[data-tab="titles-tab"]');
            if (titlesNav) {
                titlesNav.style.display = feat.flags.custom_titles === false ? 'none' : '';
            }

            // Camera Spoof
            const cameraNav = document.querySelector('.nav-item[data-tab="camera-tab"]');
            if (cameraNav) {
                cameraNav.style.display = feat.flags.camera_spoof === false ? 'none' : '';
            }

            // Live Tracker Overlay
            const trackerNav = document.querySelector('.nav-item[data-tab="tracker-tab"]');
            if (trackerNav) {
                trackerNav.style.display = feat.flags.live_tracker_overlay === false ? 'none' : '';
            }

            // Item Swapper
            const swapperNav = document.querySelector('.nav-item[data-tab="swapper-tab"]');
            if (swapperNav) {
                swapperNav.style.display = feat.flags.item_swapper === false ? 'none' : '';
            }

            // Workshop upload
            if (feat.flags.workshop_upload_enabled === false) {
                const uploadTab = document.getElementById('workshop-upload-tab');
                if (uploadTab) uploadTab.style.display = 'none';
            }
        }
    } catch (e) {
        console.warn('initFeatures non-fatal error:', e);
    }
}

function showOutdatedBuildModal(feat) {
    const modal = document.getElementById('outdated-build-modal');
    if (!modal) return;

    const clientBuildEl = document.getElementById('outdated-client-build');
    const updateBtn = document.getElementById('outdated-update-btn');
    const websiteBtn = document.getElementById('outdated-website-btn');
    const exitBtn = document.getElementById('outdated-exit-btn');
    const updateStatus = document.getElementById('outdated-update-status');

    if (clientBuildEl) clientBuildEl.textContent = String(feat.client_build_num || 'Unknown');

    modal.classList.add('active');

    const sidebar = document.querySelector('.sidebar');
    const mainWrap = document.querySelector('.main-wrap');
    if (sidebar) sidebar.setAttribute('inert', '');
    if (mainWrap) mainWrap.setAttribute('inert', '');

    if (updateBtn) {
        updateBtn.onclick = async () => {
            updateBtn.disabled = true;
            updateBtn.textContent = 'Checking for updates...';
            if (updateStatus) updateStatus.textContent = 'Contacting update server...';
            try {
                const version = await invoke('check_for_updates');
                if (version) {
                    if (updateStatus) updateStatus.textContent = `Update available: v${version}. Downloading & installing...`;
                    await invoke('install_update');
                    if (updateStatus) updateStatus.textContent = `Update v${version} installed! Restart VelocityRL to apply.`;
                    updateBtn.textContent = 'Restart Now';
                    updateBtn.disabled = false;
                    updateBtn.onclick = () => window.__TAURI__?.core?.invoke('plugin:process|restart');
                } else {
                    if (updateStatus) updateStatus.textContent = 'No automatic update package found. Please download from the website.';
                    updateBtn.disabled = false;
                    updateBtn.textContent = 'Check Again';
                }
            } catch (err) {
                if (updateStatus) updateStatus.textContent = 'Update error: ' + err;
                updateBtn.disabled = false;
                updateBtn.textContent = 'Retry Update';
            }
        };
    }

    if (websiteBtn) {
        websiteBtn.onclick = () => {
            invoke('open_external_url', { url: 'https://velocityrl.tech' }).catch(() => {});
        };
    }

    if (exitBtn) {
        exitBtn.onclick = () => {
            invoke('force_exit').catch(() => {});
        };
    }
}

