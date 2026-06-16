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

async function fetchItemsFromAPI() {
    const allItems = [];
    const limit = 200;
    let offset = 0;

    while (true) {
        const res = await fetch(`${API_BASE}/v2/rl/products?limit=${limit}&offset=${offset}`);
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
            });
        }
        if (allItems.length >= data.meta.total_filtered || data.products.length < limit) break;
        offset += limit;
    }

    return allItems;
}

function showToast(message, type = 'success') {
    const container = document.getElementById('toast-container');
    if (!container) return;

    const toast = document.createElement('div');
    toast.className = `toast ${type}`;

    const contentEl = document.createElement('div');
    contentEl.className = 'toast-content';

    if (type === 'error') {
        const discordLink = 'https://discord.gg/2HhBNbrGMj';
        contentEl.innerHTML = `<div>${escHtml(String(message))}<br><a href="#" class="toast-link" onclick="event.preventDefault(); window.__TAURI__.core.invoke('plugin:shell|open', { path: '${discordLink}' })">Join Support Discord</a></div>`;
        const copyBtn = document.createElement('button');
        copyBtn.className = 'toast-copy-btn';
        copyBtn.textContent = 'Copy';
        copyBtn.addEventListener('click', () => {
            navigator.clipboard.writeText(String(message)).then(() => {
                copyBtn.textContent = 'Copied';
                setTimeout(() => { copyBtn.textContent = 'Copy'; }, 1500);
            });
        });
        toast.appendChild(contentEl);
        toast.appendChild(copyBtn);
    } else {
        contentEl.innerHTML = String(message);
        toast.appendChild(contentEl);
    }

    container.appendChild(toast);
    
    setTimeout(() => {
        toast.style.animation = 'toastSlideOut 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards';
        setTimeout(() => toast.remove(), 300);
    }, 6000);
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

async function init() {
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
        const pName = item.Product || item.product || 'Unknown';
        const pQuality = item.Quality || item.quality || 'Common';
        const pSlot = item.Slot || item.slot || '';
        const pImg = item.image_url || item.src || '';

        const container = document.getElementById('owned-selected');
        container.innerHTML = `
            <div class="clear-item-btn">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
            </div>
            ${pImg ? `<img src="${escHtml(pImg)}" class="selected-img" />` : ''}
            <h2>${escHtml(pName)}</h2>
            <span class="quality-badge">${escHtml(pQuality)}</span>
            <p style="margin-top: 16px; font-size: 13px; color: var(--text-secondary)">${escHtml(pSlot)}</p>
        `;
        container.querySelector('.clear-item-btn').addEventListener('click', clearOwned);
        container.classList.add('selected');
        ownedSearch.value = pName;
        validate();
    });

    setupSearch(wantedSearch, wantedResults, (item) => {
        wantedItem = item;
        const pName = item.Product || item.product || 'Unknown';
        const pQuality = item.Quality || item.quality || 'Common';
        const pSlot = item.Slot || item.slot || '';
        const pImg = item.image_url || item.src || '';

        const container = document.getElementById('wanted-selected');
        container.innerHTML = `
            <div class="clear-item-btn">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
            </div>
            ${pImg ? `<img src="${escHtml(pImg)}" class="selected-img" />` : ''}
            <h2>${escHtml(pName)}</h2>
            <span class="quality-badge">${escHtml(pQuality)}</span>
            <p style="margin-top: 16px; font-size: 13px; color: var(--text-secondary)">${escHtml(pSlot)}</p>
        `;
        container.querySelector('.clear-item-btn').addEventListener('click', clearWanted);
        container.classList.add('selected');
        wantedSearch.value = pName;
        validate();
    });

    document.querySelectorAll('.nav-item[data-tab]').forEach(btn => {
        btn.onclick = () => {
            document.querySelectorAll('.nav-item').forEach(b => b.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            btn.classList.add('active');
            document.getElementById(btn.dataset.tab).classList.add('active');
            if (btn.dataset.tab === 'restore-tab') refreshBackups();
        };
    });

    document.querySelectorAll('.cat-btn').forEach(btn => {
        btn.onclick = () => {
            document.querySelectorAll('.cat-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            currentCategory = btn.dataset.slot;
            ownedSearch.dispatchEvent(new Event('input'));
            wantedSearch.dispatchEvent(new Event('input'));
        };
    });

    applyBtn.onclick = handleApply;
    document.getElementById('restore-btn').onclick = handleRestore;
    document.getElementById('website-btn').onclick = () => window.__TAURI__.core.invoke('plugin:shell|open', { path: 'https://velocityrl.tech' });
    document.getElementById('settings-btn').onclick = () => document.getElementById('settings-modal').classList.add('active');
    document.getElementById('cancel-settings').onclick = () => {
        document.getElementById('settings-modal').classList.remove('active');
        document.getElementById('install-chooser').style.display = 'none';
    };
    document.getElementById('close-settings').onclick = handleSaveSettings;
    document.getElementById('browse-dir').onclick = handleBrowse;
    document.getElementById('autodetect-dir').onclick = handleAutoDetect;
    document.getElementById('settings-modal').onclick = (e) => {
        if (e.target === document.getElementById('settings-modal')) {
            document.getElementById('settings-modal').classList.remove('active');
            document.getElementById('install-chooser').style.display = 'none';
        }
    };

    try {
        updateStatus('Verifying Integrity...', false);
        await invoke('check_integrity').catch(e => {
            throw new Error(`Security Violation: ${e}`);
        });
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
            if (installs.length === 1) {
                document.getElementById('game-dir').value = installs[0].path;
                await invoke('save_config', { config: { game_dir: installs[0].path } }).catch(() => {});
                showToast(`${installs[0].label} install detected`, 'success');
            } else if (installs.length > 1) {
                document.getElementById('settings-modal').classList.add('active');
                showInstallChooser(installs);
            } else {
                document.getElementById('settings-modal').classList.add('active');
            }
        }
        updateStatus('bitsfdb', false);
        invoke('cleanup_temp_files').catch(e => console.warn('Cleanup failed:', e));
        checkForUpdates();
    } catch (err) {
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
    container.innerHTML = `
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-opacity="0.2">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
            <circle cx="8.5" cy="8.5" r="1.5"/>
            <polyline points="21 15 16 10 5 21"/>
        </svg>
        <p style="color: var(--text-secondary); margin-top: 12px; font-size: 13px;">No item selected</p>
    `;
    container.classList.remove('selected');
    document.getElementById('owned-search').value = '';
    validate();
}

function clearWanted() {
    wantedItem = null;
    const container = document.getElementById('wanted-selected');
    container.innerHTML = `
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-opacity="0.2">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
            <circle cx="8.5" cy="8.5" r="1.5"/>
            <polyline points="21 15 16 10 5 21"/>
        </svg>
        <p style="color: var(--text-secondary); margin-top: 12px; font-size: 13px;">No item selected</p>
    `;
    container.classList.remove('selected');
    document.getElementById('wanted-search').value = '';
    validate();
}

window.clearOwned = clearOwned;
window.clearWanted = clearWanted;

async function refreshBackups() {
    if (!backupContainer) return;
    backupContainer.innerHTML = '<div style="padding: 40px; text-align: center; color: var(--text-secondary);">Scanning for backups...</div>';
    try {
        const backups = await invoke('get_backups');
        if (backups.length === 0) {
            backupContainer.innerHTML = '<div style="padding: 60px; text-align: center; color: var(--text-secondary);">No active modifications detected. Your files are clean.</div>';
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
        backupContainer.innerHTML = '<div style="padding: 40px; text-align: center; color: var(--danger);">Failed to retrieve backup list.</div>';
    }
}

async function restoreSingle(path) {
    try {
        updateStatus('Restoring...', false);
        await invoke('restore_single_backup', { path });
        updateStatus('Restored', false);
        refreshBackups();
        setTimeout(() => updateStatus('bitsfdb', false), 2000);
    } catch (err) {
        updateStatus('Error', true);
        alert(`Failed to restore: ${err}`);
    }
}



function updateStatus(text, isError = false) {
    if (!statusText) return;
    statusText.textContent = text;
    statusText.style.color = isError ? '#ef4444' : '#a1a1aa';
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

function validate() {
    if (!applyBtn) return;
    const oSlot = ownedItem ? normSlot(ownedItem.Slot || ownedItem.slot) : '';
    const wSlot = wantedItem ? normSlot(wantedItem.Slot || wantedItem.slot) : '';
    const typesMatch = !ownedItem || !wantedItem || oSlot === wSlot;
    applyBtn.disabled = !(ownedItem && wantedItem && typesMatch);
}

function openSettingsForPath() {
    document.getElementById('settings-modal').classList.add('active');
    invoke('detect_game_dir').then(installs => {
        if (installs && installs.length > 1) showInstallChooser(installs);
    }).catch(() => {});
}

async function handleApply() {
    try {
        updateStatus('Please Wait...', false);
        showProgress(true, 15);
        applyBtn.disabled = true;
        let p = 15;
        const interval = setInterval(() => { if (p < 85) p += 5; showProgress(true, p); }, 400);
        const ownedId = (ownedItem.ID !== undefined ? ownedItem.ID : ownedItem.id).toString();
        const wantedId = (wantedItem.ID !== undefined ? wantedItem.ID : wantedItem.id).toString();
        await invoke('apply_swap', { ownedId, wantedId });
        clearInterval(interval);
        showProgress(true, 100);
        updateStatus('Swap Complete', false);
        setTimeout(() => { showProgress(false); updateStatus('bitsfdb', false); }, 3000);
    } catch (err) {
        updateStatus('Swap Failed', true);
        showProgress(false);
        if (String(err).includes('Game directory not set') || String(err).includes('Game directory not configured') || String(err).includes('game_dir')) {
            showToast('Game path not set — please configure it in Settings.', 'error');
            openSettingsForPath();
        } else {
            showToast(`Swap Error: ${err}`, 'error');
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
    } finally { applyBtn.disabled = false; }
}

async function handleRestore() {
    try {
        updateStatus('Running Restoration...', false);
        const result = await invoke('restore_backups');
        updateStatus(result, false);
        refreshBackups();
        setTimeout(() => updateStatus('bitsfdb', false), 3000);
    } catch (err) {
        updateStatus('Restore Failed', true);
        if (String(err).includes('Game directory not set') || String(err).includes('Game directory not configured') || String(err).includes('game_dir')) {
            showToast('Game path not set — please configure it in Settings.', 'error');
            openSettingsForPath();
        } else {
            showToast(`Restore Error: ${err}`, 'error');
        }
        console.error(err);
    }
}

async function handleSaveSettings() {
    const existing = await invoke('get_config').catch(() => ({}));
    const dir = document.getElementById('game-dir').value.trim();
    await invoke('save_config', { config: { ...existing, game_dir: dir || existing.game_dir || '' } })
        .catch(e => console.warn('Save config failed:', e));
    document.getElementById('settings-modal').classList.remove('active');
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
    label.textContent = 'Multiple installs found — pick one:';
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
        if (!version) return;
        const toast = document.createElement('div');
        toast.className = 'toast warning';
        toast.innerHTML = `<div class="toast-content">Update v${escHtml(version)} available — <a href="#" class="toast-link" id="install-update-link">Install Now</a></div>`;
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
    } catch (_) {
        // Fallback: check GitHub API directly
        try {
            const current = await window.__TAURI__.app.getVersion();
            const res = await fetch('https://api.github.com/repos/bitsfdb/VelocityRL/releases/latest');
            if (!res.ok) return;
            const data = await res.json();
            const latest = (data.tag_name || '').replace(/^v/, '');
            if (!latest || latest === current) return;
            const url = escHtml(data.html_url || 'https://github.com/bitsfdb/VelocityRL/releases/latest');
            showToast(
                `Update v${escHtml(latest)} available — <a href="#" class="toast-link" onclick="event.preventDefault(); window.__TAURI__.core.invoke('plugin:shell|open', { path: '${url}' })">Download</a>`,
                'warning'
            );
        } catch (_) {}
    }
}

// ── Privacy ────────────────────────────────────────────────────────────────

async function fetchPrivacyVersion() {
    try {
        const resp = await fetch('https://velocityrl.tech/privacy.html');
        if (!resp.ok) return null;
        const html = await resp.text();
        // Parse "Last updated: ..." from the page
        const match = html.match(/last\s+updated[:\s]+([^\n<]{3,60})/i);
        return match ? match[1].trim() : null;
    } catch { return null; }
}

window.addEventListener('DOMContentLoaded', async () => {
    document.getElementById('privacy-link').addEventListener('click', (e) => {
        e.preventDefault();
        window.__TAURI__.core.invoke('plugin:shell|open', { path: 'https://velocityrl.tech/privacy.html' });
    });

    document.getElementById('privacy-agree-btn').addEventListener('click', async () => {
        const config = await invoke('get_config').catch(() => ({ game_dir: '', privacy_agreed: false, privacy_version: '', rl_username: '', rl_platform: 'epic', trn_api_key: '' }));
        await invoke('save_config', { config: { ...config, privacy_agreed: true, privacy_version: window._currentPrivacyVersion || '' } }).catch(() => {});
        document.getElementById('privacy-modal').classList.remove('active');
        init();
        initOverlay();
    });

    const config = await invoke('get_config').catch(() => null);

    // Strict check — any non-true value (false, null, missing) means not agreed
    const agreed = config?.privacy_agreed === true;

    // Fetch last-updated date from the privacy page itself
    const serverVersion = await fetchPrivacyVersion();
    if (serverVersion) window._currentPrivacyVersion = serverVersion;

    if (!agreed) {
        document.getElementById('privacy-modal').classList.add('active');
        return;
    }

    if (serverVersion && config.privacy_version !== serverVersion) {
        document.getElementById('privacy-modal-title').textContent = 'Privacy Policy Updated';
        document.getElementById('privacy-modal-desc').textContent = 'Our Privacy Policy has been updated. Please review and agree to continue.';
        document.getElementById('privacy-modal').classList.add('active');
        return;
    }

    init();
    initOverlay();
});

// ── Stats Tab ──────────────────────────────────────────────────────────────

function initOverlay() {
    const overlayStatus = document.getElementById('overlay-status');
    const streamStatus  = document.getElementById('stream-status');
    const streamSub     = document.getElementById('stream-sub');
    const launchBtn     = document.getElementById('overlay-launch-btn');
    const hideBtn       = document.getElementById('overlay-hide-btn');
    const startBtn      = document.getElementById('stream-start-btn');
    const stopBtn       = document.getElementById('stream-stop-btn');
    const enableApiBtn  = document.getElementById('enable-api-btn');
    const saveBtn       = document.getElementById('stats-save-btn');
    const portInput     = document.getElementById('stats-port');
    const usernameInput = document.getElementById('stats-username');
    const posBtns       = document.querySelectorAll('.pos-btn');

    let currentPos = 'top-center';

    function setOverlayStatus(on) {
        overlayStatus.textContent = on ? 'On' : 'Off';
        overlayStatus.style.color = on ? '#4ade80' : 'var(--text-secondary)';
    }
    function setStreamStatus(on) {
        streamStatus.textContent = on ? 'On' : 'Off';
        streamStatus.style.color = on ? '#4ade80' : 'var(--text-secondary)';
        streamSub.textContent = on ? 'connected' : 'connects to RL on localhost';
    }

    launchBtn.addEventListener('click', async () => {
        await invoke('create_overlay').catch(e => showToast(String(e), 'error'));
        setOverlayStatus(true);
    });

    hideBtn.addEventListener('click', async () => {
        await invoke('hide_overlay').catch(() => {});
        setOverlayStatus(false);
    });

    document.getElementById('overlay-test-btn')?.addEventListener('click', () => {
        invoke('test_overlay').catch(() => {});
    });

    document.getElementById('overlay-move-btn')?.addEventListener('click', () => {
        invoke('send_overlay_config', { payload: { edit_mode: true } }).catch(() => {});
    });

    startBtn.addEventListener('click', async () => {
        const port = parseInt(portInput.value) || 49123;
        await invoke('start_stats_stream', { port }).catch(e => showToast(String(e), 'error'));
        setStreamStatus(true);
    });

    stopBtn.addEventListener('click', async () => {
        await invoke('stop_stats_stream').catch(() => {});
        setStreamStatus(false);
    });

    enableApiBtn.addEventListener('click', async () => {
        const port = parseInt(portInput.value) || 49123;
        try {
            const path = await invoke('enable_stats_api', { port });
            showToast(`Written: ${path} — restart RL`, 'success');
        } catch (e) {
            showToast(String(e), 'error');
        }
    });

    // Game widget position buttons
    let currentGamePos = 'top-center';
    let currentHudPos  = 'top-right';
    let clock24h = false;

    async function savePos() {
        const existing = await invoke('get_config').catch(() => ({}));
        invoke('save_config', { config: { ...existing, overlay_position: currentGamePos, hud_position: currentHudPos } }).catch(() => {});
    }

    document.querySelectorAll('.game-pos-btn').forEach(b => b.addEventListener('click', () => {
        currentGamePos = b.dataset.pos;
        document.querySelectorAll('.game-pos-btn').forEach(x => x.classList.toggle('active', x === b));
        invoke('send_overlay_config', { payload: { game_pos: currentGamePos } }).catch(() => {});
        savePos();
    }));

    document.querySelectorAll('.hud-pos-btn').forEach(b => b.addEventListener('click', () => {
        currentHudPos = b.dataset.pos;
        document.querySelectorAll('.hud-pos-btn').forEach(x => x.classList.toggle('active', x === b));
        invoke('send_overlay_config', { payload: { hud_pos: currentHudPos } }).catch(() => {});
        savePos();
    }));

    // Clock format
    async function saveClock() {
        const existing = await invoke('get_config').catch(() => ({}));
        invoke('save_config', { config: { ...existing, clock_24h: clock24h } }).catch(() => {});
    }

    document.getElementById('clock-12h-btn')?.addEventListener('click', () => {
        clock24h = false;
        document.getElementById('clock-12h-btn').classList.add('active');
        document.getElementById('clock-24h-btn').classList.remove('active');
        invoke('send_overlay_config', { payload: { clock_24h: false } }).catch(() => {});
        saveClock();
    });
    document.getElementById('clock-24h-btn')?.addEventListener('click', () => {
        clock24h = true;
        document.getElementById('clock-24h-btn').classList.add('active');
        document.getElementById('clock-12h-btn').classList.remove('active');
        invoke('send_overlay_config', { payload: { clock_24h: true } }).catch(() => {});
        saveClock();
    });

    // Session reset
    document.getElementById('session-reset-btn')?.addEventListener('click', () => {
        invoke('send_overlay_config', { payload: { reset_session: true } }).catch(() => {});
        showToast('Session reset', 'success');
    });

    saveBtn.addEventListener('click', async () => {
        const existing = await invoke('get_config').catch(() => ({}));
        const username = usernameInput?.value.trim() || '';
        const port     = parseInt(portInput?.value) || 49123;
        await invoke('save_config', { config: { ...existing, rl_username: username, overlay_position: currentGamePos, hud_position: currentHudPos, stats_port: port, clock_24h: clock24h } })
            .catch(() => {});
        invoke('send_overlay_config', { payload: { player_name: username, game_pos: currentGamePos, hud_pos: currentHudPos, clock_24h: clock24h } }).catch(() => {});
        showToast('Saved', 'success');
    });

    // Populate from config
    invoke('get_config').then(cfg => {
        if (!cfg) return;
        if (usernameInput) usernameInput.value = cfg.rl_username || '';
        if (portInput)     portInput.value     = cfg.stats_port || 49123;
        currentGamePos = cfg.overlay_position || 'top-center';
        currentHudPos  = cfg.hud_position || 'top-right';
        clock24h       = cfg.clock_24h || false;
        document.querySelectorAll('.game-pos-btn').forEach(b => b.classList.toggle('active', b.dataset.pos === currentGamePos));
        document.querySelectorAll('.hud-pos-btn').forEach(b =>  b.classList.toggle('active', b.dataset.pos === currentHudPos));
        document.getElementById('clock-12h-btn')?.classList.toggle('active', !clock24h);
        document.getElementById('clock-24h-btn')?.classList.toggle('active', clock24h);
    }).catch(() => {});
}
