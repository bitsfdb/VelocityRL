const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

let ownedItem = null;
let wantedItem = null;
let items = [];
let currentCategory = 'All';

let ownedSearch, wantedSearch, ownedResults, wantedResults, applyBtn, statusText, progressBarContainer, progressFill, systemWarning, backupContainer;

function showToast(message, type = 'success') {
    const container = document.getElementById('toast-container');
    if (!container) return;
    
    const toast = document.createElement('div');
    toast.className = `toast ${type}`;
    
    let content = message;
    if (type === 'error') {
        const discordLink = 'https://discord.gg/2HhBNbrGMj';
        content = `<div>${message}<br><a href="#" class="toast-link" onclick="event.preventDefault(); window.__TAURI__.core.invoke('plugin:shell|open', { path: '${discordLink}' })">Join Support Discord</a></div>`;
    }
    
    toast.innerHTML = `<div class="toast-content">${content}</div>`;
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
    systemWarning = document.getElementById('system-warning');
    backupContainer = document.getElementById('backup-container');

    setupSearch(ownedSearch, ownedResults, (item) => {
        ownedItem = item;
        const pName = item.Product || item.product || 'Unknown';
        const pQuality = item.Quality || item.quality || 'Common';
        const pSlot = item.Slot || item.slot || '';
        const pImg = item.image_url || item.src || '';

        const container = document.getElementById('owned-selected');
        container.innerHTML = `
            <div class="clear-item-btn" onclick="clearOwned()">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
            </div>
            ${pImg ? `<img src="${pImg}" class="selected-img" />` : ''}
            <h2>${pName}</h2>
            <span class="quality-badge">${pQuality}</span>
            <p style="margin-top: 16px; font-size: 13px; color: var(--text-secondary)">${pSlot}</p>
        `;
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
            <div class="clear-item-btn" onclick="clearWanted()">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
            </div>
            ${pImg ? `<img src="${pImg}" class="selected-img" />` : ''}
            <h2>${pName}</h2>
            <span class="quality-badge">${pQuality}</span>
            <p style="margin-top: 16px; font-size: 13px; color: var(--text-secondary)">${pSlot}</p>
        `;
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
    document.getElementById('settings-btn').onclick = () => document.getElementById('settings-modal').classList.add('active');
    document.getElementById('cancel-settings').onclick = () => document.getElementById('settings-modal').classList.remove('active');
    document.getElementById('close-settings').onclick = handleSaveSettings;
    document.getElementById('browse-dir').onclick = handleBrowse;
    document.getElementById('fetch-items-btn').onclick = handleFetchItems;

    document.getElementById('settings-modal').onclick = (e) => {
        if (e.target === document.getElementById('settings-modal')) {
            document.getElementById('settings-modal').classList.remove('active');
        }
    };

    try {
        updateStatus('Verifying Integrity...', false);
        await invoke('check_integrity').catch(e => {
            throw new Error(`Security Violation: ${e}`);
        });
        updateStatus('Initializing Engine...', false);
        items = await invoke('get_items').catch(e => { 
            console.error(e);
            throw new Error(`Database Error: ${e}`); 
        });
        const config = await invoke('get_config').catch(e => { console.warn('Config load failed:', e); return { game_dir: '' }; });
        if (config) {
            if (config.game_dir) document.getElementById('game-dir').value = config.game_dir;
        } else {
            updateStatus('Setup Required', true);
            setTimeout(() => {
                document.getElementById('settings-modal').classList.add('active');
                handleBrowse();
            }, 1000);
        }
        updateStatus('bitsfdb', false);
        invoke('cleanup_temp_files').catch(e => console.warn('Cleanup failed:', e));
    } catch (err) {
        updateStatus('Init Failure', true);
        alert(`VelocityRL Initialization Failed:\n${err.message || err}`);
        console.error(err);
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
            div.innerHTML = `
                <div>
                    <div class="backup-name">${file.name}</div>
                    <div class="backup-date">Modified Package</div>
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
            lockCategory = ownedItem.Slot || ownedItem.slot || 'All';
        }

        if (term.length < 2 && lockCategory === 'All') {
            resultsDiv.style.display = 'none';
            return;
        }

        const matches = items.filter(item => {
            const pName = (item.Product || item.product || '').toLowerCase();
            const pAsset = (item.AssetPackage || item.asset_package || '').toLowerCase();
            const pSlot = item.Slot || item.slot || '';

            const invalidTypes = ['Series', 'Crate', 'Currency', 'Premium', 'Unknown'];
            if (invalidTypes.includes(pSlot)) return false;

            const matchesTerm = term.length < 2 || pName.includes(term) || pAsset.includes(term);
            const matchesCat = lockCategory === 'All' || pSlot.toLowerCase() === lockCategory.toLowerCase();
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
                const s = (item.Slot || item.slot || '').toLowerCase();
                return s === lockCategory.toLowerCase();
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
            ${pImg ? `<img src="${pImg}" class="flyout-img" />` : '<div class="flyout-img"></div>'}
            <div class="flyout-info">
                <span class="item-name">${pName}</span>
                <span style="font-size: 10px; color: var(--text-secondary)">${pSlot}</span>
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

function validate() {
    if (!systemWarning || !applyBtn) return;
    
    const oSlot = ownedItem ? (ownedItem.Slot || ownedItem.slot) : null;
    const wSlot = wantedItem ? (wantedItem.Slot || wantedItem.slot) : null;

    const isUnsupported = (ownedItem && (oSlot === 'Body' || oSlot === 'Goal Explosion')) || 
                        (wantedItem && (wSlot === 'Body' || wSlot === 'Goal Explosion'));
    
    if (isUnsupported) systemWarning.classList.remove('hidden');
    else systemWarning.classList.add('hidden');

    const typesMatch = !ownedItem || !wantedItem || oSlot === wSlot;
    applyBtn.disabled = !(ownedItem && wantedItem && typesMatch);
}

async function handleApply() {
    try {
        updateStatus('Initializing Engine...', false);
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
        alert(`Swap Error: ${err}`);
        console.error(err);
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
        alert(`Restore Error: ${err}`);
        console.error(err);
    }
}

async function handleSaveSettings() {
    const dir = document.getElementById('game-dir').value;
    try {
        await invoke('save_config', { config: { game_dir: dir } });
        document.getElementById('settings-modal').classList.remove('active');
        refreshBackups();
        if (dir) showToast('Success!', 'success');
    } catch (err) {
        showToast('Failed! please report this to the maintainer bitsfdb on the discord support server', 'error');
        console.error(err);
    }
}

async function handleBrowse() {
    try {
        const selected = await open({ 
            directory: true, 
            multiple: false, 
            title: 'Select Rocket League CookedPCConsole Directory' 
        });
        if (selected) {
            document.getElementById('game-dir').value = selected;
            // If first time, auto-save
            const config = await invoke('get_config').catch(() => ({ game_dir: '' }));
            if (!config.game_dir) {
                handleSaveSettings();
            }
        }
    } catch (err) { 
        showToast('Failed! please report this to the maintainer bitsfdb on the discord support server', 'error');
        console.error(err); 
    }
}

async function handleFetchItems() {
    try {
        const token = prompt("Enter Epic Auth Token / Exchange Token:");
        if (!token) return;
        const account = prompt("Enter Epic Account ID (optional):") || "Unknown";
        
        updateStatus('Updating Database...', false);
        showProgress(true, 10);
        
        await invoke('fetch_catalog', { token, account });
        
        showProgress(true, 100);
        showToast('Database Updated!', 'success');
        updateStatus('bitsfdb', false);
        setTimeout(() => showProgress(false), 2000);
        
        // Reload items
        items = await invoke('get_items');
    } catch (err) {
        showToast('Update Failed: ' + err, 'error');
        updateStatus('Update Error', true);
        showProgress(false);
    }
}

window.addEventListener('DOMContentLoaded', () => init());
