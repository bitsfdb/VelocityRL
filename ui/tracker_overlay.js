const invoke = window.__TAURI__?.core?.invoke || (async () => null);
const listen = window.__TAURI__?.event?.listen || null;

const THEME_SIZES = {
    circle: { w: 280, h: 165 },
    minimal: { w: 210, h: 45 },
    redesigned: { w: 165, h: 100 },
};

let currentStyle = 'circle';
let isOverlayLocked = true;
let currentScale = 100;

let currentState = {
    wins: 0,
    losses: 0,
    streak: 0,
    streak_type: 'none',
    session_rating: 1000,
    net_mmr: 0,
    connection_status: 'disconnected'
};

async function applyStyle(style) {
    if (!THEME_SIZES[style]) style = 'circle';
    currentStyle = style;

    const wrap = document.getElementById('overlay-wrap');
    if (wrap) {
        wrap.classList.remove('theme-circle', 'theme-minimal', 'theme-redesigned');
        wrap.classList.add(`theme-${style}`);
    }

    await applyScale(currentScale);
}

async function applyScale(scaleVal) {
    const num = Math.max(1, Math.min(250, Number(scaleVal) || 100));
    currentScale = num;
    const factor = num / 100;

    const wrap = document.getElementById('overlay-wrap');
    const dims = THEME_SIZES[currentStyle] || THEME_SIZES.circle;
    if (wrap) {
        wrap.style.position = 'absolute';
        wrap.style.top = '0';
        wrap.style.left = '0';
        wrap.style.width = `${dims.w}px`;
        wrap.style.height = `${dims.h}px`;
        wrap.style.transform = `scale(${factor})`;
        wrap.style.transformOrigin = '0 0';
    }

    const winW = Math.round(dims.w * factor);
    const winH = Math.round(dims.h * factor);
    await invoke('tracker_set_overlay_size', { width: winW, height: winH }).catch(() => {});
}

function applyOpacity(opacityVal) {
    const num = Math.max(10, Math.min(100, Number(opacityVal) || 85));
    const factor = (num / 100).toFixed(2);
    document.documentElement.style.setProperty('--overlay-opacity', factor);
}

function setText(el, newText) {
    if (!el) return;
    if (el.textContent !== newText) {
        el.textContent = newText;
    }
}

function updateHUD(data) {
    if (!data) return;
    Object.assign(currentState, data);

    const streakCount = currentState.streak?.count ?? currentState.streak ?? 0;
    const streakType = currentState.streak?.type ?? currentState.streak_type ?? 'none';
    const wins = currentState.wins ?? 0;
    const losses = currentState.losses ?? 0;

    let streakStr = '0';
    if (streakType === 'win' && streakCount > 0) {
        streakStr = `+${streakCount}`;
    } else if (streakType === 'loss' && streakCount > 0) {
        streakStr = `-${streakCount}`;
    } else if (streakCount !== 0) {
        streakStr = streakCount > 0 ? `+${streakCount}` : `${streakCount}`;
    }

    const winsStr = String(wins);
    const lossStr = String(losses);

    const isConn = currentState.connection === 'connected' || currentState.connection_status === 'connected';
    const detail = currentState.status_detail || '';
    ['conn-dot', 'conn-dot-min', 'conn-dot-red'].forEach(id => {
        const dot = document.getElementById(id);
        if (dot) {
            dot.classList.toggle('ok', isConn);
            dot.title = isConn ? 'Stats API: Connected' : (detail || 'Stats API: Searching Rocket League...');
        }
    });

    ['val-streak', 'min-val-streak', 'red-val-streak'].forEach(id => {
        setText(document.getElementById(id), streakStr);
    });

    ['val-wins', 'min-val-wins', 'red-val-wins'].forEach(id => {
        setText(document.getElementById(id), winsStr);
    });

    ['val-losses', 'min-val-losses', 'red-val-losses'].forEach(id => {
        setText(document.getElementById(id), lossStr);
    });
}

function setOverlayLocked(locked) {
    isOverlayLocked = locked;
    const wrap = document.getElementById('overlay-wrap');
    if (wrap) wrap.classList.toggle('is-unlocked', !locked);
    const bar = document.getElementById('unlock-bar');
    if (bar) bar.style.display = locked ? 'none' : 'flex';
}

document.addEventListener('mousedown', async (e) => {
    if (isOverlayLocked) return;
    if (e.target.closest('button, .lock-btn')) return;
    if (e.button !== 0) return;

    try {
        await invoke('tracker_start_dragging');
    } catch (_) {}
});

document.getElementById('btn-lock')?.addEventListener('click', async (e) => {
    e.stopPropagation();
    setOverlayLocked(true);
    await invoke('tracker_set_overlay_locked', { locked: true }).catch(() => {});
});

document.addEventListener('dblclick', async (e) => {
    if (e.target.closest('button, .lock-btn')) return;
    const newLocked = !isOverlayLocked;
    setOverlayLocked(newLocked);
    await invoke('tracker_set_overlay_locked', { locked: newLocked }).catch(() => {});
});

if (listen) {
    listen('overlay-state', (event) => updateHUD(event.payload));
    listen('tracker-overlay-locked', (event) => setOverlayLocked(event.payload === true));
    listen('tracker-style-changed', (event) => applyStyle(event.payload));
    listen('tracker-scale-changed', (event) => applyScale(event.payload));
    listen('tracker-opacity-changed', (event) => applyOpacity(event.payload));
}

async function syncInitialState() {
    try {
        const session = await invoke('tracker_load_session').catch(() => null);
        if (session) {
            if (session.overlay_style) await applyStyle(session.overlay_style);
            if (session.scale !== undefined) await applyScale(session.scale);
            if (session.opacity !== undefined) applyOpacity(session.opacity);
            if (session.is_locked !== undefined) setOverlayLocked(session.is_locked);
        } else {
            await applyStyle('circle');
            await applyScale(100);
        }

        const state = await invoke('get_overlay_state').catch(() => null);
        if (state) updateHUD(state);
    } catch (err) {
        console.warn('Initial overlay sync:', err);
    }
}

window.simulateMatch = (win) => invoke('test_simulate_match', { win: !!win }).catch(console.error);
window.resetSession = () => invoke('reset_session').catch(console.error);
window.applyScale = (val) => applyScale(val);
window.applyOpacity = (val) => applyOpacity(val);

function init() {
    updateHUD(currentState);
    syncInitialState();
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
} else {
    init();
}
