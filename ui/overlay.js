const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

// ── Elements ──────────────────────────────────────────────────────────────
const gameWidget  = document.getElementById('game-widget');
const hudWidget   = document.getElementById('hud-widget');
const sessWins    = document.getElementById('sess-wins');
const sessLosses  = document.getElementById('sess-losses');
const sessStreak  = document.getElementById('sess-streak');
const mmrVal      = document.getElementById('mmr-val');
const clockVal    = document.getElementById('clock-val');

// ── State ─────────────────────────────────────────────────────────────────
let rlFocused = false;
let clock24h  = false;
let localName = '';
let wins = 0, losses = 0, streak = 0;
let myTeam = null;
let cachedMmr = null;

// ── Show / hide ───────────────────────────────────────────────────────────
function setVisible(v) {
    rlFocused = v;
    gameWidget.classList.toggle('hidden', !v);
    hudWidget.classList.toggle('hidden', !v);
}

// ── Session ───────────────────────────────────────────────────────────────
function updateSession() {
    sessWins.textContent   = wins;
    sessLosses.textContent = losses;
    const s = streak;
    sessStreak.textContent = s > 0 ? `+${s}` : String(s);
    sessStreak.className   = 'sess-val ' + (s > 0 ? 'green' : s < 0 ? 'red' : '');
}

// ── Clock ─────────────────────────────────────────────────────────────────
function tickClock() {
    const now = new Date();
    if (clock24h) {
        const h = String(now.getHours()).padStart(2, '0');
        const m = String(now.getMinutes()).padStart(2, '0');
        clockVal.textContent = `${h}:${m}`;
    } else {
        let h = now.getHours();
        const ampm = h >= 12 ? 'PM' : 'AM';
        h = h % 12 || 12;
        const m = String(now.getMinutes()).padStart(2, '0');
        clockVal.textContent = `${h}:${m} ${ampm}`;
    }
}
tickClock();
setInterval(tickClock, 1000);

// ── MMR fetch ─────────────────────────────────────────────────────────────
async function fetchMmr() {
    try {
        const cfg = await invoke('get_config');
        if (!cfg.rl_username) { mmrVal.textContent = '?'; return; }
        mmrVal.textContent = '…';
        const data = await invoke('fetch_mmr', {
            platform: cfg.rl_platform || 'epic',
            username: cfg.rl_username,
            apiKey:   cfg.trn_api_key || '',
        });
        const segs = data?.data?.segments || [];
        const ranked = segs.find(s => s.type === 'playlist' && /ranked|standard|doubles|duel/i.test(s.metadata?.name || ''))
                    || segs.find(s => s.type === 'playlist');
        if (ranked) {
            const m = ranked.stats?.rating?.value;
            if (m !== undefined) {
                cachedMmr = Math.round(m);
                mmrVal.textContent = cachedMmr;
                return;
            }
        }
        mmrVal.textContent = 'N/A';
    } catch (e) {
        mmrVal.textContent = 'err';
    }
}

// ── Stats events ──────────────────────────────────────────────────────────
function onMatchEnd(data) {
    const winner = data?.WinnerTeamNum;
    if (winner !== undefined && myTeam !== null) {
        const won = (winner === myTeam);
        if (won) { wins++; streak = streak >= 0 ? streak + 1 : 1; }
        else     { losses++; streak = streak <= 0 ? streak - 1 : -1; }
        updateSession();
    }
    myTeam = null;
    fetchMmr();
}

function onMatchStart() {
    myTeam = null;
}

function onUpdateState(data) {
    const players = data?.Players || [];
    // Identify local player's team using stored rl_username — Stats API has no isLocalPlayer flag
    if (myTeam === null && localName && players.length > 0) {
        const me = players.find(p => p.Name === localName);
        if (me != null) myTeam = me.TeamNum ?? null;
    }
}

// ── Position ──────────────────────────────────────────────────────────────
const POS_CLASSES = ['pos-top-center','pos-top-left','pos-top-right','pos-bottom-left','pos-bottom-right'];

function applyPixelPos(el, x, y) {
    el.classList.remove(...POS_CLASSES);
    el.style.left = x + 'px'; el.style.top = y + 'px';
    el.style.right = 'auto'; el.style.bottom = 'auto';
    el.style.transform = 'none';
}

