"""Real WebView + SSH PTY integration; fixed loopback peer, no OS shell."""
from pathlib import Path
import json, os, socket, subprocess, sys, time, urllib.request, re, hashlib
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'artifacts/python'))
from playwright.sync_api import sync_playwright, expect

def main():
    fixture=json.loads((ROOT/'artifacts/fixture-terminal.json').read_text())
    host, port=fixture['address'].rsplit(':',1)
    assert host=='127.0.0.1'
    output=ROOT/'artifacts/terminal-ui'
    output.mkdir(parents=True,exist_ok=True)
    local=output/('files-'+str(time.time_ns()))
    local.mkdir()
    filename='terminal-'+str(time.time_ns())+'.txt'
    (local/filename).write_text('Terminal concurrent SFTP fixture\n',encoding='utf-8')
    profile=dict(id='terminal-test',name='Terminal test',group_id=None,host=host,port=int(port),username='arcnix',auth='PrivateKey',private_key=fixture['client_key_ed25519'],initial_remote_path='/')
    config=output/'config.json'
    config.write_text(json.dumps(dict(version=1,config=dict(servers=[profile],groups=[],trusted_hosts=[dict(host=host,port=int(port),fingerprint=fixture['host_key_rsa_fingerprint'])],local_directory=str(local)))))
    with socket.socket() as listener:
        listener.bind(('127.0.0.1',0)); debug_port=listener.getsockname()[1]
    endpoint=f'http://127.0.0.1:{debug_port}'
    env=os.environ.copy()
    env['WEBVIEW2_USER_DATA_FOLDER']=str(output/('webview-'+str(time.time_ns())))
    env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']=f'--remote-debugging-port={debug_port} --remote-debugging-address=127.0.0.1'
    systemroot=Path(os.environ.get('SystemRoot',r'C:\Windows'))
    env['PATH']=str(systemroot/'System32')+os.pathsep+str(systemroot)
    log=(output/'desktop.log').open('w',encoding='utf-8')
    process=subprocess.Popen([os.environ.get('ARCNIX_TEST_BINARY',str(ROOT/'target/debug/arcnix.exe')),'--config',str(config)],cwd=ROOT,env=env,stdout=log,stderr=log)
    checks=[]
    try:
        deadline=time.monotonic()+20
        while True:
            try:
                urllib.request.urlopen(endpoint+'/json/version',timeout=1).close();break
            except OSError:
                assert process.poll() is None,'desktop exited'
                assert time.monotonic()<deadline,'CDP unavailable'
                time.sleep(.1)
        with sync_playwright() as playwright:
            browser=playwright.chromium.connect_over_cdp(endpoint)
            page=browser.contexts[0].pages[0]
            page.set_default_timeout(10000)
            page.on('pageerror',lambda error:print('WEBVIEW ERROR:',error,flush=True))
            if page.locator('.rail-servers').count():
                page.locator('.rail-servers').click()
            page.locator('.host-select').first.click()
            page.locator('.connection-controls').get_by_role('button',name='Connect to server',exact=True).click()
            page.get_by_role('dialog',name='Connect to server',exact=True).get_by_role('button',name='Connect',exact=True).click()
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            page.get_by_role('button',name='Terminal',exact=True).click()
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            screen=page.locator('.terminal-surface')
            reader=screen.locator('.xterm-accessibility-tree')
            expect(reader).to_contain_text('Arcnix terminal fixture')
            keyboard=screen.locator('.xterm-helper-textarea')
            def command(text):
                expect(screen).to_be_visible()
                keyboard.focus();expect(keyboard).to_be_focused()
                page.keyboard.type(text);page.keyboard.press('Enter')
            command('pwd')
            expect(reader).to_contain_text('/fixture')
            command('unicode')
            expect(reader).to_contain_text('中文终端')
            command('ansi')
            expect(reader).to_contain_text('RED')
            keyboard.focus();page.keyboard.press('Control+c')
            expect(reader).to_contain_text('^C')
            page.screenshot(path=str(output/'terminal.png'))
            checks.append('Global SSH credentials and the original RSA pin open a real PTY with typed input, UTF-8, ANSI and Ctrl+C')
            row=reader.evaluate("el=>({count:el.children.length,index:[...el.children].findIndex(row=>row.textContent.includes('中文终端'))})")
            geometry=screen.locator('.xterm-screen').bounding_box()
            cell=screen.locator('.xterm').evaluate("el=>{const c=document.createElement('canvas'),x=c.getContext('2d'),s=getComputedStyle(el);x.font=s.fontSize+' '+s.fontFamily;return x.measureText('W').width}")
            y=geometry['y']+(row['index']+.5)*geometry['height']/row['count']
            page.mouse.move(geometry['x']+1,y);page.mouse.down();page.mouse.move(geometry['x']+cell*8+1,y,steps=8);page.mouse.up()
            expect(page.locator('.terminal-copy')).to_be_enabled()
            page.get_by_role('button',name='Copy selected text',exact=True).click()
            expect(page.locator('.terminal-copy')).to_have_class(re.compile('copied'))
            command('osc52')
            keyboard.focus();page.keyboard.type('echo ');page.keyboard.press('Control+Shift+v')
            expect(reader).to_contain_text('echo 中文终端')
            page.keyboard.press('Enter')
            keyboard.focus();page.keyboard.type('echo PASTE_BUTTON-')
            page.get_by_role('button',name='Paste into terminal',exact=True).click()
            expect(reader).to_contain_text('echo PASTE_BUTTON-中文终端')
            keyboard.focus();page.keyboard.press('Enter')
            checks.append('Unicode clipboard copy/paste works and remote OSC 52 output cannot replace clipboard content')
            payload='abcdefghijklmnopqrstuvwxyz0123456789'*32
            command('checksum '+payload)
            expect(reader).to_contain_text(hashlib.sha256(payload.encode()).hexdigest())
            checks.append('Rapid typing is acknowledged and delivered without losing, duplicating or reordering characters')
            command('echo persistent-marker')
            expect(reader).to_contain_text('persistent-marker')
            page.get_by_role('button',name='Files',exact=True).click()
            page.locator('.local-slot').get_by_role('button',name=filename,exact=True).click()
            page.locator('.local-slot').get_by_role('button',name='Upload',exact=True).click()
            expect(page.locator('.queue-row').last).to_contain_text('Completed')
            page.get_by_role('button',name='Terminal',exact=True).click()
            expect(reader).to_contain_text('persistent-marker')
            command('echo after-upload')
            expect(reader).to_contain_text('after-upload')
            checks.append('Terminal history/session survive module changes and an actual SFTP upload')
            page.locator('.settings-link').click()
            page.get_by_role('button',name='简体中文',exact=True).click()
            expect(page.locator('.context-title')).to_have_text('设置')
            page.locator('.rail-servers').click()
            (output/'language.txt').write_text(page.locator('body').inner_text(),encoding='utf-8')
            page.screenshot(path=str(output/'language.png'))
            page.get_by_role('button',name='终端',exact=True).click()
            expect(page.locator('.terminal-status')).to_have_text('交互式 SSH')
            expect(reader).to_contain_text('persistent-marker')
            command('echo after-language-change')
            expect(reader).to_contain_text('after-language-change')
            page.locator('.settings-link').click()
            page.get_by_role('button',name='English',exact=True).click()
            expect(page.locator('.context-title')).to_have_text('Settings')
            page.locator('.rail-servers').click()
            page.get_by_role('button',name='Terminal',exact=True).click()
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            expect(reader).to_contain_text('after-language-change')
            checks.append('Live language remounts preserve the terminal renderer, SSH PTY, screen and ability to keep typing')
            page.set_viewport_size(dict(width=1000,height=700))
            page.wait_for_timeout(150)
            command('size')
            expect(reader).to_contain_text(re.compile(r'[0-9]+x[0-9]+'))
            page.screenshot(path=str(output/'terminal-narrow.png'))
            checks.append('Terminal reflows and sends PTY resize requests when the workspace shrinks')
            command('burst')
            expect(reader).to_contain_text('BURST COMPLETE',timeout=15000)
            checks.append('Acknowledged output drains more than 256 KiB without freezing or dropping the final marker')
            command('exit')
            expect(page.locator('.terminal-status')).to_have_text('Terminal exited (0)')
            expect(page.locator('.connection-state')).to_have_text('SSH connected')
            page.get_by_role('button',name='Restart terminal',exact=True).click()
            expect(page.locator('.terminal-status')).to_have_text('Interactive SSH')
            expect(reader).to_contain_text('Arcnix terminal fixture')
            expect(reader).not_to_contain_text('persistent-marker')
            checks.append('Shell exit and restart leave global SSH connected and replace the old terminal screen')
            page.locator('.connection-controls').get_by_role('button',name='Disconnect',exact=True).click()
            expect(page.locator('.terminal-status')).to_have_text('Connect a server to use the terminal')
            expect(screen.locator('.xterm')).to_have_count(0)
            checks.append('Global disconnect disposes the terminal and clears its screen')
            (output/'results.json').write_text(json.dumps(dict(checks=checks),indent=2),encoding='utf-8')
            print(json.dumps(dict(passed=len(checks),checks=checks)),flush=True)
            browser.close()
    except Exception:
        try:
            (output/'failure.txt').write_text(page.locator('body').inner_text(),encoding='utf-8')
            page.screenshot(path=str(output/'failure.png'))
        except Exception:
            pass
        raise
    finally:
        if process.poll() is None:process.terminate();process.wait(timeout=5)
        log.close()
if __name__=='__main__':main()
