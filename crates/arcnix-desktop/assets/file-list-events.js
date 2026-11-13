// Carry native click counts/modifiers into finite Rust-owned browser gestures.
(() => {
    let sequence = 0;
    let lastPane = null;
    document.addEventListener("pointerdown", () => document.documentElement.classList.remove("keyboard-navigation"), true);
    document.addEventListener("keydown", () => document.documentElement.classList.add("keyboard-navigation"), true);
    const additive = event => /Mac|iPhone|iPad/.test(navigator.platform) ? event.metaKey : event.ctrlKey;
    function send(pane, kind, row, event = {}) {
        const bridge = pane?.querySelector(".file-gesture-input");
        if (!bridge || pane.closest("[inert],[hidden]")) return;
        bridge.value = JSON.stringify({kind, path: row?.getAttribute("aria-label") || "", toggle: !!additive(event), range: !!event.shiftKey, sequence: ++sequence});
        bridge.dispatchEvent(new Event("input", {bubbles: true}));
    }
    document.addEventListener("click", event => {
        if (!(event.target instanceof Element) || event.detail === 0) return;
        lastPane = event.target.closest(".file-pane") || lastPane;
        const row = event.target.closest(".file-list .file-row");
        if (!row || event.target.closest(".file-expander")) return;
        const name = row.querySelector(".file-name");
        if (!name || name.disabled) return;
        event.preventDefault(); event.stopImmediatePropagation();
        name.focus({preventScroll: true});
        if (event.detail === 1) send(row.closest(".file-pane"), "select", row, event);
        else if (event.detail === 2 && !additive(event) && !event.shiftKey) send(row.closest(".file-pane"), "activate", row);
    }, true);
    document.addEventListener("keydown", event => {
        if (!(event.target instanceof Element) || event.isComposing) return;
        const pane = event.target.closest(".file-pane") || (additive(event) && event.key.toLowerCase() === "l" && lastPane?.isConnected ? lastPane : null);
        if (!pane || pane.closest("[inert],[hidden]")) return;
        if (additive(event) && event.key.toLowerCase() === "l") {
            event.preventDefault(); event.stopImmediatePropagation();
            const field = pane.querySelector(".path-input:not(.hidden)");
            if (field) { field.focus(); field.select(); }
            else pane.querySelector(".path-edit")?.click();
            return;
        }
        if (event.target.matches("input,textarea") || event.altKey) return;
        const row = event.target.closest(".file-row");
        if (!row) return;
        let handled = true;
        if (additive(event) && event.key.toLowerCase() === "a") send(pane, "all", row);
        else if (event.key === " ") send(pane, "select", row, event);
        else if (pane.classList.contains("icons-view") && ["ArrowLeft", "ArrowRight"].includes(event.key)) {
            const rows = [...pane.querySelectorAll(".file-row")];
            const target = rows[Math.max(0, Math.min(rows.length - 1, rows.indexOf(row) + (event.key === "ArrowLeft" ? -1 : 1)))];
            if (target) { target.querySelector(".file-name").focus(); if (!additive(event) || event.shiftKey) send(pane, "select", target, event); }
        }
        else if (event.key === "ArrowRight") send(pane, "expand", row);
        else if (event.key === "ArrowLeft") {
            if (row.querySelector(".file-expander")?.textContent === "⌄") send(pane, "collapse", row);
            else {
                const rows = [...pane.querySelectorAll(".file-row")];
                const depth = parseFloat(getComputedStyle(row).paddingLeft);
                const parent = rows.slice(0, rows.indexOf(row)).reverse().find(item => parseFloat(getComputedStyle(item).paddingLeft) < depth);
                if (parent) { parent.querySelector(".file-name")?.focus(); send(pane, "select", parent); }
            }
        } else if (["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
            const rows = [...pane.querySelectorAll(".file-row")].filter(item => !item.querySelector(".file-name").disabled);
            const index = rows.indexOf(row);
            const columns = pane.classList.contains("icons-view") ? Math.max(1, getComputedStyle(pane.querySelector(".file-list")).gridTemplateColumns.split(/\s+/).length) : 1;
            const next = event.key === "Home" ? 0 : event.key === "End" ? rows.length - 1 : index + (event.key === "ArrowUp" ? -columns : columns);
            const target = rows[Math.max(0, Math.min(rows.length - 1, next))];
            if (target) { target.querySelector(".file-name").focus(); if (!additive(event) || event.shiftKey) send(pane, "select", target, event); }
        } else handled = false;
        if (handled) { event.preventDefault(); event.stopImmediatePropagation(); }
    }, true);
    const focused = new WeakSet();
    const currentPaths = new WeakMap();
    new MutationObserver(() => {
        for (const field of document.querySelectorAll(".path-input")) {
            if (field.classList.contains("hidden")) focused.delete(field);
            else if (!focused.has(field) && field.getClientRects().length) { focused.add(field); field.focus(); field.select(); }
        }
        for (const bar of document.querySelectorAll(".breadcrumbs")) {
            const current = bar.querySelector(".current")?.getAttribute("aria-label");
            if (current && currentPaths.get(bar) !== current) { currentPaths.set(bar, current); bar.scrollLeft = bar.scrollWidth; }
        }
    }).observe(document, {subtree: true, childList: true, attributes: true, attributeFilter: ["class"]});
    document.addEventListener("wheel", event => {
        const bar = event.target instanceof Element && event.target.closest(".breadcrumbs");
        if (bar && !event.ctrlKey && bar.scrollWidth > bar.clientWidth) { event.preventDefault(); bar.scrollLeft += event.deltaX || event.deltaY; }
    }, {passive:false});
})();
