/* Presentation only: all game operations remain in main.js / Tauri. */
(() => {
    'use strict';
    const root = document.documentElement;
    // Motion is a built-in part of this design, not a stored user toggle.
    root.dataset.motion = 'on';
    const running = new Set();
    const ease = 'cubic-bezier(.16, 1, .3, 1)';
    function animate(element, frames, options = {}) {
        if (!element?.animate) return;
        const animation = element.animate(frames, { duration: 560, easing: ease, ...options });
        running.add(animation);
        animation.finished.catch(() => {}).finally(() => running.delete(animation));
        return animation;
    }
    document.getElementById('studio-explore').addEventListener('click', () => {
        const select = document.getElementById('catalog-category-select');
        select.value = 'Body';
        select.dispatchEvent(new Event('change', { bubbles: true }));
        document.getElementById('catalog-search-input').focus({ preventScroll: true });
        document.querySelector('.catalog-toolbar').scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    });
    // Make existing div-based navigation operable without a pointer.
    document.querySelectorAll('.nav-item').forEach(item => {
        item.setAttribute('role', 'button');
        item.tabIndex = 0;
        item.setAttribute('aria-label', item.textContent.trim());
        item.addEventListener('keydown', event => {
            if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); item.click(); }
        });
    });
    document.querySelectorAll('.close-modal-btn').forEach(item => {
        item.setAttribute('role', 'button');
        item.setAttribute('aria-label', 'Close dialog');
        item.tabIndex = 0;
        item.addEventListener('keydown', event => {
            if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); item.click(); }
        });
    });
    // A continuous selection track connects destinations instead of blinking on/off.
    const navTrack = document.createElement('div');
    navTrack.className = 'nav-track';
    navTrack.setAttribute('aria-hidden', 'true');
    const navList = document.querySelector('.nav-links');
    navList.prepend(navTrack);
    let trackAnimation;
    function moveTrack() {
        const active = navList.querySelector('.nav-item.active');
        if (!active) return;
        const before = navTrack.getBoundingClientRect();
        const after = active.getBoundingClientRect();
        trackAnimation?.cancel();
        navTrack.style.top = `${active.offsetTop}px`;
        navTrack.style.height = `${after.height}px`;
        if (before.height) trackAnimation = animate(navTrack, [
            { transform: `translateY(${before.top - after.top}px) scaleY(${before.height / after.height})` },
            { transform: `translateY(${(after.top - before.top) * .045}px) scaleY(.96)`, offset: .72 },
            { transform: 'translateY(0) scaleY(1)' }
        ], { duration: 470 });
    }
    new ResizeObserver(moveTrack).observe(navList);
    let direction = 1;
    document.addEventListener('click', event => {
        const button = event.target.closest('.nav-item[data-tab], .subtab-btn[data-subtab]');
        if (!button || button.classList.contains('active') || document.body.classList.contains('is-loading')) return;
        const peers = [...button.parentElement.querySelectorAll(button.matches('.nav-item') ? '.nav-item[data-tab]' : '.subtab-btn[data-subtab]')];
        direction = peers.indexOf(button) > peers.findIndex(el => el.classList.contains('active')) ? 1 : -1;
        root.dataset.travel = direction === 1 ? 'forward' : 'back';
    });
    const panelObserver = new MutationObserver(records => {
        for (const { target, oldValue } of records) {
            if (!target.classList.contains('active') || (oldValue || '').split(' ').includes('active')) continue;
            if (target.matches('.tab-content')) {
                const nav = document.querySelector(`.nav-item[data-tab="${target.id}"]`);
                document.getElementById('studio-page-name').textContent = nav?.textContent.trim() || 'Workspace';
                document.querySelectorAll('.nav-item[data-tab]').forEach(item => item.setAttribute('aria-pressed', String(item === nav)));
                document.querySelector('main').scrollTop = 0;
                moveTrack();
            }
            const strip = target.matches('.subtab-pane')
                ? target.parentElement.querySelector('.subtab-strip')
                : target.querySelector('.subtab-strip');
            if (strip) updateSegment(strip);
        }
    });
    document.querySelectorAll('.tab-content, .subtab-pane').forEach(panel => panelObserver.observe(panel, { attributes: true, attributeFilter: ['class'], attributeOldValue: true }));
    const grid = document.getElementById('catalog-grid');
    const gridObserver = new MutationObserver(records => {
        const added = records.flatMap(record => [...record.addedNodes]).filter(node => node.nodeType === 1 && node.matches('.catalog-card'));
        if (!added.length) return;
        added.slice(0, 12).forEach((card, index) => animate(card, [
            { transform: 'perspective(700px) rotateX(6deg) scale(.95)', opacity: .4 },
            { transform: 'perspective(700px) rotateX(-1deg) scale(1.006)', opacity: 1, offset: .7 },
            { transform: 'perspective(700px) rotateX(0deg) scale(1)', opacity: 1 }
        ], { delay: Math.min(index % 6, 5) * 18, duration: 320 }));
    });
    gridObserver.observe(grid, { childList: true });
    document.addEventListener('click', event => {
        const card = event.target.closest('.catalog-card');
        if (card) animate(card, [{ transform: 'scale(.96)' }, { transform: 'scale(1.025)', offset: .65 }, { transform: 'scale(1)' }], { duration: 420 });
    });
    document.addEventListener('keydown', event => {
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
            const search = document.querySelector('.tab-content.active .catalog-search-input');
            if (search) { event.preventDefault(); search.focus(); search.select(); }
        }
    });
    // Focus containment for existing modal dialogs, including stacked dialogs.
    const modalFocus = new WeakMap();
    const modalObserver = new MutationObserver(records => {
        records.forEach(({ target, oldValue }) => {
            const wasOpen = (oldValue || '').split(' ').includes('active');
            const isOpen = target.classList.contains('active');
            if (isOpen && !wasOpen) {
                modalFocus.set(target, document.activeElement);
                target.setAttribute('role', 'dialog');
                target.setAttribute('aria-modal', 'true');
                const heading = target.querySelector('h2');
                if (heading && !target.hasAttribute('aria-labelledby')) {
                    heading.id ||= `${target.id}-heading`;
                    target.setAttribute('aria-labelledby', heading.id);
                }
                target.querySelector('input:not([disabled]), select, button, [tabindex="0"]')?.focus();
            } else if (!isOpen && wasOpen) { modalFocus.get(target)?.focus(); }
        });
    });
    document.querySelectorAll('.modal-overlay').forEach(modal => modalObserver.observe(modal, { attributes: true, attributeFilter: ['class'], attributeOldValue: true }));
    document.addEventListener('keydown', event => {
        const modals = [...document.querySelectorAll('.modal-overlay.active')];
        const modal = modals.at(-1);
        if (!modal || event.key !== 'Tab') return;
        const focusable = [...modal.querySelectorAll('button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea, a[href], [tabindex="0"]')].filter(el => el.getClientRects().length);
        if (!focusable.length) { event.preventDefault(); return; }
        const first = focusable[0], last = focusable.at(-1);
        if (event.shiftKey && (document.activeElement === first || !modal.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && (document.activeElement === last || !modal.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
    });
    // Sliding segmented controls keep spatial continuity within each section.
    const segmentPositions = new WeakMap();
    function updateSegment(strip) {
        const active = strip.querySelector('.subtab-btn.active');
        if (!active) return;
        let track = strip.querySelector('.segment-track');
        if (!track) {
            track = document.createElement('span');
            track.className = 'segment-track';
            track.setAttribute('aria-hidden', 'true');
            strip.prepend(track);
        }
        const last = segmentPositions.get(strip);
        const x = active.offsetLeft, width = active.offsetWidth;
        track.getAnimations().forEach(a => a.cancel());
        track.style.left = `${x}px`;
        track.style.width = `${width}px`;
        track.style.height = `${active.offsetHeight}px`;
        if (last && width) animate(track, [
            { transform: `translateX(${last.x - x}px) scaleX(${last.width / width})` },
            { transform: 'translateX(0) scaleX(1)' }
        ], { duration: 400, easing: 'cubic-bezier(.2, 1.25, .35, 1)' });
        segmentPositions.set(strip, { x, width });
    }
    document.querySelectorAll('.subtab-strip').forEach(strip => new ResizeObserver(() => updateSegment(strip)).observe(strip));

    // Custom dark graphite dropdowns with rich color swatches and accessible keyboard navigation.
    const PAINT_COLORS = {
        0: 'linear-gradient(135deg, #3a3d45 45%, #6a6f7d 45%, #6a6f7d 55%, #3a3d45 55%)',
        all: 'conic-gradient(from 0deg, #ff0055, #ffaa00, #00ff66, #00bbff, #bb00ff, #ff0055)',
        1: '#DC143C', 2: '#32CD32', 3: '#18181b', 4: '#00B4FF', 5: '#0047AB',
        6: '#8B4513', 7: '#228B22', 8: '#800080', 9: '#FF69B4', 10: '#FF8C00',
        11: '#808080', 12: '#F5F5F5', 13: '#F4C430', 14: '#FFD700',
        15: '#B76E79', 16: '#D4AF37', 17: '#353839', 18: '#E5E4E2',
        19: '#38BDF8', 20: '#3B82F6', 21: '#A26C45', 22: '#4ADE80',
        23: '#BEF264', 24: '#FB923C', 25: '#F472B6', 26: '#C084FC',
        27: '#F87171', 28: '#FFFFFF', 29: '#FDE047',
    };
    const RARITY_COLORS = {
        'Common': '#9ca3af', 'Uncommon': '#10b981', 'Rare': '#3b82f6',
        'Very Rare': '#8b5cf6', 'Import': '#ef4444', 'Exotic': '#f59e0b',
        'Black Market': '#ec4899', 'Premium': '#06b6d4', 'Limited': '#f97316'
    };

    function closeAllCustomSelects() {
        document.querySelectorAll('.catalog-select-wrap.is-open').forEach(w => {
            w.classList.remove('is-open');
            w.querySelector('.custom-select-trigger')?.setAttribute('aria-expanded', 'false');
        });
    }

    function initCustomSelects() {
        document.querySelectorAll('.catalog-select-wrap').forEach(wrap => {
            if (wrap.dataset.customized) return;
            const select = wrap.querySelector('select');
            if (!select) return;
            wrap.dataset.customized = 'true';

            const isPaint = select.id === 'catalog-paint-select';
            const isRarity = select.id === 'catalog-rarity-select';

            const trigger = document.createElement('div');
            trigger.className = 'custom-select-trigger';
            trigger.tabIndex = 0;
            trigger.setAttribute('role', 'combobox');
            trigger.setAttribute('aria-expanded', 'false');

            const leadingIcon = wrap.querySelector('.catalog-toolbar-icon, .catalog-paint-dot');
            if (leadingIcon) trigger.appendChild(leadingIcon);

            const label = document.createElement('span');
            label.className = 'custom-select-label';
            trigger.appendChild(label);

            const chevron = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
            chevron.setAttribute('class', 'custom-select-chevron');
            chevron.setAttribute('width', '13');
            chevron.setAttribute('height', '13');
            chevron.setAttribute('viewBox', '0 0 24 24');
            chevron.setAttribute('fill', 'none');
            chevron.setAttribute('stroke', 'currentColor');
            chevron.setAttribute('stroke-width', '2');
            chevron.setAttribute('stroke-linecap', 'round');
            chevron.setAttribute('stroke-linejoin', 'round');
            chevron.innerHTML = '<path d="m6 9 6 6 6-6"/>';
            trigger.appendChild(chevron);

            wrap.appendChild(trigger);

            const menu = document.createElement('div');
            menu.className = 'custom-select-menu';
            menu.setAttribute('role', 'listbox');
            wrap.appendChild(menu);

            function populateOptions() {
                menu.innerHTML = '';
                Array.from(select.options).forEach(opt => {
                    const item = document.createElement('div');
                    item.className = 'custom-select-option' + (opt.selected ? ' is-selected' : '');
                    item.dataset.value = opt.value;
                    item.setAttribute('role', 'option');
                    item.setAttribute('aria-selected', String(opt.selected));

                    if (isPaint) {
                        const dot = document.createElement('span');
                        dot.className = 'custom-select-opt-dot';
                        const c = PAINT_COLORS[opt.value] || '#555';
                        dot.style.background = c;
                        if (Number(opt.value) >= 19 && Number(opt.value) <= 29) {
                            dot.style.boxShadow = `0 0 7px ${c}`;
                        }
                        item.appendChild(dot);
                    } else if (isRarity && RARITY_COLORS[opt.value]) {
                        const dot = document.createElement('span');
                        dot.className = 'custom-select-opt-dot';
                        dot.style.background = RARITY_COLORS[opt.value];
                        item.appendChild(dot);
                    }

                    const text = document.createElement('span');
                    text.textContent = opt.textContent;
                    item.appendChild(text);

                    const check = document.createElement('span');
                    check.className = 'custom-select-opt-check';
                    check.innerHTML = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>';
                    item.appendChild(check);

                    item.addEventListener('click', (e) => {
                        e.stopPropagation();
                        if (select.value !== opt.value) {
                            select.value = opt.value;
                            select.dispatchEvent(new Event('change', { bubbles: true }));
                        }
                        closeAllCustomSelects();
                    });

                    menu.appendChild(item);
                });
                syncSelection();
            }

            function syncSelection() {
                const opt = select.selectedOptions[0] || select.options[0];
                label.textContent = opt ? opt.textContent : '';
                menu.querySelectorAll('.custom-select-option').forEach(item => {
                    const sel = item.dataset.value === select.value;
                    item.classList.toggle('is-selected', sel);
                    item.setAttribute('aria-selected', String(sel));
                });
            }

            populateOptions();
            new MutationObserver(populateOptions).observe(select, { childList: true });
            select.addEventListener('change', syncSelection);

            trigger.addEventListener('click', (e) => {
                e.stopPropagation();
                const isOpen = wrap.classList.contains('is-open');
                closeAllCustomSelects();
                if (!isOpen) {
                    wrap.classList.add('is-open');
                    trigger.setAttribute('aria-expanded', 'true');
                    const sel = menu.querySelector('.custom-select-option.is-selected');
                    if (sel) sel.scrollIntoView({ block: 'nearest' });
                }
            });

            trigger.addEventListener('keydown', (e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    trigger.click();
                } else if (e.key === 'Escape') {
                    closeAllCustomSelects();
                } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                    e.preventDefault();
                    if (!wrap.classList.contains('is-open')) {
                        trigger.click();
                        return;
                    }
                    const items = Array.from(menu.querySelectorAll('.custom-select-option'));
                    const currentIndex = items.findIndex(i => i.classList.contains('is-selected'));
                    let nextIndex = e.key === 'ArrowDown' ? currentIndex + 1 : currentIndex - 1;
                    if (nextIndex >= 0 && nextIndex < items.length) {
                        items[nextIndex].click();
                        wrap.classList.add('is-open');
                        trigger.setAttribute('aria-expanded', 'true');
                    }
                }
            });
        });
    }

    initCustomSelects();
    document.addEventListener('click', (e) => {
        if (!e.target.closest('.catalog-select-wrap')) closeAllCustomSelects();
    });
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') closeAllCustomSelects();
    });

    // Pointer-linked depth is local to the hovered card, with a soft return to rest.
    let hoveredCard;
    document.addEventListener('pointermove', event => {
        const next = event.target.closest('.catalog-card');
        if (hoveredCard !== next) {
            hoveredCard?.style.removeProperty('rotate');
            hoveredCard = next;
        }
        if (!next || event.pointerType === 'touch') return;
        const rect = next.getBoundingClientRect();
        next.style.rotate = `${-(event.clientY - rect.top - rect.height / 2) / rect.height} ${(event.clientX - rect.left - rect.width / 2) / rect.width} 0 5deg`;
    }, { passive: true });
    document.documentElement.addEventListener('pointerleave', () => { hoveredCard?.style.removeProperty('rotate'); hoveredCard = null; });

    // Opaque startup lockup: chevrons assemble, accelerate, then the shell unfolds.
    const boot = document.getElementById('velocity-boot');
    const bootStart = performance.now();
    let bootFinished = false;
    function finishBoot() {
        if (bootFinished) return;
        bootFinished = true;
        const delay = Math.max(0, 1350 - (performance.now() - bootStart));
        setTimeout(() => {
            boot.classList.add('is-leaving');
            animate(document.querySelector('.sidebar'), [
                { transform: 'translateX(-42px)' }, { transform: 'translateX(2px)', offset: .75 }, { transform: 'translateX(0)' }
            ], { duration: 700 });
            animate(document.querySelector('.main-wrap'), [
                { transform: 'perspective(1400px) rotateY(-3deg) translateX(50px)', transformOrigin: 'left center' },
                { transform: 'perspective(1400px) rotateY(0deg) translateX(0)', transformOrigin: 'left center' }
            ], { duration: 850 });
            setTimeout(() => boot.remove(), 850);
        }, delay);
    }
    const bootObserver = new MutationObserver(() => {
        if (!document.body.classList.contains('is-loading')) { finishBoot(); bootObserver.disconnect(); }
    });
    bootObserver.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    if (!document.body.classList.contains('is-loading')) { finishBoot(); bootObserver.disconnect(); }
    // Window control buttons for borderless custom window frame.
    const winMin = document.getElementById('win-btn-minimize');
    const winMax = document.getElementById('win-btn-maximize');
    const winClose = document.getElementById('win-btn-close');
    const tauriInvoke = (...args) => window.__TAURI__?.core?.invoke ? window.__TAURI__.core.invoke(...args) : Promise.resolve();

    if (winMin) {
        winMin.addEventListener('click', (e) => {
            e.stopPropagation();
            tauriInvoke('app_window_minimize').catch(() => {});
        });
    }
    if (winMax) {
        winMax.addEventListener('click', (e) => {
            e.stopPropagation();
            tauriInvoke('app_window_toggle_maximize').catch(() => {});
        });
    }
    if (winClose) {
        winClose.addEventListener('click', (e) => {
            e.stopPropagation();
            tauriInvoke('app_window_close').catch(() => {});
        });
    }

    const topbar = document.querySelector('.studio-topbar');
    if (topbar) {
        topbar.addEventListener('dblclick', (e) => {
            if (e.target.closest('button, input, select, a, .win-btn, [role="button"]')) return;
            tauriInvoke('app_window_toggle_maximize').catch(() => {});
        });
    }

    // Decorative intro never becomes a new application loading gate.
    setTimeout(finishBoot, 4500);

    // Moving contour field. One bounded canvas, no per-frame DOM updates.
    const canvas = document.getElementById('velocity-field');
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    let width = 0, height = 0, frame = 0, previous = 0, time = 0;
    let pointerX = .7, pointerY = .4, targetX = .7, targetY = .4;
    function resizeField() {
        width = innerWidth; height = innerHeight;
        const ratio = Math.min(devicePixelRatio || 1, 1.5);
        canvas.width = Math.round(width * ratio); canvas.height = Math.round(height * ratio);
        ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    }
    resizeField();
    addEventListener('resize', resizeField, { passive: true });
    addEventListener('pointermove', event => { targetX = event.clientX / width; targetY = event.clientY / height; }, { passive: true });
    function drawField(now) {
        frame = requestAnimationFrame(drawField);
        if (now - previous < 32) return;
        time += Math.min((now - (previous || now)) / 1000, .05);
        previous = now;
        pointerX += (targetX - pointerX) * .045;
        pointerY += (targetY - pointerY) * .045;
        ctx.clearRect(0, 0, width, height);
        for (let line = 0; line < 26; line++) {
            ctx.beginPath();
            for (let step = 0; step <= 54; step++) {
                const x = width * step / 54;
                const wave = Math.sin(x / width * 5 + time * .28 + line * .14);
                const wave2 = Math.cos(x / width * 8 - time * .18 + line * .08);
                const distance = (x / width - pointerX) * 3;
                const y = height * .2 + line * 22 + wave * 58 + wave2 * 30 + Math.exp(-distance * distance) * (pointerY - .5) * 100;
                if (!step) ctx.moveTo(x, y); else ctx.lineTo(x, y);
            }
            ctx.strokeStyle = `rgba(131,148,174,${.055 + (Math.sin(line * .25 + time * .35) + 1) * .035})`;
            ctx.lineWidth = 1;
            ctx.stroke();
        }
        for (let i = 0; i < 18; i++) {
            const x = ((i * .137 + time * .022) % 1) * width;
            const y = ((i * .173) % 1) * height + Math.sin(time + i) * 16;
            ctx.fillStyle = 'rgba(167,182,202,.24)';
            ctx.fillRect(x, y, 14 + i % 4 * 7, 1);
        }
    }
    frame = requestAnimationFrame(drawField);
    document.addEventListener('visibilitychange', () => {
        cancelAnimationFrame(frame);
        document.body.classList.toggle('motion-suspended', document.hidden);
        if (!document.hidden) { previous = 0; frame = requestAnimationFrame(drawField); }
    });
})();
