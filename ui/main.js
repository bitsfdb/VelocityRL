const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

let ownedItem = null;
let wantedItem = null;
let items = [];
let currentCategory = 'All';

// UI Elements
let ownedSearch, wantedSearch, ownedResults, wantedResults, applyBtn, statusText, progressBarContainer, progressFill, systemWarning;

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
    // Select elements after DOM is ready
    ownedSearch = document.getElementById('owned-search');
    wantedSearch = document.getElementById('wanted-search');
    ownedResults = document.getElementById('owned-results');
    wantedResults = document.getElementById('wanted-results');
    applyBtn = document.getElementById('apply-swap');
    statusText = document.getElementById('status-text');
    progressBarContainer = document.getElementById('progress-bar-container');
    progressFill = document.getElementById('progress-fill');
    systemWarning = document.getElementById('system-warning');

    setupSearch(ownedSearch, ownedResults, (item) => {
        ownedItem = item;
        const container = document.getElementById('owned-selected');
        container.innerHTML = `
            <h2 class="${qColorMap[item.Quality] || ''}">${item.Product}</h2>
            <span class="quality-badge ${qBgMap[item.Quality] || ''}">${item.Quality}</span>
            <p style="margin-top: 16px; font-size: 13px; color: var(--text-secondary)">${item.Slot}</p>
            <p style="font-family: monospace; font-size: 10px; color: var(--accent-blue); margin-top: 4px;">${item.AssetPackage}</p>
        `;
        container.classList.add('selected');
        ownedSearch.value = item.Product;
        validate();
    });

    setupSearch(wantedSearch, wantedResults, (item) => {
        wantedItem = item;
        const container = document.getElementById('wanted-selected');
        container.innerHTML = `
            <h2 class="${qColorMap[item.Quality] || ''}">${item.Product}</h2>
            <span class="quality-badge ${qBgMap[item.Quality] || ''}">${item.Quality}</span>
            <p style="margin-top: 16px; font-size: 13px; color: var(--text-secondary)">${item.Slot}</p>
            <p style="font-family: monospace; font-size: 10px; color: var(--accent-blue); margin-top: 4px;">${item.AssetPackage}</p>
        `;
        container.classList.add('selected');
        wantedSearch.value = item.Product;
        validate();
    });

    // Tab Switching
    document.querySelectorAll('.nav-item[data-tab]').forEach(btn => {
        btn.onclick = () => {
            document.querySelectorAll('.nav-item').forEach(b => b.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            
            btn.classList.add('active');
            document.getElementById(btn.dataset.tab).classList.add('active');
        };
    });

    // Category Filtering
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
    document.getElementById('close-settings').onclick = handleSaveSettings;
    document.getElementById('browse-dir').onclick = handleBrowse;

    try {
        updateStatus('Connecting Engine...', false);
        items = await invoke('get_items');
        updateStatus('bitsfdb', false); // Original "bitsfdb" as requested
        
        const config = await invoke('get_config');
        if (config.game_dir) {
            document.getElementById('game-dir').value = config.game_dir;
        }
    } catch (err) {
        updateStatus('Init Failure', true);
        console.error(err);
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
        if (term.length < 2 && currentCategory === 'All') {
            resultsDiv.style.display = 'none';
            return;
        }

        const matches = items.filter(item => {
            const matchesTerm = term.length < 2 || item.Product.toLowerCase().includes(term) || item.AssetPackage.toLowerCase().includes(term);
            const matchesCat = currentCategory === 'All' || item.Slot === currentCategory;
            return matchesTerm && matchesCat;
        }).slice(0, 50);

        renderResults(matches, resultsDiv, selectionHandler);
    });

    input.addEventListener('focus', () => {
        if (currentCategory !== 'All' && input.value === '') {
            const matches = items.filter(item => item.Slot === currentCategory).slice(0, 50);
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
        div.innerHTML = `
            <span class="item-name ${qColorMap[item.Quality] || ''}">${item.Product}</span>
            <span class="item-meta">${item.Slot} | ${item.AssetPackage}</span>
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
    
    const isUnsupported = (ownedItem && (ownedItem.Slot === 'Body' || ownedItem.Slot === 'Goal Explosion')) || 
                        (wantedItem && (wantedItem.Slot === 'Body' || wantedItem.Slot === 'Goal Explosion'));

    if (isUnsupported) {
        systemWarning.classList.remove('hidden');
    } else {
        systemWarning.classList.add('hidden');
    }
    applyBtn.disabled = !(ownedItem && wantedItem);
}

async function handleApply() {
    try {
        updateStatus('Initializing Swap Protocol...', false);
        showProgress(true, 15);
        applyBtn.disabled = true;
        
        let p = 15;
        const interval = setInterval(() => {
            if (p < 85) p += 5;
            showProgress(true, p);
        }, 400);

        await invoke('apply_swap', { 
            ownedId: ownedItem.ID.toString(), 
            wantedId: wantedItem.ID.toString() 
        });
        
        clearInterval(interval);
        showProgress(true, 100);
        updateStatus('Swap Complete', false);
        setTimeout(() => {
            showProgress(false);
            updateStatus('bitsfdb', false);
        }, 3000);
    } catch (err) {
        updateStatus('Swap Failed', true);
        showProgress(false);
        console.error(err);
    } finally {
        applyBtn.disabled = false;
    }
}

async function handleRestore() {
    try {
        updateStatus('Running Restoration...', false);
        const result = await invoke('restore_backups');
        updateStatus(result, false);
        setTimeout(() => updateStatus('bitsfdb', false), 3000);
    } catch (err) {
        updateStatus('Restore Failed', true);
        console.error(err);
    }
}

async function handleSaveSettings() {
    const dir = document.getElementById('game-dir').value;
    await invoke('save_config', { config: { game_dir: dir } });
    document.getElementById('settings-modal').classList.remove('active');
}

async function handleBrowse() {
    try {
        const selected = await open({
            directory: true,
            multiple: false,
            title: 'Select Directory'
        });
        if (selected) {
            document.getElementById('game-dir').value = selected;
        }
    } catch (err) {
        console.error(err);
    }
}

window.addEventListener('DOMContentLoaded', () => {
    init();
});
