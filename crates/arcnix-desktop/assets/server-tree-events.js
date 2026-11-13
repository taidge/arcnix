// Route native context-menu gestures through the same Rust-owned More buttons.
(() => {
    let pointerMenu = null;
    let keyboardMenu = null;
    let scheduled = false;
    let draggedGroup = null;
    let dropPosition = null;
    let dragSequence = 0;
    function clearDrag() {
        document.querySelectorAll('.dragging-group,.drop-before,.drop-after').forEach(row => row.classList.remove('dragging-group','drop-before','drop-after'));
        draggedGroup = null; dropPosition = null;
    }
    function groupId(row) { return row?.querySelector('.tree-group-id')?.textContent || ''; }
    document.addEventListener('dragstart', event => {
        const button = event.target instanceof Element && event.target.closest('.group-button');
        const tree = button?.closest('.server-tree');
        if (!tree || tree.classList.contains('reorder-disabled') || tree.closest('[inert]')) return;
        const row = button.closest('.tree-folder-row');
        draggedGroup = groupId(row);
        if (!draggedGroup) return;
        row.classList.add('dragging-group');
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData('application/x-arcnix-group',draggedGroup);
        // Drag just the row, so an expanded folder does not cover the drop target.
        event.dataTransfer.setDragImage(row,20,row.offsetHeight / 2);
    },true);
    document.addEventListener('dragover', event => {
        if (!draggedGroup || !(event.target instanceof Element)) return;
        const tree = event.target.closest('.server-tree');
        if (!tree || tree.classList.contains('reorder-disabled') || tree.closest('[inert]')) return;
        let row = event.target.closest('.tree-folder')?.querySelector(':scope > .tree-folder-row');
        if (!row) {
            const rows = tree.querySelectorAll('.tree-folder-row');
            const last = rows[rows.length - 1];
            if (last && event.clientY > last.getBoundingClientRect().bottom) row = last;
        }
        document.querySelectorAll('.drop-before,.drop-after').forEach(item=>item.classList.remove('drop-before','drop-after'));
        dropPosition = null;
        if (!row || groupId(row)===draggedGroup) return;
        const box = row.getBoundingClientRect();
        const after = event.clientY > box.top + box.height / 2;
        row.classList.add(after ? 'drop-after' : 'drop-before');
        dropPosition = {target:groupId(row),after};
        event.preventDefault();event.dataTransfer.dropEffect='move';
        const bounds=tree.getBoundingClientRect();
        if(event.clientY < bounds.top+28) tree.scrollTop-=12;
        else if(event.clientY > bounds.bottom-28) tree.scrollTop+=12;
    },true);
    document.addEventListener('drop', event => {
        if (!draggedGroup || !dropPosition || !(event.target instanceof Element)) {clearDrag();return;}
        const sidebar=event.target.closest('.server-explorer');
        const bridge=sidebar?.querySelector('.server-tree-input');
        if(bridge && !sidebar.closest('[inert]') && !sidebar.querySelector('.reorder-disabled')) {
            event.preventDefault();event.stopImmediatePropagation();
            bridge.value=JSON.stringify({group:draggedGroup,...dropPosition,sequence:++dragSequence});
            bridge.dispatchEvent(new Event('input',{bubbles:true}));
        }
        clearDrag();
    },true);
    document.addEventListener('dragend',clearDrag,true);
    const sidebarFor = target => target instanceof Element ? target.closest('.server-explorer') : null;
    function openMenu(target, point) {
        const sidebar = sidebarFor(target);
        if (!sidebar || sidebar.closest('[inert]') || target.closest('input,textarea,.context-popover')) return false;
        const row = target.closest('.host-row,.group-row');
        const more = row?.querySelector('.host-more,.group-more') || sidebar.querySelector('.tree-more');
        if (!more || more.disabled) return false;
        const owner = row || sidebar.querySelector('.tree-root-row');
        pointerMenu = point ? {owner, ...point} : null;
        keyboardMenu = point ? null : owner;
        more.focus({preventScroll: true});
        if (!owner.classList.contains('menu-open')) more.click();
        schedule();
        return true;
    }
    document.addEventListener('contextmenu', event => {
        const target = event.target instanceof Element && event.target.matches('.menu-dismiss')
            ? document.elementsFromPoint(event.clientX, event.clientY).find(item => sidebarFor(item)) : event.target;
        if (openMenu(target, {x: event.clientX, y: event.clientY})) {
            event.preventDefault();
            event.stopImmediatePropagation();
        }
    }, true);
    document.addEventListener('pointerdown', event => {
        if (event.button === 0 && !(event.target instanceof Element && event.target.closest('.context-popover'))) {
            pointerMenu = null;
            schedule();
        }
    }, true);
    document.addEventListener('keydown', event => {
        const target = event.target;
        const sidebar = sidebarFor(target);
        if (!sidebar || sidebar.closest('[inert]')) return;
        if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
            if (openMenu(target, null)) { event.preventDefault(); event.stopImmediatePropagation(); }
            return;
        }
        const menu = target.closest('.context-popover');
        if (menu && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
            const items = [...menu.querySelectorAll('button:not(:disabled)')];
            const index = items.indexOf(target.closest('button'));
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowUp' ? -1 : 1) + items.length) % items.length;
            items[next]?.focus(); event.preventDefault(); event.stopImmediatePropagation(); return;
        }
        if (target.closest('input,textarea,.context-popover') || event.altKey || event.ctrlKey || event.metaKey) return;
        const button = target.closest('.group-button,.host-select');
        if (!button) return;
        const folder = button.closest('.group-row');
        if (folder && (event.key === 'ArrowRight' || event.key === 'ArrowLeft')) {
            if ((event.key === 'ArrowRight') !== folder.classList.contains('expanded')) button.click();
        } else if (event.key === 'ArrowLeft' && button.closest('.tree-children')) {
            button.closest('.tree-folder')?.querySelector('.group-button')?.focus();
        } else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
            const buttons = [...sidebar.querySelectorAll('.group-button,.host-select')].filter(item => !item.disabled && item.getClientRects().length);
            const index = buttons.indexOf(button);
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : Math.max(0, Math.min(buttons.length - 1, index + (event.key === 'ArrowUp' ? -1 : 1)));
            buttons[next]?.focus();
        } else return;
        event.preventDefault(); event.stopImmediatePropagation();
    }, true);
    function placeMenu() {
        scheduled = false;
        for (const button of document.querySelectorAll('.server-tree .group-button')) {
            const draggable=!button.closest('.reorder-disabled,[inert]');
            if(button.draggable!==draggable) button.draggable=draggable;
        }
        for (const menu of document.querySelectorAll('.server-explorer .context-popover')) {
            if (keyboardMenu?.contains(menu)) {
                menu.querySelector('button:not(:disabled)')?.focus(); keyboardMenu = null;
            }
            if (pointerMenu?.owner.isConnected && pointerMenu.owner.contains(menu)) {
                if (!menu.classList.contains('pointer-menu')) menu.classList.add('pointer-menu');
                const box = menu.getBoundingClientRect();
                menu.style.left = `${Math.max(8, Math.min(pointerMenu.x, innerWidth - box.width - 8))}px`;
                menu.style.top = `${Math.max(8, Math.min(pointerMenu.y, innerHeight - box.height - 8))}px`;
            } else if (menu.classList.contains('pointer-menu')) {
                menu.classList.remove('pointer-menu');
                menu.style.removeProperty('left'); menu.style.removeProperty('top');
            }
        }
    }
    function schedule() { if (!scheduled) { scheduled = true; requestAnimationFrame(placeMenu); } }
    new MutationObserver(schedule).observe(document, {subtree: true, childList: true, attributes: true, attributeFilter: ['class']});
    // A menu's target can scroll away; dismiss via the normal Rust-owned scrim.
    document.addEventListener('scroll', event => {
        if (event.target instanceof Element && event.target.matches('.server-tree')) {
            document.querySelector('.menu-dismiss:not(.hidden)')?.click(); pointerMenu = null;
        }
    }, true);
})();
