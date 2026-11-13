// The WebView's native preference follows the OS. Rust owns the resolved theme
// so every component, SVG variant and terminal uses the same theme classes.
(() => {
    const preference = matchMedia('(prefers-color-scheme: dark)');
    const initialized = new WeakSet();
    function publish(bridge) {
        bridge.value = preference.matches ? 'dark' : 'light';
        bridge.dispatchEvent(new Event('input', {bubbles: true}));
    }
    function initialize() {
        const bridge = document.querySelector('.system-theme-input');
        if (bridge && !initialized.has(bridge)) {
            initialized.add(bridge);
            publish(bridge);
        }
    }
    preference.addEventListener('change', () => {
        const bridge = document.querySelector('.system-theme-input');
        if (bridge) publish(bridge);
    });
    // Language changes remount the app; publish to each new bridge once.
    new MutationObserver(initialize).observe(document, {subtree: true, childList: true});
    initialize();
})();
