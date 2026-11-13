"""Exercise application preferences in an isolated real desktop WebView."""
from pathlib import Path
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.request

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"artifacts/python"))
from playwright.sync_api import sync_playwright,expect


def main():
    output=ROOT/"artifacts/settings-ui"
    output.mkdir(parents=True,exist_ok=True)
    local=output/"files"
    local.mkdir(exist_ok=True)
    (local/"visible.txt").write_text("Fixture",encoding="utf-8")
    (local/".hidden-test").write_text("Fixture",encoding="utf-8")
    config=output/"config.json"
    config.write_text(json.dumps(dict(version=1,config=dict(groups=[],servers=[],trusted_hosts=[],local_directory=str(local)))),encoding="utf-8")
    process=None
    checks=[]
    log=(output/"desktop.log").open("w",encoding="utf-8")

    def launch(playwright):
        nonlocal process
        with socket.socket() as sock:
            sock.bind(("127.0.0.1",0));port=sock.getsockname()[1]
        endpoint=f"http://127.0.0.1:{port}"
        env=os.environ.copy()
        env["WEBVIEW2_USER_DATA_FOLDER"]=str(output/("webview-"+str(time.time_ns())))
        env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"]=f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
        process=subprocess.Popen([str(ROOT/"target/debug/arcnix.exe"),"--config",str(config)],cwd=ROOT,env=env,stdout=log,stderr=log)
        deadline=time.monotonic()+20
        while True:
            try:
                urllib.request.urlopen(endpoint+"/json/version",timeout=1).close();break
            except OSError:
                if process.poll() is not None or time.monotonic()>deadline:
                    raise RuntimeError("Desktop unavailable: "+(output/"desktop.log").read_text(encoding="utf-8"))
                time.sleep(.1)
        browser=playwright.chromium.connect_over_cdp(endpoint)
        page=browser.contexts[0].pages[0]
        page.set_default_timeout(8000)
        page.set_viewport_size(dict(width=1440,height=980))
        page.wait_for_selector(".settings-link")
        return browser,page

    try:
        with sync_playwright() as playwright:
            browser,page=launch(playwright)
            expect(page.get_by_role("button",name="Files",exact=True)).to_be_visible()
            expect(page.locator(".local-slot .file-row")).to_have_count(2)
            page.locator(".settings-link").click()
            expect(page.get_by_text("Application settings",exact=True)).to_be_visible()
            page.screenshot(path=str(output/"settings-dark.png"))
            page.get_by_role("button",name="Light",exact=True).click()
            expect(page.locator(".app")).to_have_class("app theme-light")
            page.get_by_role("button",name="Hide hidden files",exact=True).click()
            page.get_by_role("button",name="Every 10 seconds",exact=True).click()
            page.screenshot(path=str(output/"settings-light.png"))
            page.get_by_role("button",name="Files",exact=True).click()
            expect(page.locator(".local-slot .file-row")).to_have_count(1)
            page.locator(".settings-link").click()
            page.locator(".settings-section").first.locator(".choice").nth(1).click()
            expect(page.get_by_role("button",name="文件管理",exact=True)).to_be_visible()
            page.screenshot(path=str(output/"settings-chinese.png"))
            stored=json.loads(config.read_text(encoding="utf-8"))["config"]["settings"]
            assert stored==dict(language="SimplifiedChinese",theme="Light",show_hidden_files=False,resource_interval_seconds=10),stored
            checks.append("English is default; language/theme update live and file visibility changes immediately")
            browser.close()
            if process.poll() is None:
                process.terminate();process.wait(timeout=5)
            browser,page=launch(playwright)
            expect(page.get_by_role("button",name="文件管理",exact=True)).to_be_visible()
            expect(page.locator(".app")).to_have_class("app theme-light")
            expect(page.locator(".local-slot .file-row")).to_have_count(1)
            page.locator(".settings-link").click()
            page.locator(".settings-section").first.locator(".choice").first.click()
            expect(page.get_by_role("button",name="Files",exact=True)).to_be_visible()
            expect(page.get_by_role("button",name="Every 10 seconds",exact=True)).to_have_class(re.compile(r"\bchoice active\b"))
            page.get_by_role("button",name="Resources",exact=True).click()
            expect(page.locator(".resources-page")).to_contain_text("10")
            checks.append("A fresh process restores language/theme/hidden files/monitoring interval")

            page.get_by_role("button",name="New server",exact=True).click()
            dialog=page.get_by_role("dialog",name="New server",exact=True)
            dialog.get_by_role("button",name="Private key",exact=True).click()
            expect(dialog.get_by_role("button",name="Browse…",exact=True)).to_be_visible()
            if "--picker" in sys.argv:
                old=output/"previous key"
                selected=output/"mock key 中文"
                old.write_text("TEST FILE ONLY",encoding="utf-8")
                selected.write_text("TEST FILE ONLY",encoding="utf-8")
                field=dialog.get_by_role("textbox",name="Private key file",exact=True)
                field.fill(str(old))
                dialog.get_by_role("button",name="Browse…",exact=True).click()
                if dialog.locator(".form-error").count():
                    raise RuntimeError(dialog.locator(".form-error").inner_text())
                (output/"picker-ready.json").write_text(json.dumps(dict(pid=process.pid,path=str(selected),phase="select")),encoding="utf-8")
                print("Native picker ready for test file",flush=True)
                expect(field).to_have_value(str(selected),timeout=120000)
                dialog.get_by_role("button",name="Browse…",exact=True).click()
                (output/"picker-ready.json").write_text(json.dumps(dict(pid=process.pid,path=str(selected),phase="cancel")),encoding="utf-8")
                print("Native picker ready for cancellation",flush=True)
                marker=output/"picker-cancelled"
                deadline=time.monotonic()+60
                while not marker.exists():
                    if time.monotonic()>deadline:raise TimeoutError("Native cancellation test timed out")
                    time.sleep(.1)
                expect(field).to_have_value(str(selected))
                marker.unlink()
                checks.append("Native picker selects an extensionless Unicode test file and cancellation preserves the path")
            page.keyboard.press("Escape")
            expect(dialog).to_have_count(0)
            browser.close()
        print(json.dumps(dict(passed=len(checks),checks=checks),ensure_ascii=False),flush=True)
    finally:
        if process is not None and process.poll() is None:
            process.terminate();process.wait(timeout=5)
        log.close()


if __name__=="__main__":main()
