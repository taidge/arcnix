"""Verify the app rail and server-scoped UI in a real WebView and loopback SSH.

Uses isolated configs and the project's SSH fixture; never contacts a real server.
Optional dependencies live in artifacts/python (playwright, paramiko).
"""
from pathlib import Path
import argparse
import json
import os
import socket
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "artifacts/python"))
from playwright.sync_api import sync_playwright, expect
from PIL import Image


def wait_until(probe, process, timeout=20):
    deadline = time.monotonic() + timeout
    while True:
        try:
            return probe()
        except (OSError, ValueError):
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Fixture or desktop failed to start")
            time.sleep(0.1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path)
    args = parser.parse_args()
    if args.exe:
        executable = args.exe.resolve()
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT
        ))
        executable = Path(metadata["target_directory"]) / "debug/arcnix.exe"
    output = ROOT / "artifacts/navigation-redesign"
    output.mkdir(parents=True, exist_ok=True)
    ready = output / ("fixture-" + str(time.time_ns()) + ".json")
    log = (output / "smoke.log").open("w", encoding="utf-8")
    fixture = subprocess.Popen([sys.executable, str(ROOT / "tools/sftp_fixture.py"), "--ready", str(ready)], stdout=log, stderr=log)
    process = None
    checks = []

    def launch(playwright, config):
        nonlocal process
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        endpoint = f"http://127.0.0.1:{port}"
        env = os.environ.copy()
        env["WEBVIEW2_USER_DATA_FOLDER"] = str(output / ("webview-" + str(time.time_ns())))
        env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
        process = subprocess.Popen([str(executable), "--config", str(config)], cwd=ROOT, env=env, stdout=log, stderr=log)
        def probe():
            with urllib.request.urlopen(endpoint + "/json/version", timeout=1) as response:
                return response.read()
        wait_until(probe, process)
        browser = playwright.chromium.connect_over_cdp(endpoint)
        page = browser.contexts[0].pages[0]
        page.set_default_timeout(10000)
        page.set_viewport_size(dict(width=1440, height=980))
        expect(page.locator(".server-landing")).to_be_visible()
        return browser, page

    def stop(browser):
        nonlocal process
        browser.close()
        process.terminate()
        process.wait(timeout=10)
        process = None

    try:
        peer = wait_until(lambda: json.loads(ready.read_text()), fixture)
        host, port = peer["address"].rsplit(":", 1)
        assert host == "127.0.0.1"
        local = output / "files"
        local.mkdir(exist_ok=True)
        (local / "navigation.txt").write_text("Navigation regression fixture\n", encoding="utf-8")
        profile = dict(id="navigation-test", name="Development", host=host, port=int(port), username="arcnix", auth="PrivateKey", private_key=peer["client_key_ed25519"], initial_remote_path="/")
        offline = dict(profile, id="offline-test", name="Staging", port=9)
        config = output / "smoke-config.json"
        config.write_text(json.dumps(dict(version=1, config=dict(servers=[profile, offline], groups=[], trusted_hosts=[dict(host=host, port=int(port), fingerprint=peer["host_key_rsa_fingerprint"])], local_directory=str(local)))), encoding="utf-8")
        with sync_playwright() as playwright:
            browser, page = launch(playwright, config)
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            expect(page.locator(".navigation-rail button")).to_have_count(2)
            expect(page.locator(".rail-notifications")).to_have_count(0)
            expect(page.locator(".home-page")).to_have_count(0)
            page.locator(".rail-logo").click()
            expect(page.locator(".server-landing")).to_be_visible()
            expect(page.locator(".module-nav")).to_have_count(0)
            expect(page.locator(".files-module")).to_have_count(0)
            page.locator(".rail-servers").click()
            expect(page.get_by_text("Choose a server", exact=True)).to_be_visible()
            expect(page.locator(".module-nav")).to_have_count(0)
            page.screenshot(path=str(output / "servers-unselected-dark.png"))
            page.locator(".host-select").first.click()
            expect(page.locator(".server-overview")).to_contain_text("Development")
            expect(page.locator(".module-nav")).to_have_count(0)
            expect(page.locator(".terminal-module")).to_be_hidden()
            page.screenshot(path=str(output / "server-disconnected-dark.png"))
            checks.append("Logo is inert, notifications are absent, and unselected/disconnected server states never expose server modules")

            page.locator(".overview-connect").click()
            page.get_by_role("dialog").get_by_role("button", name="Connect", exact=True).click()
            expect(page.locator(".connection-state")).to_have_text("SSH connected")
            expect(page.locator(".module-nav .module-tab")).to_have_count(5)
            expect(page.locator(".files-module")).to_be_visible()
            page.screenshot(path=str(output / "server-connected-dark.png"))
            page.get_by_role("button", name="Terminal", exact=True).click()
            expect(page.locator(".terminal-status")).to_have_text("Interactive SSH")
            reader = page.locator(".terminal-surface .xterm-accessibility-tree")
            expect(reader).to_contain_text("Arcnix terminal fixture")
            page.evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))")
            page.locator(".xterm-helper-textarea").focus()
            page.keyboard.type("echo navigation-session-preserved", delay=10)
            page.keyboard.press("Enter")
            expect(reader).to_contain_text("navigation-session-preserved")
            page.locator(".rail-logo").click()
            expect(page.locator(".terminal-module")).to_be_visible()
            expect(reader).to_contain_text("navigation-session-preserved")
            checks.append("Clicking the logo preserves the connected workspace and terminal session")

            dark_icon = page.locator(".brand-icon").evaluate("el=>getComputedStyle(el).backgroundImage")
            page.locator(".settings-link").click()
            expect(page.get_by_role("button", name="English", exact=True)).to_be_visible()
            page.locator(".context-sidebar").get_by_role("button", name="Appearance", exact=True).click()
            page.get_by_role("button", name="Light", exact=True).click()
            expect(page.locator(".app")).to_have_class("app theme-light")
            light_icon = page.locator(".brand-icon").evaluate("el=>getComputedStyle(el).backgroundImage")
            assert dark_icon != light_icon
            page.screenshot(path=str(output / "settings-light.png"))
            page.locator(".rail-servers").click()
            expect(page.locator(".connection-state")).to_have_text("SSH connected")
            expect(page.locator(".terminal-module")).to_be_visible()
            expect(reader).to_contain_text("navigation-session-preserved")
            assert page.locator(".server-modules").count() == 0
            page.get_by_role("button", name="Files", exact=True).click()
            page.locator(".local-slot").get_by_role("button", name="navigation.txt", exact=True).click()
            page.locator(".local-slot").get_by_role("button", name="Upload", exact=True).click()
            expect(page.locator(".queue-row").last).to_contain_text("Completed")
            page.screenshot(path=str(output / "server-connected-light.png"))
            checks.append("Settings switch themes and SVG variants while SSH, terminal history and SFTP remain functional")

            page.locator(".host-select").nth(1).click()
            expect(page.locator(".module-nav")).to_have_count(0)
            expect(page.locator(".files-module")).to_have_count(0)
            expect(page.locator(".server-overview")).to_contain_text("Staging")
            page.screenshot(path=str(output / "server-disconnected-light.png"))
            page.set_viewport_size(dict(width=900, height=700))
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            page.screenshot(path=str(output / "servers-light-narrow.png"))
            page.set_viewport_size(dict(width=1440, height=980))
            page.locator(".settings-link").click()
            page.locator(".context-sidebar").get_by_role("button", name="General", exact=True).click()
            page.get_by_role("button", name="简体中文", exact=True).click()
            expect(page.locator(".context-title")).to_have_text("设置")
            page.locator(".rail-logo").click()
            expect(page.locator(".context-title")).to_have_text("设置")
            expect(page.locator(".rail-notifications")).to_have_count(0)
            page.screenshot(path=str(output / "settings-chinese-light.png"))
            assert not errors, errors
            checks.append("Switching servers clears scoped modules; narrow layout and live Chinese localization work")
            stop(browser)

            empty = output / "empty-config.json"
            empty.write_text(json.dumps(dict(version=1, config=dict(servers=[], groups=[], trusted_hosts=[]))), encoding="utf-8")
            browser, page = launch(playwright, empty)
            expect(page.get_by_text("Choose a server", exact=True)).to_be_visible()
            page.locator(".rail-servers").click()
            expect(page.locator(".module-nav")).to_have_count(0)
            page.locator(".sidebar-heading").get_by_role("button", name="New server", exact=True).click()
            expect(page.get_by_role("dialog")).to_be_visible()
            checks.append("First run has useful empty states and a working create-server action")
            stop(browser)
        for name in ("arcnix.svg", "arcnix-dark.svg", "arcnix-light.svg"):
            import cairosvg
            from io import BytesIO
            image = Image.open(BytesIO(cairosvg.svg2png(url=str(ROOT / "crates/arcnix-desktop/assets" / name)))).convert("RGBA")
            assert image.size == (512, 512)
            assert all(image.getpixel(corner)[3] == 0 for corner in ((0, 0), (511, 0), (0, 511), (511, 511)))
            assert image.getchannel("A").getbbox() is not None
        checks.append("All three 512x512 SVG variants render visible artwork with transparent corners")
        result = dict(passed=len(checks), checks=checks)
        (output / "results.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        print(json.dumps(result, indent=2), flush=True)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        fixture.terminate()
        fixture.wait(timeout=10)
        log.close()


if __name__ == "__main__":
    main()
