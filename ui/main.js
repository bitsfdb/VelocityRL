const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const API_BASE = 'https://api.velocityrl.tech';

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
    const pImg = item.image_url || item.src || '';
    const bgClass = qBgMap[pQuality] || 'bg-common';

    container.innerHTML = `
        <div class="clear-item-btn">×</div>
        ${pImg ? `<img src="${escHtml(pImg)}" class="selected-img" />` : ''}
        <h2>${escHtml(pName)}</h2>
        <span class="quality-badge ${bgClass}">${escHtml(pQuality)}</span>
        <p class="item-slot-label">${escHtml(pSlot)}</p>
    `;
    container.querySelector('.clear-item-btn').addEventListener('click', onClear);
    container.classList.add('selected');
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
    wirePaintSwatches('swap-paint-swatches', 'swap-paint', 'swap-paint-selected');
    syncSwapPaintUi();
    refreshSwapRlHint();

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
            if (btn.dataset.tab === 'camera-tab') initCameraTab();
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
    document.getElementById('version-btn').onclick = () => {
        if (isAppLoading()) return;
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
    document.getElementById('autodetect-dir').onclick = handleAutoDetect;
    document.getElementById('settings-modal').onclick = (e) => {
        if (e.target === document.getElementById('settings-modal')) handleCancelSettings();
    };

    try {
        updateStatus('Verifying Integrity...', false);
        const repair = await invoke('check_integrity').catch(e => {
            throw new Error(`Integrity check failed: ${e}`);
        });
        if (repair && repair.repaired) {
            sessionStorage.setItem('velocityrl_repair_report', JSON.stringify(repair));
        }
        updateStatus('Please Wait...', false);
        items = await invoke('get_items').catch(async (e) => {
            console.warn('API get_items failed, falling back to paginated fetch API...', e);
            return await fetchItemsFromAPI();
        });
        const config = await invoke('get_config').catch(e => { console.warn('Config load failed:', e); return { game_dir: '' }; });
        if (config && config.game_dir) {
            document.getElementById('game-dir').value = config.game_dir;
        } else {
            const installs = await invoke('detect_game_dir').catch(() => []);
            document.getElementById('settings-modal').classList.add('active');
            if (installs.length === 1) {
                document.getElementById('game-dir').value = installs[0].path;
            } else if (installs.length > 1) {
                showInstallChooser(installs);
            }
        }
        updateStatus('bitsfdb', false);
        invoke('cleanup_temp_files').catch(e => console.warn('Cleanup failed:', e));

        if (!nameSpoofForceOffDone) {
            nameSpoofForceOffDone = true;
            await forceNameSpoofOffInConfig();
        }

        await forceInventorySpoofOffInConfig();
        await forcePingSpoofOffInConfig();
        await hydrateSpoofToolsFromDisk();
        await refreshPaletteStatus().catch(() => {});
        attachCloseGuard();
        // Hosts + proxy BEFORE releasing the loading gate so RL is not launched
        // against stock config.psynet.gg (logo/MotD/titles/camera are boot-fetched).
        try {
            const hostsDone = await invoke('ensure_psynet_hosts');
            if (hostsDone === false) {
                // If it returned false, it might have triggered UAC and succeeded, or it was already done.
                // We just log it.
            }
        } catch (e) {
            invoke('append_launch_log', { message: `psynet: boot hosts failed: ${e}` }).catch(() => {});
        }
        await autoStartPsyNetProxy();
        releaseAppLoading();
        checkForUpdates();
        if (config.changelog_on_startup !== false) openChangelog();
        wireReswapButton();
    } catch (err) {
        releaseAppLoading();
        updateStatus('Init Failure', true);
        alert(`VelocityRL Initialization Failed:\n${err.message || err}`);
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

async function refreshBackups() {
    if (!backupContainer) return;
    backupContainer.innerHTML = '<div class="backup-empty">Scanning for backups...</div>';
    try {
        const backups = await invoke('get_backups');
        if (backups.length === 0) {
            backupContainer.innerHTML = '<div class="backup-empty">No active modifications detected. Your files are clean.</div>';
            return;
        }
        backupContainer.innerHTML = '';
        backups.forEach(file => {
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
            div.innerHTML = `
                <div style="display: flex; align-items: center; gap: 12px;">
                    ${pImg ? `<img src="${escHtml(pImg)}" class="flyout-img" style="width: 40px; height: 40px; border-radius: 6px; object-fit: contain; background: rgba(0,0,0,0.2);" />` : '<div class="flyout-img" style="width: 40px; height: 40px; border-radius: 6px; background: rgba(0,0,0,0.2); display: flex; align-items: center; justify-content: center;"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-opacity="0.2"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg></div>'}
                    <div>
                        <div class="backup-name">${escHtml(file.name)}</div>
                        <div class="backup-date">Modified Product</div>
                    </div>
                </div>
                <div class="restore-mini-btn" title="Restore this file">
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="var(--accent-blue)" stroke-width="2"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
                </div>
            `;
            div.querySelector('.restore-mini-btn').onclick = (e) => {
                e.stopPropagation();
                restoreSingle(file.path);
            };
            backupContainer.appendChild(div);
        });
    } catch (err) {
        console.error(err);
        backupContainer.innerHTML = '<div class="backup-empty backup-empty-error">Failed to retrieve backup list.</div>';
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
        const pImg = item.image_url || item.src || '';

        div.innerHTML = `
            ${pImg ? `<img src="${escHtml(pImg)}" class="flyout-img" />` : '<div class="flyout-img"></div>'}
            <div class="flyout-info">
                <span class="item-name">${escHtml(pName)}</span>
                <span style="font-size: 10px; color: var(--text-secondary)">${escHtml(pSlot)}</span>
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

const PAINT_HINT_UNPAINTABLE = "No painted UPK for this item — use None, or set Paintable in items.json";

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

    const enabled = !wantedItem || itemIsPaintable(wantedItem);
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
        await invoke('apply_swap', { ownedId, wantedId, paintId });
        clearInterval(interval);
        interval = null;
        showProgress(true, 100);
        updateStatus('Swap Complete', false);
        const ownedName = ownedItem.product || ownedItem.Product || 'item';
        const wantedName = wantedItem.product || wantedItem.Product || 'item';
        const paintName = paintLabel(paintId);
        const paintBit = paintId > 0 ? ` (${escHtml(paintName)})` : '';
        showToast(`🎉 Swapped <strong>${escHtml(ownedName)}</strong> → <strong>${escHtml(wantedName)}</strong>${paintBit}`, 'success');
        setTimeout(() => { showProgress(false); updateStatus('bitsfdb', false); }, 3000);
    } catch (err) {
        if (interval) clearInterval(interval);
        updateStatus('Swap Failed', true);
        showProgress(false);
        const msg = String(err);
        if (msg.includes('Game directory not set') || msg.includes('Game directory not configured') || msg.includes('game_dir') || msg.includes('file not found') || msg.includes('donor file') || msg.includes('target file')) {
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
        showToast(`${installs[0].label} install detected`, 'success');
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
        btn.onclick = () => {
            document.getElementById('game-dir').value = install.path;
            container.innerHTML = '';
            showToast(`${install.label} selected`, 'success');
        };
        container.appendChild(btn);
    });
    container.style.display = 'block';
}

async function handleBrowse() {
    const dir = await open({ directory: true, multiple: false, title: 'Select Rocket League CookedPCConsole folder' });
    if (dir) {
        document.getElementById('game-dir').value = dir;
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
            showToast('Downloading update, please wait...', 'warning');
            try {
                await invoke('install_update');
                showToast('Update installed! Restarting...', 'success');
                setTimeout(() => window.__TAURI__.process.relaunch(), 2000);
            } catch (err) {
                showToast(`Update failed: ${escHtml(String(err))}`, 'error');
            }
        });
    } catch (err) {
        invoke('append_launch_log', { message: `updater: unexpected invoke error: ${err}` }).catch(() => {});
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
                <a href="#" id="gd-fix-btn" style="font-weight:700;color:#fff;text-decoration:underline;cursor:pointer;">Fix →</a>
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
    // Drop internal/engineering dump lines from GitHub release bodies.
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
const NAME_SPOOF_KEY = 'velocityrl_name_spoof';
const LOGO_SPOOF_KEY = 'velocityrl_logo_spoof';
const BLOG_SPOOF_KEY = 'velocityrl_blog_spoof';
const FAKE_RANKS_KEY = 'velocityrl_fake_ranks';
const CAMERA_SPOOF_KEY = 'velocityrl_camera_spoof';
const DEFAULT_SEASON23_LOGO_URL = 'https://rl-cdn.psyonix.com/LogoImages/S23/rl_season-logo_23_EN_1.png';
const DEFAULT_BLOG_MOTD = '<a href="https://discord.gg/2HhBNbrGMj"><font color="#66CCFF"><u>VelocityRL</u></font></a>';
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
    let name = {};
    try { titles = JSON.parse(localStorage.getItem(TITLE_SPOOF_KEY) || '{}'); } catch {  }
    try { name = JSON.parse(localStorage.getItem(NAME_SPOOF_KEY) || '{}'); } catch {  }
    return { ...titles, ...name };
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

/** Prefer proxy file (psynet_config.json); fall back to localStorage per tool. */
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

/**
 * Boot: load each tool from tools/psynet_proxy/go_mitm/psynet_config.json
 * into localStorage + in-memory title swaps so tabs restore without re-Save.
 * Returns a merge payload for writing back to the proxy file.
 */
async function hydrateSpoofToolsFromDisk() {
    let disk = {};
    try {
        disk = await invoke('get_psynet_spoof') || {};
    } catch (e) {
        invoke('append_launch_log', { message: `psynet: get_psynet_spoof failed: ${e}` }).catch(() => {});
        disk = {};
    }

    // Titles
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

    // Fake ranks
    {
        const fr = toolSliceFromDiskOrLocal(disk, FAKE_RANKS_KEY, 'fake_ranks');
        const fake_ranks = (fr && typeof fr === 'object' && ('enabled' in fr || fr.playlists || fr.reward_levels))
            ? clampFakeRanksRewardWins({ ...fr, reward_levels: fr.reward_levels ? { ...fr.reward_levels } : undefined })
            : { enabled: false, playlists: {} };
        localStorage.setItem(FAKE_RANKS_KEY, JSON.stringify({ fake_ranks }));
    }

    // Camera
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

    // Logo
    {
        const ls = toolSliceFromDiskOrLocal(disk, LOGO_SPOOF_KEY, 'logo_spoof');
        let logo_spoof;
        if (ls && typeof ls === 'object' && ('enabled' in ls || ls.logo_url != null)) {
            const enabled = !!ls.enabled;
            let logo_url = String(ls.logo_url || '').trim();
            // Enabled with empty URL is inactive in Go — keep the toggle honest.
            if (enabled && !logo_url) logo_url = DEFAULT_SEASON23_LOGO_URL;
            logo_spoof = { enabled, logo_url };
        } else {
            logo_spoof = { enabled: false, logo_url: DEFAULT_SEASON23_LOGO_URL };
        }
        localStorage.setItem(LOGO_SPOOF_KEY, JSON.stringify({ logo_spoof }));
    }

    // Blog / MotD
    {
        const bs = toolSliceFromDiskOrLocal(disk, BLOG_SPOOF_KEY, 'blog_spoof');
        const blog_spoof = (bs && typeof bs === 'object' && ('enabled' in bs || bs.motd != null))
            ? { enabled: !!bs.enabled, motd: bs.motd || '' }
            : { enabled: false, motd: DEFAULT_BLOG_MOTD };
        localStorage.setItem(BLOG_SPOOF_KEY, JSON.stringify({ blog_spoof }));
    }

    // Push into DOM now so toggles/queues match disk before the user opens a tab.
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

/** Full proxy write from hydrated localStorage (boot only — not every Save). */
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
        ping_spoof: { enabled: false, ms: 0 },
        inventory_spoof: { enabled: false, items: [] },
    };
}

/**
 * Boot write must never downgrade logo/MotD that are still enabled on disk
 * (e.g. hydrate saw {} after a transient read miss, then invented enabled:false).
 */
function preserveEnabledLogoBlogFromDisk(payload, disk) {
    const out = { ...payload };
    if (disk?.logo_spoof?.enabled) {
        const diskUrl = String(disk.logo_spoof.logo_url || '').trim();
        const outUrl = String(out.logo_spoof?.logo_url || '').trim();
        // Never invent enabled:false over disk, and never wipe a disk URL with "".
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

function anySpoofToolEnabled(payload) {
    const p = payload || payloadFromHydratedLocal();
    if (p.enabled && p.swaps?.length) return true;
    if (p.fake_ranks?.enabled) return true;
    if (p.camera_spoof?.enabled) return true;
    if (p.logo_spoof?.enabled) return true;
    if (p.blog_spoof?.enabled) return true;
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
    } catch { /* ignore */ }
    try {
        // Always refresh proxy file from hydrated tool state (hot-reload if already up).
        // Merge write keeps logo_spoof/blog_spoof when other tools Save; boot includes them.
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
    } catch { /* ignore */ }
    if (!anySpoofToolEnabled(payload)) {
        invoke('append_launch_log', { message: 'psynet: no spoof tools enabled - skip auto-start' }).catch(() => {});
        return;
    }
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
            showToast(`Starting PsyNet proxy (${reason}) — approve UAC if prompted…`, 'success');
            const st = await invoke('start_psynet_proxy', {});
            setProxyUi(!!st.running);
            if (st.running) {
                showToast('PsyNet proxy running. Keep VelocityRL open.', 'success');
            }
            return !!st.running;
        } catch (e) {
            setProxyUi(false);
            showToast(String(e), 'error');
            return false;
        } finally {
            proxyEnsurePromise = null;
        }
    })();
    return proxyEnsurePromise;
}

/** Single-flight Save: merge one tool slice into psynet_config.json (does not wipe others). */
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
            overlay.classList.remove('active');
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
        await invoke('stop_psynet_proxy', { revertHosts });
    } catch (e) {
        console.warn('stop_psynet_proxy on close:', e);
    }
}

function attachCloseGuard() {
    if (closeGuardAttached) return;
    const winApi = window.__TAURI__?.window;
    if (!winApi?.getCurrentWindow) return;
    closeGuardAttached = true;
    const appWindow = winApi.getCurrentWindow();
    appWindow.onCloseRequested(async (event) => {
        event.preventDefault();
        if (closeModalOpen || closeInProgress) return;

        closeModalOpen = true;
        const choice = await promptCloseModal();
        closeModalOpen = false;
        if (choice === 'stay') return;

        closeInProgress = true;
        await stopProxyOnClose(choice === 'revert');
        await appWindow.destroy();
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

let namesTabReady = false;

const NAME_SPOOF_UNAVAILABLE = true;
let nameSpoofForceOffDone = false;

function updateNamePreview() {
    const fromEl = document.getElementById('name-preview-from');
    const toEl = document.getElementById('name-preview-to');
    const real = document.getElementById('name-spoof-real')?.value?.trim() || 'bitss.';
    const display = document.getElementById('name-spoof-display')?.value?.trim() || 'evil bits';
    if (fromEl) fromEl.textContent = real || '—';
    if (toEl) toEl.textContent = display || '—';
}

function mergeNameSpoofLocalPlayerId(player_id) {
    if (NAME_SPOOF_UNAVAILABLE) return;
    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(NAME_SPOOF_KEY) || '{}'); } catch {  }
    const name_spoof = { ...(saved.name_spoof || {}), player_id };
    localStorage.setItem(NAME_SPOOF_KEY, JSON.stringify({ ...saved, name_spoof }));
}

let playerIdPollTimer = null;

async function refreshLearnedPlayerId() {
    if (NAME_SPOOF_UNAVAILABLE) return;
    const playerIdEl = document.getElementById('name-spoof-player-id');
    if (!playerIdEl || document.activeElement === playerIdEl) return;
    try {
        const st = await invoke('get_psynet_status');
        const pid = (st.player_id || '').trim();
        if (!pid || playerIdEl.value.trim()) return;
        playerIdEl.value = pid;
        mergeNameSpoofLocalPlayerId(pid);
    } catch {  }
}

async function forceInventorySpoofOffInConfig() {
    try { localStorage.removeItem('velocityrl_inventory_spoof'); } catch {  }
    try {
        await invoke('save_psynet_spoof', {
            payload: {
                enabled: true,
                method: 'raw',
                inventory_spoof: { enabled: false, items: [] },
            },
        });
    } catch {  }
}

async function forcePingSpoofOffInConfig() {
    try { localStorage.removeItem('velocityrl_ping_spoof'); } catch {  }
    try {
        await invoke('save_psynet_spoof', {
            payload: {
                enabled: true,
                method: 'raw',
                ping_spoof: { enabled: false, ms: 0 },
            },
        });
    } catch {  }
}

async function forceNameSpoofOffInConfig() {
    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(NAME_SPOOF_KEY) || '{}'); } catch {  }
    const prev = saved.name_spoof || {};
    const name_spoof = {
        enabled: false,
        display_name: prev.display_name || saved.display_name || saved.custom_name || '',
        real_name: prev.real_name || '',
        player_id: prev.player_id || '',
        replace_all_player_names: false,
        broker: true,
        classprop_name: false,
        websocket: false,
        ws_enabled: false,
    };
    localStorage.setItem(NAME_SPOOF_KEY, JSON.stringify({
        ...saved,
        name_spoof,
        display_name: name_spoof.display_name,
        custom_name: name_spoof.display_name,
    }));
    try {
        await invoke('save_psynet_spoof', {
            payload: {
                enabled: true,
                method: 'raw',
                custom_name: name_spoof.display_name,
                name_spoof,
            },
        });
    } catch {  }
}

function lockNameSpoofControls() {
    const ids = [
        'name-spoof-enabled',
        'name-spoof-display',
        'name-spoof-real',
        'name-spoof-player-id',
        'name-spoof-lab-all',
        'name-spoof-save-btn',
    ];
    for (const id of ids) {
        const el = document.getElementById(id);
        if (!el) continue;
        el.disabled = true;
        el.setAttribute('aria-disabled', 'true');
        if (el.tagName === 'INPUT' && (el.type === 'text' || el.type === 'search')) {
            el.readOnly = true;
            el.tabIndex = -1;
        }
    }
    const card = document.querySelector('#names-tab .name-spoof-disabled, #names-tab .spoof-card');
    if (card) {
        card.classList.add('name-spoof-disabled');
        card.setAttribute('aria-disabled', 'true');
    }
}

function initNamesTab() {
    const enabledEl = document.getElementById('name-spoof-enabled');
    const displayEl = document.getElementById('name-spoof-display');
    const realEl = document.getElementById('name-spoof-real');
    const playerIdEl = document.getElementById('name-spoof-player-id');
    const labEl = document.getElementById('name-spoof-lab-all');
    const saveBtn = document.getElementById('name-spoof-save-btn');
    if (!enabledEl || !saveBtn) return;

    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(NAME_SPOOF_KEY) || '{}'); } catch {  }
    const ns = saved.name_spoof || {};

    enabledEl.checked = false;
    if (displayEl) displayEl.value = ns.display_name || saved.display_name || saved.custom_name || '';
    if (realEl) realEl.value = ns.real_name || '';
    if (playerIdEl) playerIdEl.value = ns.player_id || '';
    if (labEl) labEl.checked = false;
    syncNameSpoofSwitchAria(enabledEl);
    syncNameSpoofSwitchAria(labEl);
    updateNamePreview();
    lockNameSpoofControls();

    if (playerIdPollTimer) {
        clearInterval(playerIdPollTimer);
        playerIdPollTimer = null;
    }

    if (NAME_SPOOF_UNAVAILABLE && !nameSpoofForceOffDone) {
        nameSpoofForceOffDone = true;
        forceNameSpoofOffInConfig();
    }

    if (namesTabReady) return;
    namesTabReady = true;

    saveBtn.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        showToast('Name spoof is not available yet.', 'error');
    });
}

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

/** Season reward "wins this level" is capped at 10 (UI max + legacy config clamp). */
function clampSeasonLevelWins(n) {
    const v = Number(n);
    if (!Number.isFinite(v)) return null;
    return Math.max(0, Math.min(10, Math.round(v)));
}

function clampFakeRanksRewardWins(fake_ranks) {
    if (!fake_ranks || typeof fake_ranks !== 'object' || !fake_ranks.reward_levels) return fake_ranks;
    const wins = fake_ranks.reward_levels.season_level_wins;
    if (!Number.isFinite(wins)) return fake_ranks;
    const clamped = clampSeasonLevelWins(wins);
    if (clamped === null) return fake_ranks;
    fake_ranks.reward_levels.season_level_wins = clamped;
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

let fakeRanksAddFormState = { tier: 19, mmr: rankMeta(19).mmr };

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

function rankOverrideFromState(tier, displayMmr) {
    const meta = rankMeta(tier);
    const mmr = Number.isFinite(displayMmr) ? displayMmr : meta.mmr;
    return {
        display_mmr: mmr,
        tier,
        division: rankDivisionForTier(tier),
    };
}

function tierFromOverride(ov) {
    if (!ov || typeof ov !== 'object') return 19;
    if (Number.isFinite(ov.tier)) return Math.max(0, Math.min(22, Number(ov.tier)));
    return 19;
}

function mmrFromOverride(ov, tier) {
    if (ov && Number.isFinite(ov.display_mmr)) return Number(ov.display_mmr);
    if (ov && Number.isFinite(ov.mu)) return Math.round(ov.mu * 20 + 100);
    return rankMeta(tier).mmr;
}

function ensureFakeRanksEntry(id, fallbackTier = 19) {
    if (!fakeRanksPlaylistState[id]) {
        const tier = fallbackTier;
        fakeRanksPlaylistState[id] = { tier, mmr: rankMeta(tier).mmr };
    }
}

function syncFakeRanksAddFormUi() {
    const meta = rankMeta(fakeRanksAddFormState.tier);
    const icon = document.getElementById('fake-ranks-add-rank-icon');
    const name = document.getElementById('fake-ranks-add-rank-name');
    const mmrInput = document.getElementById('fake-ranks-add-mmr');
    if (icon) {
        icon.src = rankIconSrc(fakeRanksAddFormState.tier);
        icon.alt = meta.name;
        icon.onerror = () => { icon.onerror = null; icon.src = rankIconUrl(fakeRanksAddFormState.tier); };
    }
    if (name) name.textContent = meta.name;
    if (mmrInput && document.activeElement !== mmrInput) {
        mmrInput.value = String(Math.round(fakeRanksAddFormState.mmr ?? meta.mmr));
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
        const st = fakeRanksPlaylistState[id] || { tier: 19, mmr: rankMeta(19).mmr };
        const meta = rankMeta(st.tier);
        const mmr = Math.round(st.mmr ?? meta.mmr);
        return `<div class="backup-item" data-playlist="${escHtml(id)}">
            <div>
                <div class="backup-name rank-queue-row-preview">
                    <span class="rank-queue-playlist">${escHtml(playlistLabel(id))}</span>
                    <span class="rank-queue-sep">·</span>
                    <img class="rank-playlist-icon rank-queue-icon" src="${rankIconSrc(st.tier)}" width="22" height="22" alt="${escHtml(meta.name)}">
                    <span class="rank-queue-rank">${escHtml(meta.name)}</span>
                    <input type="number" class="rank-queue-mmr-input" data-playlist="${escHtml(id)}" min="0" max="3000" step="1" inputmode="numeric" autocomplete="off" value="${mmr}" aria-label="${escHtml(playlistLabel(id))} MMR">
                </div>
                <div class="backup-date">Playlist ${escHtml(id)}</div>
            </div>
            <div class="rank-queue-actions">
                <button type="button" class="rank-queue-edit-btn" data-edit-playlist="${escHtml(id)}" title="Change rank">Edit rank</button>
                <div class="restore-mini-btn" data-remove-index="${i}" title="Remove playlist">Remove</div>
            </div>
        </div>`;
    }).join('');
    list.querySelectorAll('.rank-queue-mmr-input').forEach((input) => {
        input.addEventListener('input', () => {
            const pid = input.dataset.playlist;
            const n = Number(input.value);
            ensureFakeRanksEntry(pid);
            fakeRanksPlaylistState[pid].mmr = Number.isFinite(n) ? n : null;
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
    const id = sel?.value?.trim();
    if (!id) {
        showToast('All playlists are already configured.', 'error');
        return;
    }
    const mmrVal = mmrInput?.value !== '' ? Number(mmrInput.value) : fakeRanksAddFormState.mmr;
    fakeRanksPlaylistState[id] = {
        tier: fakeRanksAddFormState.tier,
        mmr: Number.isFinite(mmrVal) ? mmrVal : rankMeta(fakeRanksAddFormState.tier).mmr,
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
                if (!fakeRanksAddFormState.mmr) fakeRanksAddFormState.mmr = meta.mmr;
                syncFakeRanksAddFormUi();
            } else {
                ensureFakeRanksEntry(target, tier);
                fakeRanksPlaylistState[target].tier = tier;
                if (!fakeRanksPlaylistState[target].mmr) fakeRanksPlaylistState[target].mmr = meta.mmr;
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
        const mmrVal = mmrInput?.value !== '' ? Number(mmrInput.value) : (st.mmr ?? null);
        playlists[id] = rankOverrideFromState(st.tier, mmrVal);
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
        };
        if (!fakeRanksQueueOrder.includes(id)) fakeRanksQueueOrder.push(id);
    });
}

function loadFakeRanksFromSaved(saved) {
    buildSeasonRewardSelect();
    const fr = saved.fake_ranks || {};
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
                mmr: mmrFromOverride(ov, tier),
            };
            fakeRanksQueueOrder.push(id);
        });
    }
    applyLegacyDefaultToPlaylists(fr);
    fakeRanksAddFormState = { tier: 19, mmr: rankMeta(19).mmr };
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
    document.getElementById('fake-ranks-add-mmr')?.addEventListener('input', (e) => {
        const n = Number(e.target.value);
        fakeRanksAddFormState.mmr = Number.isFinite(n) ? n : null;
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
        const hasPlaylists = fake_ranks.playlists && Object.keys(fake_ranks.playlists).length;
        const hasRewardLevels = fake_ranks.reward_levels && (
            Number.isFinite(fake_ranks.reward_levels.season_level)
            || Number.isFinite(fake_ranks.reward_levels.season_level_wins)
        );
        if (enabled && !hasPlaylists && !hasRewardLevels) {
            showToast('Add at least one playlist override, set season reward, or turn off fake ranks.', 'error');
            return;
        }
        try {
            await runToolSave(saveBtn, 'fake ranks', { fake_ranks }, { enabled });
            localStorage.setItem(FAKE_RANKS_KEY, JSON.stringify({ fake_ranks }));
            flashButtonLabel(saveBtn, enabled ? 'Saved' : 'Saved (off)');
            if (!enabled) showToast('Fake ranks off.', 'success');
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
        const confirmed = await window.__TAURI__.dialog.ask(
            'This action is irreversible and should only be used if you have recently verified your game files in the Epic Games launcher.\n\nAre you sure you want to reswap all items?', 
            { title: 'Reswap All Items', kind: 'warning' }
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

function setPaletteBusy(busy) {
    paletteBusy = !!busy;
    if (!paletteBusy) return;
    const applyBtn = document.getElementById('palette-apply-btn');
    const restoreBtn = document.getElementById('palette-restore-btn');
    if (applyBtn) applyBtn.disabled = true;
    if (restoreBtn) restoreBtn.disabled = true;
}

function syncPaletteUi(applied, status) {
    const toggle = document.getElementById('rich-palette-enabled');
    const row = document.getElementById('palette-switch-row');
    const applyBtn = document.getElementById('palette-apply-btn');
    const restoreBtn = document.getElementById('palette-restore-btn');
    const on = !!applied;
    const hasBackup = !!status?.backup_present;
    if (toggle) {
        toggle.checked = on;
        toggle.disabled = true;
        syncNameSpoofSwitchAria(toggle);
    }
    row?.classList.toggle('is-applied', on);
    if (applyBtn) applyBtn.disabled = on || paletteBusy;
    if (restoreBtn) restoreBtn.disabled = !on || !hasBackup || paletteBusy;
}

async function refreshPaletteStatus() {
    try {
        const st = await invoke('get_palette_status');
        syncPaletteUi(!!st.applied, st);
    } catch {
        syncPaletteUi(false);
    }
}

async function runPaletteAction(btn, command, doneLabel, fallbackMsg) {
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

function initMiscTab() {
    const enabledEl = document.getElementById('logo-spoof-enabled');
    const urlEl = document.getElementById('logo-spoof-url');
    const saveBtn = document.getElementById('logo-spoof-save-btn');
    const defaultLink = document.getElementById('logo-spoof-default');
    const blogEnabledEl = document.getElementById('blog-spoof-enabled');
    const blogMotdEl = document.getElementById('blog-spoof-motd');
    const blogSaveBtn = document.getElementById('blog-spoof-save-btn');
    const paletteApply = document.getElementById('palette-apply-btn');
    const paletteRestore = document.getElementById('palette-restore-btn');
    if (!enabledEl || !urlEl || !saveBtn) return;

    // Do not reload the form on every tab visit — that wiped unsaved custom URLs.
    if (saveBtn.dataset.wired === '1') {
        refreshPaletteStatus();
        return;
    }
    saveBtn.dataset.wired = '1';

    let saved = {};
    try { saved = JSON.parse(localStorage.getItem(LOGO_SPOOF_KEY) || '{}'); } catch {  }
    const ls = saved.logo_spoof || {};
    enabledEl.checked = !!ls.enabled;
    syncNameSpoofSwitchAria(enabledEl);
    urlEl.value = ls.logo_url || saved.logo_url || DEFAULT_SEASON23_LOGO_URL;

    let blogSaved = {};
    try { blogSaved = JSON.parse(localStorage.getItem(BLOG_SPOOF_KEY) || '{}'); } catch {  }
    const bs = blogSaved.blog_spoof || {};
    if (blogEnabledEl) {
        blogEnabledEl.checked = !!bs.enabled;
        syncNameSpoofSwitchAria(blogEnabledEl);
    }
    if (blogMotdEl) blogMotdEl.value = bs.motd || blogSaved.motd || DEFAULT_BLOG_MOTD;

    refreshPaletteStatus();
    try {
        const raw = sessionStorage.getItem('velocityrl_repair_report');
        if (raw) showRepairBanner(JSON.parse(raw));
    } catch {  }
    invoke('check_integrity').then((r) => {
        if (r?.repaired) {
            sessionStorage.setItem('velocityrl_repair_report', JSON.stringify(r));
            showRepairBanner(r);
        }
    }).catch(() => {});

    enabledEl.addEventListener('change', () => syncNameSpoofSwitchAria(enabledEl));
    blogEnabledEl?.addEventListener('change', () => syncNameSpoofSwitchAria(blogEnabledEl));
    defaultLink?.addEventListener('click', (e) => {
        e.preventDefault();
        urlEl.value = DEFAULT_SEASON23_LOGO_URL;
        showToast('Season 23 default set.', 'success');
    });
    paletteApply?.addEventListener('click', () => runPaletteAction(
        paletteApply, 'apply_rich_palette', 'Applied', 'Palette on. Restart Rocket League.',
    ));
    paletteRestore?.addEventListener('click', () => runPaletteAction(
        paletteRestore, 'restore_rich_palette', 'Restored', 'Palette off.',
    ));
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
    const keepCustom = userEditedCustomText && !!(customEl?.value?.trim());
    set('title-display-id', id);

    if (!keepCustom) set('title-custom-text', text);
    setSelectedSlot('display-selected', title, 'Search below - or type custom text');
    renderDisplayList(document.getElementById('display-search')?.value || '');
    updateTitlePreview();
    if (toast && id) {
        const shown = keepCustom
            ? (customEl.value.trim() || text || id)
            : (text || id);
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
    listEl.querySelectorAll('.title-row').forEach(row => {
        row.onclick = () => {
            const t = findTitleById(row.dataset.id);
            if (t) onPick(t, true);
        };
    });
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
    titleSwaps.push(entry);
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

async function openChangelog() {
    document.getElementById('changelog-modal').classList.add('active');
    invoke('get_config').catch(() => ({})).then(cfg => {
        const btn = document.getElementById('toggle-changelog-startup');
        if (btn) btn.textContent = cfg.changelog_on_startup === false ? 'Show on startup' : "Don't show on startup";
    });
    const body = document.getElementById('changelog-body');
    try {
        const res = await fetch('https://api.github.com/repos/bitsfdb/VelocityRL/releases?per_page=20');
        if (!res.ok) throw new Error('fetch failed');
        const releases = await res.json();
        const filtered = releases.filter(r => semverGte(r.tag_name || '', '2.0.0'));
        if (!filtered.length) { body.innerHTML = '<div style="color:var(--text-secondary);padding:20px;">No releases yet.</div>'; return; }
        body.innerHTML = filtered.map(r => {
            const date = r.published_at ? new Date(r.published_at).toLocaleDateString('en-US', { year:'numeric', month:'long', day:'numeric' }) : '';
            return `
                <div class="changelog-release">
                    <div class="changelog-release-tag">${escHtml(r.tag_name || r.name)}</div>
                    <div class="changelog-release-date">${date}</div>
                    <div class="changelog-release-body">${formatChangelogNotes(r.body)}</div>
                </div>`;
        }).join('');
    } catch {
        body.innerHTML = '<div style="color:var(--text-secondary);padding:20px;">Could not load changelog. Check your connection.</div>';
    }
}

document.addEventListener('contextmenu', (e) => e.preventDefault());

window.addEventListener('DOMContentLoaded', () => {
    document.getElementById('privacy-link').addEventListener('click', (e) => {
        e.preventDefault();
        window.__TAURI__.core.invoke('plugin:shell|open', { path: 'https://velocityrl.tech/privacy.html' });
    });
    init();
});
