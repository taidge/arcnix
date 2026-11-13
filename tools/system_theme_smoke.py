"""Verify OS-following themes in the real WebView using isolated configs.

CDP media emulation changes only the test WebView, never Windows settings.
"""
from pathlib import Path
import argparse
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "artifacts/python"))
from playwright.sync_api import sync_playwright, expect


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path)
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT
    ))
    executable = args.exe.resolve() if args.exe else Path(metadata["target_directory"]) / "debug/arcnix.exe"
    output = ROOT / "artifacts/system-theme"
    output.mkdir(parents=True, exist_ok=True)
    config = output / "config.json"
    config.write_text(json.dumps(dict(version=1, config=dict(groups=[], servers=[], trusted_hosts=[]))), encoding="utf-8")
    process = None
    log = (output / "runtime.log").open("w", encoding="utf-8")
    checks = []

    def launch(playwright, path=config):
        nonlocal process
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        endpoint = f"http://127.0.0.1:{port}"
        env = os.environ.copy()
        env["WEBVIEW2_USER_DATA_FOLDER"] = str(output / ("webview-" + str(time.time_ns())))
        env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
        process = subprocess.Popen([str(executable), "--config", str(path)], cwd=ROOT, env=env, stdout=log, stderr=log)
        deadline = time.monotonic() + 20
        while True:
            try:
                urllib.request.urlopen(endpoint + "/json/version", timeout=1).close()
                break
            except OSError:
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Desktop failed to start")
                time.sleep(0.1)
        browser = playwright.chromium.connect_over_cdp(endpoint)
        page = browser.contexts[0].pages[0]
        page.set_viewport_size(dict(width=1440, height=980))
        page.set_default_timeout(8000)
        expect(page.locator(".system-theme-input")).to_have_count(1)
        page.locator(".settings-link").click()
        page.locator(".context-sidebar button").nth(1).click()
        return browser, page

    def stop(browser):
        nonlocal process
        browser.close()
        process.terminate()
        process.wait(timeout=10)
        process = None

    def theme(page, name):
        expect(page.locator(".app")).to_have_class(f"app theme-{name}")

    try:
        with sync_playwright() as playwright:
            browser, page = launch(playwright)
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            follow = page.get_by_role("button", name="Follow system", exact=True)
            expect(follow).to_have_class(re.compile(r"choice active"))
            before = config.read_bytes()
            page.emulate_media(color_scheme="dark")
            theme(page, "dark")
            dark_icon = page.locator(".brand-icon").evaluate("el=>getComputedStyle(el).backgroundImage")
            page.screenshot(path=str(output / "follow-system-dark.png"))
            page.emulate_media(color_scheme="light")
            theme(page, "light")
            light_icon = page.locator(".brand-icon").evaluate("el=>getComputedStyle(el).backgroundImage")
            assert dark_icon != light_icon
            expect(follow).to_have_class(re.compile(r"choice active"))
            assert config.read_bytes() == before, "OS changes must not overwrite the saved preference"
            page.screenshot(path=str(output / "follow-system-light.png"))
            checks.append("Fresh configs default to Follow system; live OS changes switch UI and SVG colors without saving")

            page.get_by_role("button", name="Dark", exact=True).click()
            theme(page, "dark")
            page.emulate_media(color_scheme="dark")
            page.emulate_media(color_scheme="light")
            theme(page, "dark")
            assert json.loads(config.read_text())["config"]["settings"]["theme"] == "Dark"
            page.get_by_role("button", name="Light", exact=True).click()
            page.emulate_media(color_scheme="dark")
            theme(page, "light")
            assert json.loads(config.read_text())["config"]["settings"]["theme"] == "Light"
            follow.click()
            theme(page, "dark")
            assert json.loads(config.read_text())["config"]["settings"]["theme"] == "System"
            checks.append("Explicit Dark/Light override the OS; switching back to System uses the latest preference")

            page.locator(".context-sidebar button").nth(0).click()
            page.get_by_role("button", name="简体中文", exact=True).click()
            page.locator(".context-sidebar").get_by_role("button", name="外观", exact=True).click()
            expect(page.get_by_role("button", name="跟随系统", exact=True)).to_have_class(re.compile(r"choice active"))
            page.emulate_media(color_scheme="light")
            theme(page, "light")
            page.screenshot(path=str(output / "follow-system-chinese.png"))
            assert not errors, errors
            stop(browser)

            browser, page = launch(playwright)
            page.emulate_media(color_scheme="dark")
            theme(page, "dark")
            expect(page.get_by_role("button", name="跟随系统", exact=True)).to_have_class(re.compile(r"choice active"))
            page.emulate_media(color_scheme="light")
            theme(page, "light")
            stop(browser)
            checks.append("System preference persists across restart and language remounts keep listening to OS changes")

            legacy = output / "legacy-config.json"
            legacy.write_text(json.dumps(dict(version=1, config=dict(groups=[], servers=[], trusted_hosts=[], settings=dict(language="English", theme="Dark", show_hidden_files=True, resource_interval_seconds=3)))), encoding="utf-8")
            browser, page = launch(playwright, legacy)
            page.emulate_media(color_scheme="light")
            theme(page, "dark")
            expect(page.get_by_role("button", name="Dark", exact=True)).to_have_class(re.compile(r"choice active"))
            stop(browser)
            checks.append("Previously saved explicit themes are preserved")
        result = dict(passed=len(checks), checks=checks)
        (output / "results.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        print(json.dumps(result, indent=2), flush=True)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
        log.close()


if __name__ == "__main__":
    main()
