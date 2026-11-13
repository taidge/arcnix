"""Exercise the real desktop WebView2 with isolated fixtures over loopback CDP.

Optional dependency: python -m pip install --target artifacts/python playwright
No browser installation, real server, user configuration, or credentials needed.
"""
from pathlib import Path
import json
import os
import socket
import subprocess
import sys
import time
import urllib.request
import hashlib
import base64
import re

WORKSPACE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(WORKSPACE / "artifacts" / "python"))
from playwright.sync_api import sync_playwright, expect
from ui_locale import install, tr
install()


def main():
    package_checks = "--packages" in sys.argv
    output = WORKSPACE / "artifacts" / "ui-redesign"
    output.mkdir(parents=True, exist_ok=True)
    local = output / ("files-" + str(time.time_ns()))
    local.mkdir()
    (local / "Documents").mkdir()
    (local / "notes.txt").write_text("Isolated UI fixture\n", encoding="utf-8")
    (local / "中文文件.txt").write_text("界面测试\n", encoding="utf-8")
    profiles = [dict(id="development", name="开发服务器", group_id="dev", host="dev.example.invalid", port=22, username="deploy", auth="Password", private_key=None, initial_remote_path="/srv"),
                dict(id="production", name="生产服务器", group_id="prod", host="prod.example.invalid", port=22, username="admin", auth="PrivateKey", private_key="example-key.pem", initial_remote_path="/var/www")]
    config = output / "config.json"
    trusted_hosts = []
    if package_checks:
        # Test-owned OpenSSH Ed25519 key; no user's credentials or SSH client.
        import paramiko
        fixture = json.loads((WORKSPACE / "artifacts" / "fixture.json").read_text(encoding="utf-8"))
        host, fixture_port = fixture["address"].rsplit(":", 1)
        assert host == "127.0.0.1"
        transport = paramiko.Transport((host, int(fixture_port)))
        transport.start_client()
        fingerprint = "SHA256:" + base64.b64encode(hashlib.sha256(transport.get_remote_server_key().asbytes()).digest()).decode().rstrip("=")
        transport.close()
        if "--legacy-host-key" in sys.argv:
            # Previously saved under WinCNG, while OpenSSL now prefers Ed25519.
            assert fingerprint != fixture["host_key_rsa_fingerprint"]
            fingerprint = fixture["host_key_rsa_fingerprint"]
        profiles[0].update(host=host, port=int(fixture_port), username="arcnix", auth="PrivateKey", private_key=fixture["client_key_ed25519"], initial_remote_path="/")
        trusted_hosts.append(dict(host=host, port=int(fixture_port), fingerprint=fingerprint))
    config.write_text(json.dumps(dict(version=1, config=dict(groups=[dict(id="dev", name="开发环境"), dict(id="prod", name="生产环境")], servers=profiles, trusted_hosts=trusted_hosts, local_directory=str(local))), ensure_ascii=False), encoding="utf-8")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    endpoint = f"http://127.0.0.1:{port}"
    env = os.environ.copy()
    if "--isolated-crypto-runtime" in sys.argv:
        # Keep the GUI's OS runtime available, with no OpenSSL/Perl/SSH tools
        # discoverable through PATH. Only the desktop child gets this environment.
        windows = Path(env.get("SystemRoot", r"C:\Windows"))
        env["PATH"] = str(windows / "System32") + os.pathsep + str(windows)
        env.pop("OPENSSL_CONF", None)
        env.pop("OPENSSL_MODULES", None)
    env["WEBVIEW2_USER_DATA_FOLDER"] = str(output / ("webview-" + str(time.time_ns())))
    env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
    log = (output / "desktop.log").open("w", encoding="utf-8")
    process = subprocess.Popen([str(WORKSPACE / "target" / "debug" / "arcnix.exe"), "--config", str(config)], cwd=WORKSPACE, env=env, stdout=log, stderr=log)
    checks = []
    try:
        deadline = time.monotonic() + 20
        while True:
            if process.poll() is not None:
                raise RuntimeError("Desktop exited: " + (output / "desktop.log").read_text(encoding="utf-8"))
            try:
                with urllib.request.urlopen(endpoint + "/json/version", timeout=1) as response:
                    json.load(response)
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise RuntimeError("WebView2 CDP endpoint unavailable")
                time.sleep(.15)
        with sync_playwright() as playwright:
            browser = playwright.chromium.connect_over_cdp(endpoint)
            page = browser.contexts[0].pages[0]
            page.on("pageerror", lambda error: print("WEBVIEW ERROR:", error, flush=True))
            page.on("console", lambda message: print("CONSOLE:", message.type, message.text, flush=True) if message.type == "error" else None)
            page.set_default_timeout(8000)
            page.wait_for_selector(".host-row")
            def host_menu(index=0):
                row = page.locator(".host-row").nth(index)
                row.scroll_into_view_if_needed()
                row.locator(".host-more").focus()
                row.locator(".host-more").click()

            def host_action(label, index=0):
                host_menu(index)
                page.locator(".context-popover").get_by_role("button", name=label, exact=True).click()

            expect(page.locator(".file-row")).to_have_count(3)

            local_pane = page.locator(".local-slot")
            local_path = local_pane.locator(".breadcrumb.current")
            documents = local_pane.get_by_role("button", name="Documents", exact=True)
            expect(documents).to_have_text("Documents")
            documents.click()
            expect(local_path).to_have_attribute("aria-label", str(local))
            expect(documents.locator("xpath=..")).to_have_class(re.compile(r"row-selected"))
            page.wait_for_timeout(550)
            expect(local_path).to_have_attribute("aria-label", str(local))
            documents.dblclick()
            expect(local_path).to_have_attribute("aria-label", str(local / "Documents"))
            expect(local_pane.locator(".file-empty")).to_have_text("此文件夹为空")
            local_pane.get_by_role("button", name="本地上级目录", exact=True).click()
            expect(documents).to_be_visible()
            local_pane.get_by_role("button", name="notes.txt", exact=True).dblclick()
            expect(local_path).to_have_attribute("aria-label", str(local))
            expect(local_pane.get_by_role("button", name="notes.txt", exact=True).locator("xpath=..")).to_have_class(re.compile(r"row-selected"))
            documents.focus()
            documents.press("Enter")
            expect(local_path).to_have_attribute("aria-label", str(local / "Documents"))
            local_pane.get_by_role("button", name="本地上级目录", exact=True).click()
            expect(documents).to_be_visible()
            documents.locator("xpath=..").locator(".file-size").dblclick()
            expect(local_path).to_have_attribute("aria-label", str(local / "Documents"))
            local_pane.get_by_role("button", name="本地上级目录", exact=True).click()
            expect(documents).to_be_visible()
            documents.click()
            expect(local_path).to_have_attribute("aria-label", str(local))
            documents.click(modifiers=["Control"])
            expect(local_path).to_have_attribute("aria-label", str(local))
            page.screenshot(path=str(output / "folder-double-click.png"))
            checks.append("Folder rows omit selection dots, single-click replaces selection, double-click opens and Enter opens")
            expect(local_pane.locator(".selection-toggle")).to_have_count(0)
            note = local_pane.get_by_role("button", name="notes.txt", exact=True)
            chinese = local_pane.get_by_role("button", name="中文文件.txt", exact=True)
            documents.click()
            note.click()
            expect(local_pane.locator(".row-selected")).to_have_count(1)
            expect(note.locator("xpath=..")).to_have_class(re.compile("row-selected"))
            documents.click(modifiers=["Control"])
            expect(local_pane.locator(".row-selected")).to_have_count(2)
            documents.click(modifiers=["Control"])
            expect(local_pane.locator(".row-selected")).to_have_count(1)
            for control in [documents, note]:
                focus_style = control.evaluate("el=>{const s=getComputedStyle(el);return {outline:s.outlineStyle,shadow:s.boxShadow}}")
                assert focus_style == dict(outline="none", shadow="none"), "file names must not display an input-like focus box"
            assert note.evaluate("el=>getComputedStyle(el.closest('.file-row')).outlineStyle") == "none", "pointer selection uses only the row highlight"
            page.screenshot(path=str(output / "file-focus-selection.png"))
            note.focus()
            note.press("Tab")
            page.keyboard.press("Shift+Tab")
            expect(note).to_be_focused()
            assert note.evaluate("el=>getComputedStyle(el).outlineStyle") == "none"
            focus_state = note.evaluate("el=>{const row=el.closest('.file-row'),s=getComputedStyle(row);return {root:document.documentElement.className,row:row.className,within:row.matches(':focus-within'),width:s.outlineWidth,style:s.outlineStyle,active:document.activeElement.className}}")
            assert focus_state["style"] == "solid" and .9 <= float(focus_state["width"].removesuffix("px")) <= 1.1, focus_state
            documents.click()
            chinese.click(modifiers=["Shift"])
            expect(local_pane.locator(".row-selected")).to_have_count(3)
            note.click()
            note.press("Control+a")
            expect(local_pane.locator(".row-selected")).to_have_count(3)
            note.press("Control+Space")
            expect(local_pane.locator(".row-selected")).to_have_count(2)
            documents.click()
            documents.press("Shift+ArrowDown")
            expect(local_pane.locator(".row-selected")).to_have_count(2)
            local_pane.get_by_role("button", name="清空", exact=True).click()
            checks.append("Explorer selection replaces normally, Ctrl toggles, Shift ranges, Ctrl+A and Shift+arrows use visible rows")

            note.click()
            info = local_pane.locator(".file-information")
            expect(info.locator(".file-information-title")).to_contain_text("notes.txt")
            expect(info.locator(".file-information-title")).to_contain_text("TXT")
            expect(info.locator(".file-information-title")).to_contain_text(str((local / "notes.txt").stat().st_size))
            expect(info.locator(".file-information-metadata")).to_contain_text(re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}"))
            expect(info.locator(".file-information-path")).to_have_text(str(local / "notes.txt"))
            page.screenshot(path=str(output / "file-details-info.png"))
            local_pane.get_by_role("button", name="平铺图标", exact=True).click()
            expect(local_pane.locator(".file-pane")).to_have_class(re.compile("icons-view"))
            assert local_pane.locator(".file-list").evaluate("el=>getComputedStyle(el).display") == "grid"
            expect(page.locator(".remote-slot .file-pane")).to_have_class(re.compile("details-view"))
            expect(local_pane.locator(".row-selected")).to_have_count(1)
            chinese.click(modifiers=["Shift"])
            expect(local_pane.locator(".row-selected")).to_have_count(2)
            page.screenshot(path=str(output / "files-icon-grid.png"))
            documents.dblclick()
            expect(local_path).to_have_attribute("aria-label", str(local / "Documents"))
            expect(local_pane.locator(".file-pane")).to_have_class(re.compile("icons-view"))
            local_pane.get_by_role("button", name="本地上级目录", exact=True).click()
            expect(documents).to_be_visible()
            local_pane.get_by_role("button", name="详细列表", exact=True).click()
            expect(local_pane.locator(".file-header")).to_be_visible()
            checks.append("Each pane switches independently between details and icon grid, keeps selection/navigation, and shows type, exact size, date and path")

            nested = local / "Documents" / "nested 中文"
            nested.mkdir()
            deep = nested / "deep.txt"
            deep.write_text("Tree child transfer fixture\n", encoding="utf-8")
            expand = local_pane.get_by_role("button", name="展开 Documents", exact=True)
            expect(expand).to_have_text("›")
            expand.click()
            nested_name = local_pane.get_by_role("button", name="nested 中文", exact=True)
            expect(nested_name).to_be_visible()
            expect(local_path).to_have_attribute("aria-label", str(local))
            expect(local_pane.get_by_role("button", name="收起 Documents", exact=True)).to_have_text("⌄")
            local_pane.get_by_role("button", name="展开 nested 中文", exact=True).click()
            deep_name = local_pane.get_by_role("button", name="deep.txt", exact=True)
            expect(deep_name).to_be_visible()
            deep_name.click()
            expect(local_pane.locator(".row-selected")).to_have_count(1)
            page.screenshot(path=str(output / "file-tree-expanded.png"))
            local_pane.get_by_role("button", name="收起 Documents", exact=True).click()
            expect(deep_name).to_have_count(0)
            expect(local_pane.locator(".row-selected")).to_have_count(0)
            checks.append("Folder arrows lazily expand nested rows without navigating, collapse descendants and clear hidden selections")

            documents.focus()
            documents.press("Control+l")
            editor = local_pane.locator(".path-input")
            expect(editor).to_be_visible()
            expect(editor).to_be_focused()
            editor.fill(str(nested))
            editor.press("Escape")
            expect(editor).to_be_hidden()
            expect(local_path).to_have_attribute("aria-label", str(local))
            documents.press("Control+l")
            editor.fill(str(nested))
            editor.press("Enter")
            expect(local_path).to_have_attribute("aria-label", str(nested))
            expect(local_pane.locator(".breadcrumbs .breadcrumb").last).to_have_text("nested 中文")
            local_pane.get_by_role("button", name="复制本地路径", exact=True).click()
            expect(local_pane.locator(".path-copy")).to_have_class(re.compile("copied"))
            # Read only the path just copied by this test; never print clipboard data.
            import ctypes
            user32, kernel32 = ctypes.windll.user32, ctypes.windll.kernel32
            user32.GetClipboardData.restype = ctypes.c_void_p
            kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
            kernel32.GlobalLock.restype = ctypes.c_void_p
            kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
            clipboard_deadline = time.monotonic() + 2
            while not user32.OpenClipboard(None):
                assert time.monotonic() < clipboard_deadline, "clipboard stayed busy"
                time.sleep(.02)
            try:
                memory = user32.GetClipboardData(13)
                buffer = kernel32.GlobalLock(memory)
                try:
                    assert ctypes.wstring_at(buffer) == str(nested), "copy must use the complete Unicode path"
                finally:
                    kernel32.GlobalUnlock(memory)
            finally:
                user32.CloseClipboard()
            local_pane.get_by_role("button", name=str(local), exact=True).click()
            expect(local_path).to_have_attribute("aria-label", str(local))
            deep.unlink()
            nested.rmdir()
            page.screenshot(path=str(output / "explorer-files.png"))
            checks.append("Breadcrumb ancestors navigate directly; Ctrl+L edits, Escape cancels, and native copy preserves the full Unicode path")
            assert page.locator(".module-nav").bounding_box()["y"] < 50, "module tabs should share the header row"
            expect(page.locator(".topbar").get_by_role("button", name="连接服务器", exact=True)).to_be_visible()
            expect(page.locator(".topbar").get_by_role("button", name="连接服务器", exact=True)).to_be_disabled()
            expect(page.locator(".connection-state")).to_have_text("请选择左侧服务器")
            expect(page.get_by_role("textbox", name="SSH 配置文件路径")).to_have_count(0)
            expect(page.get_by_role("button", name="编辑服务器", exact=True)).to_have_count(0)
            page.get_by_role("button", name="系统软件", exact=True).click()
            expect(page.get_by_text("连接服务器后管理系统软件", exact=True)).to_be_visible()
            page.get_by_role("button", name="文件管理", exact=True).click()
            scrollbar = page.locator(".server-list").evaluate("el=>({width:getComputedStyle(el,'::-webkit-scrollbar').width,buttons:getComputedStyle(el,'::-webkit-scrollbar-button').display})")
            assert scrollbar == dict(width="6px", buttons="none"), scrollbar
            page.screenshot(path=str(output / "workspace.png"))
            checks.append("Workspace has no permanent server forms or global edit/delete")

            search_box = page.get_by_role("textbox", name="搜索", exact=True)
            search_box.focus()
            focus_style = search_box.evaluate("el=>{const s=getComputedStyle(el);return {outline:s.outlineStyle,shadow:s.boxShadow,border:s.borderTopWidth,color:s.borderTopColor}}")
            assert focus_style["outline"] == "none" and focus_style["shadow"] == "none", focus_style
            assert .9 <= float(focus_style["border"].removesuffix("px")) <= 1.1 and focus_style["color"] == "rgb(100, 156, 255)", focus_style
            page.screenshot(path=str(output / "input-focus.png"))
            checks.append("Focused inputs use a single 1px accent border without an outer ring or shadow")

            for name in ["转到本地路径", "转到远程路径"]:
                expect(page.get_by_role("button", name=name, exact=True)).to_have_count(0)
            for name in ["本地上级目录", "刷新本地目录", "筛选本地文件", "收起活动面板", "暂停队列"]:
                control = page.get_by_role("button", name=name, exact=True)
                expect(control).to_be_visible()
                assert control.text_content() == "", f"icon action still has visible text: {name}"
                assert control.locator(".action-glyph").evaluate("el=>getComputedStyle(el).maskImage") != "none"
            page.get_by_role("button", name="收起活动面板", exact=True).click()
            expect(page.locator(".activity-dock")).to_have_class("activity-dock collapsed")
            page.get_by_role("button", name="展开活动面板", exact=True).click()
            page.get_by_role("button", name="暂停队列", exact=True).click()
            expect(page.get_by_role("button", name="继续队列", exact=True)).to_be_visible()
            page.get_by_role("button", name="继续队列", exact=True).click()
            checks.append("Sapid icon actions render SVG masks and dock/queue icons track state")
            local_path = local_pane.locator(".breadcrumb.current")
            local_pane.get_by_role("button", name="编辑本地路径", exact=True).click()
            local_pane.locator(".path-input").fill(str(local / "Documents"))
            local_pane.locator(".path-input").press("Enter")
            expect(page.locator(".local-slot .file-empty")).to_have_text("此文件夹为空")
            local_pane.get_by_role("button", name="编辑本地路径", exact=True).click()
            local_pane.locator(".path-input").fill(str(local))
            local_pane.locator(".path-input").press("Enter")
            expect(page.locator(".file-row")).to_have_count(3)

            more = page.locator(".host-more").first
            assert more.evaluate("el => getComputedStyle(el).opacity") == "0"
            page.locator(".host-row").first.hover()
            expect(more).to_have_css("opacity", "1")
            more.click()
            expect(page.get_by_role("button", name="编辑服务器", exact=True)).to_be_visible()
            page.screenshot(path=str(output / "host-menu.png"))
            page.get_by_role("button", name="编辑服务器", exact=True).click()
            page.screenshot(path=str(output / "after-edit.png"))
            dialog = page.get_by_role("dialog", name="编辑服务器", exact=True)
            expect(dialog).to_be_visible()
            expect(dialog.get_by_role("textbox", name="名称", exact=True)).to_have_value("开发服务器")
            assert page.locator(".app-shell").evaluate("el=>el.inert")
            assert dialog.evaluate("el=>el.contains(document.activeElement)")
            close = dialog.get_by_role("button", name="关闭对话框", exact=True)
            assert "sapid-close-button" in close.get_attribute("class")
            assert close.text_content() == ""
            page.keyboard.press("Tab")
            close.focus()
            page.wait_for_function("el=>getComputedStyle(el,'::after').visibility==='visible'", arg=close.element_handle())
            assert close.evaluate("el=>getComputedStyle(el,'::after').content") == '"' + tr("关闭对话框") + '"'
            page.screenshot(path=str(output / "close-icon-tooltip.png"))
            close.click()
            expect(dialog).to_have_count(0)
            checks.append("CloseButton has no visible text, has a tooltip/accessibility name, and closes the dialog")
            page.get_by_role("button", name="新建服务器", exact=True).click()
            dialog = page.get_by_role("dialog", name="新建服务器", exact=True)
            expect(dialog).to_be_visible()
            page.keyboard.press("Escape")
            expect(dialog).to_have_count(0)
            checks.append("Hover menu targets its own host; modal focus/background inert and Escape work")

            page.get_by_role("button", name="新建服务器", exact=True).click()
            dialog = page.get_by_role("dialog", name="新建服务器", exact=True)
            expect(dialog).to_be_visible()
            expect(dialog.get_by_role("textbox", name="私钥文件路径", exact=True)).to_have_count(0)
            dialog.get_by_role("button", name="私钥", exact=True).click()
            expect(dialog.get_by_role("textbox", name="私钥文件路径", exact=True)).to_be_visible()
            dialog.get_by_role("button", name="密码", exact=True).click()
            dialog.get_by_role("button", name="保存服务器", exact=True).click()
            expect(dialog.locator(".form-error")).to_be_visible()
            page.screenshot(path=str(output / "new-server-validation.png"))
            dialog.get_by_role("textbox", name="名称", exact=True).fill("测试站点")
            dialog.get_by_role("textbox", name="主机地址", exact=True).fill("test.example.invalid")
            dialog.get_by_role("textbox", name="用户名", exact=True).fill("tester")
            expect(dialog.locator(".form-error")).to_have_count(0)
            page.screenshot(path=str(output / "new-server.png"))
            dialog.get_by_role("button", name="保存服务器", exact=True).click()
            expect(dialog).to_have_count(0)
            expect(page.locator(".host-row.selected .host-name")).to_have_text("测试站点")
            page.get_by_role("button", name="所有服务器  3", exact=True).click()
            expect(page.locator(".host-row")).to_have_count(3)
            assert len(json.loads(config.read_text(encoding="utf-8"))["config"]["servers"]) == 3
            checks.append("Server modal has explicit auth choices, inline validation, transactional save")

            page.get_by_role("button", name="本地文件操作", exact=True).click()
            page.get_by_role("button", name="新建文件夹", exact=True).click()
            dialog = page.get_by_role("dialog", name="新建文件夹", exact=True)
            dialog.get_by_role("textbox", name="文件夹名称", exact=True).fill("Documents")
            dialog.get_by_role("button", name="创建文件夹", exact=True).click()
            expect(dialog.locator(".form-error")).to_be_visible()
            dialog.get_by_role("textbox", name="文件夹名称", exact=True).fill("new-folder")
            dialog.get_by_role("button", name="创建文件夹", exact=True).click()
            expect(dialog).to_have_count(0)
            assert (local / "new-folder").is_dir()
            checks.append("File actions use contextual dialog and close only after worker success")

            page.get_by_role("button", name="操作日志", exact=True).click()
            expect(page.locator(".logs")).to_be_visible()
            expect(page.locator(".overview")).to_have_count(0)
            expect(page.get_by_role("button", name="服务器信息", exact=True)).to_have_count(0)
            expect(page.locator(".dock-tab")).to_have_count(2)
            page.get_by_role("button", name="传输 (0)", exact=True).click()
            expect(page.locator(".logs")).to_have_count(0)
            checks.append("File activity dock contains only Transfers and Operation Logs, with one panel visible")

            page.get_by_role("button", name="新建分组", exact=True).click()
            dialog = page.get_by_role("dialog", name="新建分组", exact=True)
            dialog.get_by_role("textbox", name="分组名称", exact=True).fill("临时测试组")
            dialog.get_by_role("button", name="保存分组", exact=True).click()
            expect(dialog).to_have_count(0)
            expect(page.get_by_role("button", name="临时测试组", exact=True)).to_be_visible()
            checks.append("Group management uses the same modal primitive")

            imported = output / "ssh_config"
            imported.write_text("\n".join(f"Host fixture-{n}\n HostName fixture-{n}.example.invalid\n User tester" for n in range(20)), encoding="utf-8")
            page.get_by_role("button", name="导入 SSH 配置", exact=True).click()
            dialog = page.get_by_role("dialog", name="导入 SSH 配置", exact=True)
            dialog.get_by_role("textbox", name="SSH 配置文件路径", exact=True).fill(str(output / "missing-config"))
            dialog.get_by_role("button", name="导入站点", exact=True).click()
            expect(dialog.locator(".form-error")).to_be_visible()
            dialog.get_by_role("textbox", name="SSH 配置文件路径", exact=True).fill(str(imported))
            dialog.get_by_role("button", name="导入站点", exact=True).click()
            expect(dialog).to_have_count(0)
            expect(page.locator(".host-row")).to_have_count(23)
            last_host = page.locator(".host-row").last
            last_host.scroll_into_view_if_needed()
            last_host.locator(".host-more").focus()
            last_host.locator(".host-more").click()
            popover = page.locator(".context-popover")
            expect(popover).to_be_visible()
            box = popover.bounding_box()
            assert box["y"] >= 0 and box["y"] + box["height"] <= page.evaluate("innerHeight"), "host menu clipped by viewport"
            page.screenshot(path=str(output / "long-list-menu.png"))
            page.get_by_role("button", name="删除服务器", exact=True).click()
            dialog = page.get_by_role("dialog", name="删除服务器", exact=True)
            expect(dialog).to_be_visible()
            expect(dialog).to_contain_text("fixture-19")
            page.keyboard.press("Escape")
            checks.append("Import errors stay inline; long-list menu avoids clipping; delete targets named host and can cancel")

            page.set_viewport_size(dict(width=1000, height=700))
            page.locator(".sidebar-heading").scroll_into_view_if_needed()
            page.get_by_role("button", name="新建服务器", exact=True).click()
            dialog = page.get_by_role("dialog", name="新建服务器", exact=True)
            expect(dialog).to_be_visible()
            box = dialog.bounding_box()
            assert box["x"] >= 0 and box["y"] >= 0 and box["y"] + box["height"] <= 700
            footer = dialog.locator(".modal-footer").bounding_box()
            assert footer["y"] + footer["height"] <= 700
            page.screenshot(path=str(output / "narrow-dialog.png"))
            page.keyboard.press("Escape")
            checks.append("1000x700 dialog footer stays in viewport")
            page.set_viewport_size(dict(width=1440, height=980))
            page.locator(".host-select").first.click()
            expect(page.locator(".topbar").get_by_role("button", name="连接服务器", exact=True)).to_be_enabled()
            expect(page.locator(".connection-target")).to_have_text("开发服务器")
            splitter = page.locator(".pane-splitter")
            def drag_splitter(delta):
                box = splitter.bounding_box()
                x, y = box["x"] + box["width"] / 2, box["y"] + 80
                page.mouse.move(x, y)
                page.mouse.down()
                page.mouse.move(x + delta, y, steps=12)
                page.mouse.up()
                page.wait_for_timeout(150)

            width_before = page.locator(".local-slot").bounding_box()["width"]
            drag_splitter(140)
            assert page.locator(".local-slot").bounding_box()["width"] > width_before + 80, "divider must resize both panes"
            splitter.focus()
            page.keyboard.press("Home")
            page.wait_for_timeout(100)
            assert abs(page.locator(".local-slot").bounding_box()["width"] - width_before) < 3, "Home resets equal panes"
            drag_splitter(120)
            first_width = page.locator(".local-slot").bounding_box()["width"]
            first_ratio = json.loads(config.read_text(encoding="utf-8"))["config"]["servers"][0]["file_pane_ratio"]
            page.locator(".host-select").nth(1).click()
            drag_splitter(-100)
            saved_layout = json.loads(config.read_text(encoding="utf-8"))["config"]["servers"]
            assert saved_layout[1]["file_pane_ratio"] < 500 < first_ratio, "different hosts must persist independent splits"
            page.locator(".host-select").first.click()
            assert abs(page.locator(".local-slot").bounding_box()["width"] - first_width) < 3, "host switch restores its layout"
            page.screenshot(path=str(output / "split-panes.png"))
            checks.append("Compact header and captured splitter resize, reset, persist and restore independently for each server")
            if package_checks:
                page.set_viewport_size(dict(width=1440, height=980))
                page.locator(".host-select").first.click()
                page.get_by_role("button", name="系统软件", exact=True).click()
                page.locator(".topbar").get_by_role("button", name="连接服务器", exact=True).click()
                dialog = page.get_by_role("dialog", name="连接服务器", exact=True)
                expect(dialog).to_be_visible()
                # Unencrypted, generated fixture key needs no input secret.
                dialog.get_by_role("button", name="连接", exact=True).click()
                expect(page.locator(".connection-state")).to_have_text("SSH 已连接")
                expect(page.locator(".software-environment")).to_contain_text("Ubuntu 24.04 LTS")
                expect(page.get_by_role("button", name="已安装 curl", exact=True)).to_be_visible()
                assert "Connected" in page.locator(".host-row.selected .host-address").inner_text()
                page.locator(".settings-link").click()
                page.locator(".settings-section").first.locator(".choice").nth(1).click()
                expect(page.locator(".connection-state")).to_have_text(re.compile(r"^SSH 已连接$"))
                page.locator(".settings-section").first.locator(".choice").first.click()
                expect(page.locator(".connection-state")).to_have_text("SSH 已连接")
                page.get_by_role("button",name="系统软件",exact=True).click()
                expect(page.get_by_role("button",name="已安装 curl",exact=True)).to_be_visible()
                checks.append("Live language changes preserve the connected SSH session and module state")
                page.get_by_role("button", name="资源概览", exact=True).click()
                expect(page.locator(".metric-card").nth(1)).to_contain_text("50.0%")
                expect(page.locator(".metric-card").first).to_contain_text("等待下一次采样")
                assert not re.search(r"[\u3400-\u9fff]",page.locator(".resources-page").inner_text()), "Resource UI should use English"
                expect(page.locator(".metric-card").first).to_contain_text("30.0%", timeout=10000)
                expect(page.locator(".resources-page")).to_contain_text("1.0 KiB/s")
                expect(page.locator(".resources-page")).to_contain_text("/srv/my data")
                page.screenshot(path=str(output / "resources.png"))
                page.set_viewport_size(dict(width=1000, height=700))
                assert page.locator(".metric-grid").evaluate("el=>el.scrollWidth<=el.clientWidth"), "resource cards overflow narrow page"
                assert page.locator(".metric-grid").evaluate("el=>getComputedStyle(el).gridTemplateColumns.split(' ').length===2"), "narrow resource cards should use two columns"
                page.screenshot(path=str(output / "resources-narrow.png"))
                page.set_viewport_size(dict(width=1440, height=980))
                checks.append("Numeric resources calculate real SSH sample deltas, display disks/network, and fit a narrow viewport")

                page.get_by_role("button", name="系统日志", exact=True).click()
                logs = page.locator(".system-log-output")
                expect(logs).to_contain_text("Accepted publickey for arcnix")
                expect(logs).to_contain_text("Second line preserved")
                page.get_by_role("textbox", name="筛选当前日志", exact=True).fill("warning")
                expect(logs).to_contain_text("Fixture service warning")
                expect(logs).not_to_contain_text("Accepted publickey")
                page.get_by_role("textbox", name="筛选当前日志", exact=True).fill("")
                page.screenshot(path=str(output / "system-logs.png"))
                page.get_by_role("textbox", name="日志服务单元", exact=True).fill("ssh.service")
                page.get_by_role("button", name="查询日志", exact=True).click()
                expect(logs).to_contain_text("Accepted publickey")
                expect(logs).not_to_contain_text("Fixture service warning")
                page.get_by_role("button", name="日志时间范围：今天", exact=True).click()
                expect(page.get_by_role("button", name="查询日志", exact=True)).to_be_enabled()
                page.get_by_role("button", name="日志严重级别：错误及以上", exact=True).click()
                expect(logs).to_contain_text("当前条件下没有日志记录")
                page.get_by_role("button", name="日志严重级别：全部级别", exact=True).click()
                expect(logs).to_contain_text("Accepted publickey")
                page.get_by_role("textbox", name="日志服务单元", exact=True).fill("denied.service")
                expect(logs).to_have_text("")
                page.get_by_role("button", name="查询日志", exact=True).click()
                expect(page.locator(".system-logs-page .software-error")).to_contain_text("Permission denied")
                page.get_by_role("button", name="系统日志 /var/log/syslog", exact=True).click()
                expect(logs).to_contain_text("Fixture file log entry")
                expect(page.get_by_role("textbox", name="日志服务单元", exact=True)).to_have_count(0)
                page.get_by_role("button", name="切换日志自动刷新", exact=True).click()
                expect(page.get_by_role("button", name="切换日志自动刷新", exact=True)).to_contain_text("On")
                status = page.locator(".system-logs-page .observability-status")
                previous_status = status.inner_text()
                expect(status).not_to_have_text(re.compile(re.escape(previous_status) + "$|" + re.escape(tr("正在读取日志…")) + "$"), timeout=10000)
                page.set_viewport_size(dict(width=1000, height=700))
                assert page.locator(".system-logs-page").evaluate("el=>el.scrollWidth<=el.clientWidth"), "log page overflows narrow viewport"
                page.screenshot(path=str(output / "system-logs-narrow.png"))
                page.set_viewport_size(dict(width=1440, height=980))
                checks.append("System logs discover sources, preserve multiline entries, filter service/time/severity/text, show permission errors and read file logs")
                page.get_by_role("button", name="系统软件", exact=True).click()
                page.get_by_role("textbox", name="搜索软件包", exact=True).fill("nginx")
                page.get_by_role("button", name="搜索", exact=True).click()
                expect(page.get_by_role("button", name="预览安装 nginx", exact=True)).to_be_visible()
                page.screenshot(path=str(output / "software.png"))
                page.get_by_role("button", name="预览安装 nginx", exact=True).click()
                dialog = page.get_by_role("dialog", name="确认系统操作", exact=True)
                expect(dialog).to_be_visible()
                expect(dialog.locator(".package-preview")).to_contain_text("Inst nginx")
                expect(dialog.locator(".package-command")).to_contain_text("sudo -n")
                expect(dialog.locator(".package-command")).not_to_contain_text("arcnix:packages")
                page.screenshot(path=str(output / "package-confirm.png"))
                dialog.get_by_role("button", name="确认执行", exact=True).click()
                expect(page.locator(".host-select").nth(1)).to_be_disabled()
                expect(page.locator(".topbar").get_by_role("button", name="断开连接", exact=True)).to_be_disabled()
                expect(page.locator(".software-output")).to_contain_text("Package operation completed")
                expect(page.locator(".software-output")).to_contain_text("退出码 0")
                expect(page.get_by_role("button", name="已安装 nginx", exact=True)).to_be_visible()
                page.screenshot(path=str(output / "package-completed.png"))
                page.get_by_role("button", name="更新软件源索引", exact=True).click()
                dialog = page.get_by_role("dialog", name="确认系统操作", exact=True)
                expect(dialog).to_be_visible()
                expect(dialog.locator(".package-preview")).to_contain_text("不会升级")
                expect(dialog.locator(".package-command")).to_contain_text("update")
                dialog.get_by_role("button", name="取消", exact=True).click()
                expect(dialog).to_have_count(0)
                page.get_by_role("button", name="文件管理", exact=True).click()
                remote_pane = page.locator(".remote-slot")
                remote_path = remote_pane.locator(".breadcrumb.current")
                remote_folder = remote_pane.get_by_role("button", name="folder", exact=True)
                expect(remote_folder).to_have_text("folder")
                remote_folder.click()
                expect(remote_path).to_have_attribute("aria-label", "/")
                expect(remote_folder.locator("xpath=..")).to_have_class(re.compile(r"row-selected"))
                remote_folder.dblclick()
                expect(remote_path).to_have_attribute("aria-label", "/folder")
                expect(remote_pane.locator(".file-empty")).to_have_text("此文件夹为空")
                remote_pane.get_by_role("button", name="远程上级目录", exact=True).click()
                expect(remote_path).to_have_attribute("aria-label", "/")
                expect(remote_folder).to_be_visible()
                remote_folder.focus()
                remote_folder.press("Enter")
                expect(remote_path).to_have_attribute("aria-label", "/folder")
                remote_pane.get_by_role("button", name="远程上级目录", exact=True).click()
                expect(remote_path).to_have_attribute("aria-label", "/")
                expect(remote_folder).to_be_visible()
                remote_nested = Path(fixture["root"]) / "folder" / "nested 空格"
                remote_nested.mkdir()
                remote_child = remote_nested / "remote-child.txt"
                remote_child.write_bytes(b"Expanded remote file transfer\n")
                remote_pane.get_by_role("button", name="展开 folder", exact=True).click()
                expect(remote_pane.get_by_role("button", name="nested 空格", exact=True)).to_be_visible()
                remote_pane.get_by_role("button", name="展开 nested 空格", exact=True).click()
                child_button = remote_pane.get_by_role("button", name="remote-child.txt", exact=True)
                expect(child_button).to_be_visible()
                child_button.click()
                expect(remote_path).to_have_attribute("aria-label", "/")
                remote_pane.get_by_role("button", name="远程文件操作", exact=True).click()
                page.get_by_role("button", name="重命名", exact=True).click()
                rename_dialog = page.get_by_role("dialog", name="重命名", exact=True)
                rename_dialog.get_by_role("textbox", name="新名称", exact=True).fill("renamed-child.txt")
                rename_dialog.get_by_role("button", name="重命名", exact=True).click()
                expect(rename_dialog).to_have_count(0)
                renamed_child = remote_nested / "renamed-child.txt"
                assert renamed_child.exists() and not remote_child.exists(), "rename must stay in the child's own directory"
                remote_pane.get_by_role("button", name="展开 folder", exact=True).click()
                remote_pane.get_by_role("button", name="展开 nested 空格", exact=True).click()
                remote_pane.get_by_role("button", name="renamed-child.txt", exact=True).click()
                remote_pane.get_by_role("button", name="下载", exact=True).click()
                expect(page.locator(".queue-row").last).to_contain_text("已完成")
                assert (local / "renamed-child.txt").read_bytes() == renamed_child.read_bytes()
                remote_folder.dblclick()
                remote_pane.get_by_role("button", name="nested 空格", exact=True).dblclick()
                expect(remote_path).to_have_attribute("aria-label", "/folder/nested 空格")
                remote_pane.get_by_role("button", name="复制远程路径", exact=True).click()
                expect(remote_pane.locator(".path-copy")).to_have_class(re.compile("copied"))
                clipboard_deadline = time.monotonic() + 2
                while not user32.OpenClipboard(None):
                    assert time.monotonic() < clipboard_deadline, "clipboard stayed busy"
                    time.sleep(.02)
                try:
                    memory = user32.GetClipboardData(13)
                    buffer = kernel32.GlobalLock(memory)
                    try:
                        assert ctypes.wstring_at(buffer) == "/folder/nested 空格", "remote copy must retain the complete POSIX path"
                    finally:
                        kernel32.GlobalUnlock(memory)
                finally:
                    user32.CloseClipboard()
                remote_pane.get_by_role("button", name="/", exact=True).click()
                expect(remote_path).to_have_attribute("aria-label", "/")
                remote_pane.get_by_role("button", name="平铺图标", exact=True).click()
                expect(remote_pane.locator(".file-pane")).to_have_class(re.compile("icons-view"))
                expect(local_pane.locator(".file-pane")).to_have_class(re.compile("details-view"))
                remote_folder.dblclick()
                remote_pane.get_by_role("button", name="nested 空格", exact=True).dblclick()
                remote_pane.get_by_role("button", name="renamed-child.txt", exact=True).click()
                remote_info = remote_pane.locator(".file-information")
                expect(remote_info).to_contain_text("renamed-child.txt")
                expect(remote_info.locator(".file-information-metadata")).to_contain_text(re.compile(r"[0-7]{4}"))
                expect(remote_info.locator(".file-information-path")).to_have_text("/folder/nested 空格/renamed-child.txt")
                remote_pane.get_by_role("button", name="/", exact=True).click()
                expect(remote_path).to_have_attribute("aria-label", "/")
                remote_pane.get_by_role("button", name="详细列表", exact=True).click()
                renamed_child.unlink()
                remote_nested.rmdir()
                checks.append("Remote icon grid preserves folder navigation and displays permission metadata and full POSIX file paths")
                checks.append("Expanded SFTP children rename in their own parent, download correctly, and POSIX breadcrumbs/copy retain Unicode")
                checks.append("Remote SFTP folders share single-selection, double-click entry and Enter navigation")
                remote_note = Path(fixture["root"]) / "notes.txt"
                remote_note.write_bytes(b"Keep existing remote content\n")
                local_note = local / "notes.txt"
                local_pane, remote_pane = page.locator(".file-pane").nth(0), page.locator(".file-pane").nth(1)

                def upload_note():
                    count = page.locator(".queue-row").count()
                    local_pane.get_by_role("button", name="notes.txt", exact=True).click()
                    local_pane.get_by_role("button", name="上传", exact=True).click()
                    expect(page.locator(".queue-row")).to_have_count(count + 1)

                upload_note()
                dialog = page.get_by_role("dialog", name="目标文件已存在", exact=True)
                expect(dialog).to_be_visible()
                expect(dialog).to_contain_text("notes.txt")
                assert not re.search(r"[\u3400-\u9fff]", " ".join(dialog.locator(".choice-list button").all_text_contents())), "Conflict choices should use English"
                dialog.get_by_role("button", name="跳过文件", exact=True).click()
                expect(page.locator(".queue-row").last).to_contain_text("已跳过")
                assert remote_note.read_bytes() == b"Keep existing remote content\n"
                upload_note()
                expect(dialog).to_be_visible()
                dialog.get_by_role("button", name="冲突处理：覆盖", exact=True).click()
                dialog.get_by_role("button", name="冲突规则范围：以后连接此服务器", exact=True).click()
                page.screenshot(path=str(output / "file-conflict.png"))
                dialog.get_by_role("button", name="覆盖文件", exact=True).click()
                expect(dialog).to_have_count(0)
                expect(page.locator(".queue-row").last).to_contain_text("已完成")
                assert remote_note.read_bytes() == local_note.read_bytes()
                persisted_rules = json.loads(config.read_text(encoding="utf-8"))["config"]["transfer_rules"]
                assert persisted_rules == [dict(server_id="development", direction="Upload", action="Overwrite")]
                local_note.write_bytes(b"Second upload applies the remembered rule\n")
                upload_note()
                expect(page.locator(".queue-row").last).to_contain_text("已完成")
                expect(dialog).to_have_count(0)
                assert remote_note.read_bytes() == local_note.read_bytes()
                page.locator(".topbar").get_by_role("button", name="断开连接", exact=True).click()
                expect(page.locator(".connection-state")).to_have_text("SSH 未连接")
                host_action("连接服务器")
                page.get_by_role("dialog", name="连接服务器", exact=True).get_by_role("button", name="连接", exact=True).click()
                expect(remote_pane.get_by_role("button", name="notes.txt", exact=True)).to_be_visible()
                local_note.write_bytes(b"Remembered upload rule survives a new connection\n")
                upload_note()
                expect(page.locator(".queue-row").last).to_contain_text("已完成")
                expect(dialog).to_have_count(0)
                assert remote_note.read_bytes() == local_note.read_bytes()
                remote_note.write_bytes(b"Downloaded replacement\n")
                count = page.locator(".queue-row").count()
                remote_pane.get_by_role("button", name="notes.txt", exact=True).click()
                remote_pane.get_by_role("button", name="下载", exact=True).click()
                expect(page.locator(".queue-row")).to_have_count(count + 1)
                expect(dialog).to_be_visible()
                expect(dialog).to_contain_text("A file with this name already exists in Local")
                dialog.get_by_role("button", name="冲突处理：覆盖", exact=True).click()
                dialog.get_by_role("button", name="覆盖文件", exact=True).click()
                expect(page.locator(".queue-row").last).to_contain_text("已完成")
                assert local_note.read_bytes() == remote_note.read_bytes()
                host_action("重置文件冲突选择")
                upload_note()
                expect(dialog).to_be_visible()
                dialog.get_by_role("button", name="关闭对话框", exact=True).click()
                expect(dialog).to_have_count(0)
                expect(page.locator(".queue-row").last).to_contain_text("已取消")
                checks.append("SFTP conflicts skip or overwrite both directions, remember per-host upload rules, reset choices and cancel safely")
                page.get_by_role("button", name="系统软件", exact=True).click()
                host_action("断开连接")
                expect(page.get_by_text("连接服务器后管理系统软件", exact=True)).to_be_visible()
                page.get_by_role("button", name="资源概览", exact=True).click()
                expect(page.locator(".metric-card")).to_have_count(0)
                page.get_by_role("button", name="系统日志", exact=True).click()
                expect(page.locator(".system-log-output")).to_have_count(0)
                expect(page.get_by_text("连接服务器后查看系统日志。", exact=True)).to_be_visible()
                checks.append("Software module detects, queries, previews and streams a confirmed canned package operation over real loopback SSH")
                host_action("编辑服务器")
                edit_dialog = page.get_by_role("dialog", name="编辑服务器", exact=True)
                edit_dialog.get_by_role("textbox", name="用户名", exact=True).fill("arcnix-nosftp")
                edit_dialog.get_by_role("button", name="保存服务器", exact=True).click()
                expect(edit_dialog).to_have_count(0)
                page.locator(".topbar").get_by_role("button", name="连接服务器", exact=True).click()
                page.get_by_role("dialog", name="连接服务器", exact=True).get_by_role("button", name="连接", exact=True).click()
                expect(page.locator(".connection-state")).to_have_text("SSH 已连接")
                page.get_by_role("button", name="文件管理", exact=True).click()
                expect(page.locator(".remote-slot .pane-heading .pane-summary")).to_have_text("SFTP · 不可用")
                expect(page.locator(".remote-slot .file-empty")).to_contain_text("SSH 已连接，但 SFTP 不可用")
                expect(page.locator(".remote-slot").get_by_role("button", name="下载", exact=True)).to_be_disabled()
                page.screenshot(path=str(output / "ssh-without-sftp.png"))
                page.get_by_role("button", name="资源概览", exact=True).click()
                expect(page.locator(".metric-card").nth(1)).to_contain_text("50.0%")
                page.get_by_role("button", name="系统软件", exact=True).click()
                expect(page.get_by_role("button", name="已安装 curl", exact=True)).to_be_visible()
                page.get_by_role("button", name="系统日志", exact=True).click()
                expect(page.locator(".system-log-output")).to_contain_text("Accepted publickey")
                page.locator(".topbar").get_by_role("button", name="断开连接", exact=True).click()
                expect(page.locator(".connection-state")).to_have_text("SSH 未连接")
                checks.append("Global SSH controls remain visible across modules; unavailable SFTP disables only files while resources/software/logs work")
            browser.close()
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                restart_port = sock.getsockname()[1]
            endpoint = f"http://127.0.0.1:{restart_port}"
            env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={restart_port} --remote-debugging-address=127.0.0.1"
            process = subprocess.Popen([str(WORKSPACE / "target" / "debug" / "arcnix.exe"), "--config", str(config)], cwd=WORKSPACE, env=env, stdout=log, stderr=log)
            deadline = time.monotonic() + 15
            while True:
                try:
                    with urllib.request.urlopen(endpoint + "/json/version", timeout=1) as response:
                        json.load(response)
                    break
                except OSError:
                    if time.monotonic() > deadline:
                        raise RuntimeError("Restarted desktop did not expose its test endpoint")
                    time.sleep(.15)
            browser = playwright.chromium.connect_over_cdp(endpoint)
            page = browser.contexts[0].pages[0]
            page.set_viewport_size(dict(width=1440, height=980))
            page.wait_for_selector(".host-row")
            for index in [0, 1]:
                page.locator(".host-select").nth(index).click()
                page.wait_for_timeout(100)
                left = page.locator(".local-slot").bounding_box()["width"]
                right = page.locator(".remote-slot").bounding_box()["width"]
                expected_ratio = saved_layout[index]["file_pane_ratio"] / 1000
                assert abs(left / (left + right) - expected_ratio) < .005, "restart must restore each host's saved divider position"
            checks.append("A fresh desktop process restores both servers' independent divider ratios from local configuration")
            if "--legacy-host-key" in sys.argv:
                recorded = json.loads(config.read_text(encoding="utf-8"))["config"]["trusted_hosts"]
                assert any(item["fingerprint"] == fixture["host_key_rsa_fingerprint"] for item in recorded), "successful negotiation must preserve the original RSA trust record"
                assert all(item["fingerprint"] == fixture["host_key_rsa_fingerprint"] for item in recorded), "must not auto-trust a newly preferred host key"
            (output / "results.json").write_text(json.dumps(dict(checks=checks), indent=2, ensure_ascii=False), encoding="utf-8")
            print(json.dumps(dict(passed=len(checks), checks=checks), ensure_ascii=False))
            browser.close()
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        log.close()


if __name__ == "__main__":
    main()
