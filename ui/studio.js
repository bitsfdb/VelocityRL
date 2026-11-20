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
    let viewTransition;
    document.addEventListener('click', event => {
        const button = event.target.closest('.nav-item[data-tab], .subtab-btn[data-subtab]');
        if (!button || button.classList.contains('active') || typeof button.onclick !== 'function' || document.body.classList.contains('is-loading')) return;
        const peers = [...button.parentElement.querySelectorAll(button.matches('.nav-item') ? '.nav-item[data-tab]' : '.subtab-btn[data-subtab]')];
        direction = peers.indexOf(button) > peers.findIndex(el => el.classList.contains('active')) ? 1 : -1;
        root.dataset.travel = direction === 1 ? 'forward' : 'back';
        if (!document.startViewTransition) return;
        // Preserve the app's own navigation handler and every native command it owns.
        event.preventDefault();
        event.stopImmediatePropagation();
        viewTransition?.skipTransition();
        const transition = document.startViewTransition(() => button.onclick.call(button, event));
        viewTransition = transition;
        transition.finished.catch(() => {}).finally(() => { if (viewTransition === transition) viewTransition = null; });
    }, true);
    const panelObserver = new MutationObserver(records => {
        for (const { target, oldValue } of records) {
            if (!target.classList.contains('active') || (oldValue || '').split(' ').includes('active')) continue;
            if (!document.startViewTransition) animate(target, [
                { transform: `translateX(${direction * 32}px)`, clipPath: direction > 0 ? 'inset(0 0 0 8%)' : 'inset(0 8% 0 0)' },
                { transform: 'translateX(0)', clipPath: 'inset(0)' }
            ], { duration: 350 });
            if (target.matches('.tab-content')) {
                const nav = document.querySelector(`.nav-item[data-tab="${target.id}"]`);
                document.getElementById('studio-page-name').textContent = nav?.textContent.trim() || 'Workspace';
                document.querySelectorAll('.nav-item[data-tab]').forEach(item => item.setAttribute('aria-pressed', String(item === nav)));
                document.querySelector('main').scrollTop = 0;
                moveTrack();
            }
            const strip = target.parentElement.querySelector('.subtab-strip');
            if (strip) updateSegment(strip);
        }
    });
    document.querySelectorAll('.tab-content, .subtab-pane').forEach(panel => panelObserver.observe(panel, { attributes: true, attributeFilter: ['class'], attributeOldValue: true }));
    const grid = document.getElementById('catalog-grid');
    const gridObserver = new MutationObserver(records => {
        const added = records.flatMap(record => [...record.addedNodes]).filter(node => node.nodeType === 1 && node.matches('.catalog-card'));
        added.forEach((card, index) => animate(card, [
            { transform: 'perspective(700px) rotateX(7deg) scale(.94)', opacity: .3 },
            { transform: 'perspective(700px) rotateX(-1deg) scale(1.008)', opacity: 1, offset: .7 },
            { transform: 'perspective(700px) rotateX(0deg) scale(1)', opacity: 1 }
        ], { delay: Math.min(index % 6, 5) * 22, duration: 420 }));
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
