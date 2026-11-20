/* Read-only preview using the checked-in catalog; no simulated native commands. */
export async function startPreview() {
    document.body.classList.add('browser-preview');
    const status = document.getElementById('status-text');
    status.textContent = 'Browser preview · game actions require the desktop app';
    const notice = () => {
        const container = document.getElementById('toast-container');
        container.replaceChildren();
        const toast = document.createElement('div');
        toast.className = 'toast preview-notice';
        toast.setAttribute('role', 'status');
        toast.textContent = 'Open the desktop app to use game actions. This preview is read-only.';
        container.append(toast);
        setTimeout(() => toast.remove(), 4500);
    };
    document.querySelectorAll('.nav-item[data-tab]').forEach(nav => {
        nav.onclick = () => {
            document.querySelectorAll('.nav-item[data-tab], .tab-content').forEach(el => el.classList.remove('active'));
            nav.classList.add('active');
            document.getElementById(nav.dataset.tab).classList.add('active');
        };
    });
    document.querySelectorAll('.subtab-btn[data-subtab]').forEach(button => {
        button.onclick = () => {
            const parent = button.closest('.tab-content');
            parent.querySelectorAll('.subtab-btn, .subtab-pane').forEach(el => el.classList.remove('active'));
            button.classList.add('active');
            document.getElementById(button.dataset.subtab)?.classList.add('active');
        };
    });
    document.getElementById('settings-btn').onclick = () => document.getElementById('settings-modal').classList.add('active');
    document.querySelectorAll('.close-modal-btn').forEach(button => {
        button.onclick = () => button.closest('.modal-overlay')?.classList.remove('active');
    });
    document.addEventListener('keydown', event => {
        if (event.key === 'Escape') document.querySelector('.modal-overlay.active')?.classList.remove('active');
    });
    const allowed = '.nav-item[data-tab], .subtab-btn, #settings-btn, .close-modal-btn, #studio-explore, .catalog-page-btn, #catalog-select-page-btn, #catalog-clear-sel-btn, #catalog-search-clear';
    window.addEventListener('click', event => {
        const action = event.target.closest('button, [role="button"], .nav-item');
        if (!action || action.matches(allowed) || action.matches('.catalog-card')) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        notice();
    }, true);
    window.addEventListener('keydown', event => {
        if (event.target.id === 'version-btn' && ['Enter', ' '].includes(event.key)) {
            event.preventDefault();
            event.stopImmediatePropagation();
            notice();
        }
    }, true);
    try {
        const response = await fetch('../resources/items.json');
        if (!response.ok) throw new Error('Serve the repository root to preview its item catalog.');
        const data = await response.json();
        items = normalizeItems(data.items || data).map(item => ({
            ...item,
            image_url: item.image_url || (item.thumbnail_url ? `https://api.velocityrl.tech/thumbnails/${encodeURIComponent(item.thumbnail_url)}` : '')
        }));
        await setAppLanguage('en');
        initCatalogGrid();
        renderCatalogGridPage();
        updateCatalogSelectionUi();
    } catch (error) {
        document.getElementById('catalog-grid').textContent = `Catalog unavailable: ${error.message}`;
    } finally {
        releaseAppLoading();
    }
}