function setGamePos(pos) {
    const saved = JSON.parse(localStorage.getItem('gameWidgetPos') || 'null');
    if (saved) { applyPixelPos(gameWidget, saved.x, saved.y); return; }
    gameWidget.classList.remove(...POS_CLASSES);
    gameWidget.style.cssText = '';
    gameWidget.classList.add('pos-' + pos);
}
function setHudPos(pos) {
    const saved = JSON.parse(localStorage.getItem('hudWidgetPos') || 'null');
    if (saved) { applyPixelPos(hudWidget, saved.x, saved.y); return; }
    hudWidget.classList.remove(...POS_CLASSES);
    hudWidget.style.cssText = '';
    hudWidget.classList.add('pos-' + pos);
}

// ── Drag (edit mode) ──────────────────────────────────────────────────────
let editMode = false;

function makeDraggable(el, storageKey) {
    let ox = 0, oy = 0, dragging = false;
    el.addEventListener('mousedown', e => {
        if (!editMode) return;
        dragging = true;
        const rect = el.getBoundingClientRect();
        ox = e.clientX - rect.left;
        oy = e.clientY - rect.top;
        applyPixelPos(el, rect.left, rect.top);
        e.preventDefault();
    });
    document.addEventListener('mousemove', e => {
        if (!dragging) return;
        let x = Math.max(0, Math.min(e.clientX - ox, window.innerWidth  - el.offsetWidth));
        let y = Math.max(0, Math.min(e.clientY - oy, window.innerHeight - el.offsetHeight));
        el.style.left = x + 'px'; el.style.top = y + 'px';
    });
    document.addEventListener('mouseup', () => {
        if (!dragging) return;
        dragging = false;
        localStorage.setItem(storageKey, JSON.stringify({ x: parseInt(el.style.left), y: parseInt(el.style.top) }));
    });
}

async function enterEditMode() {
    editMode = true;
    gameWidget.classList.remove('hidden');
    hudWidget.classList.remove('hidden');
    gameWidget.classList.add('editable');
    hudWidget.classList.add('editable');
    document.getElementById('edit-bar').classList.remove('hidden');
    await invoke('set_overlay_passthrough', { enabled: false }).catch(() => {});
}

async function exitEditMode() {
    editMode = false;
    gameWidget.classList.remove('editable');
    hudWidget.classList.remove('editable');
    document.getElementById('edit-bar').classList.add('hidden');
    await invoke('set_overlay_passthrough', { enabled: true }).catch(() => {});
    setVisible(rlFocused);
}

// ── Init ──────────────────────────────────────────────────────────────────
async function init() {
    try {
        const cfg = await invoke('get_config');
        localName = cfg.rl_username || '';
        clock24h  = cfg.clock_24h  || false;
        setGamePos(cfg.overlay_position || 'top-center');
        setHudPos(cfg.hud_position      || 'top-right');
        if (cfg.rl_username) fetchMmr();
        else mmrVal.textContent = '?';
    } catch {}

    updateSession();
    setVisible(true); // show immediately; process watcher will hide if RL isn't running
    setInterval(fetchMmr, 5 * 60 * 1000); // refresh MMR every 5 minutes

    makeDraggable(gameWidget, 'gameWidgetPos');
    makeDraggable(hudWidget,  'hudWidgetPos');
    document.getElementById('edit-done-btn').addEventListener('click', exitEditMode);

    // Focus watcher — only show when RL is focused
    await listen('rl_focused', (e) => setVisible(e.payload));

    // Stats stream events — RL API uses PascalCase: { Event: "...", Data: { ... } }
    await listen('rl_stats', (e) => {
        const name = e.payload?.Event || '';
        const data = e.payload?.Data  || {};
        if (name === 'UpdateState')    onUpdateState(data);
        else if (name === 'MatchEnded')    onMatchEnd(data);
        else if (name === 'MatchCreated' || name === 'MatchInitialized') onMatchStart();
    });

    // Config updates relayed from main window via Rust
    await listen('overlay_config', (e) => {
        const cfg = e.payload;
        if (cfg.game_pos)      { localStorage.removeItem('gameWidgetPos'); setGamePos(cfg.game_pos); }
        if (cfg.hud_pos)       { localStorage.removeItem('hudWidgetPos');  setHudPos(cfg.hud_pos); }
        if (cfg.player_name !== undefined) {
            localName = cfg.player_name;
            myTeam = null;
            fetchMmr();
        }
        if (cfg.clock_24h   !== undefined) { clock24h = cfg.clock_24h; tickClock(); }
        if (cfg.reset_session) { wins = 0; losses = 0; streak = 0; updateSession(); }
        if (cfg.edit_mode)     enterEditMode();
    });
}

init();
