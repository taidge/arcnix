"""Actual WebView/SSH/SFTP progress, throttled loopback peer, isolated files only."""
from pathlib import Path
import json, os, re, socket, subprocess, sys, time, urllib.request, hashlib
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'artifacts/python'))
from playwright.sync_api import sync_playwright, expect

def main():
    output=ROOT/'artifacts/transfer-progress-ui';output.mkdir(parents=True,exist_ok=True)
    ready=output/('fixture-'+str(time.time_ns())+'.json')
    log=(output/'desktop.log').open('w',encoding='utf-8')
    fixture=subprocess.Popen([sys.executable,str(ROOT/'tools/sftp_fixture.py'),'--ready',str(ready),'--io-delay','0.012'],cwd=ROOT,stdout=log,stderr=log)
    app=None;checks=[]
    try:
        deadline=time.monotonic()+20
        while not ready.exists():
            assert fixture.poll() is None and time.monotonic()<deadline
            time.sleep(.1)
        peer=json.loads(ready.read_text());host,port=peer['address'].rsplit(':',1);assert host=='127.0.0.1'
        local=output/('files-'+str(time.time_ns()));local.mkdir()
        content=bytes(range(256))*24576
        filename='progress.bin';path=local/filename;path.write_bytes(content)
        config=output/'config.json'
        profile=dict(id='progress-test',name='Transfer test',host=host,port=int(port),username='arcnix',auth='PrivateKey',private_key=peer['client_key_ed25519'],initial_remote_path='/')
        config.write_text(json.dumps(dict(version=1,config=dict(servers=[profile],groups=[],trusted_hosts=[dict(host=host,port=int(port),fingerprint=peer['host_key_rsa_fingerprint'])],local_directory=str(local)))))
        with socket.socket() as listener: listener.bind(('127.0.0.1',0));debug=listener.getsockname()[1]
        endpoint=f'http://127.0.0.1:{debug}'
        env=os.environ.copy();env['WEBVIEW2_USER_DATA_FOLDER']=str(output/('webview-'+str(time.time_ns())))
        env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']=f'--remote-debugging-port={debug} --remote-debugging-address=127.0.0.1'
        metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=ROOT))
        app=subprocess.Popen([str(Path(metadata['target_directory'])/'debug/arcnix.exe'),'--config',str(config)],cwd=ROOT,env=env,stdout=log,stderr=log)
        deadline=time.monotonic()+20
        while True:
            try: urllib.request.urlopen(endpoint+'/json/version',timeout=1).close();break
            except OSError:
                assert app.poll() is None and time.monotonic()<deadline;time.sleep(.1)
        with sync_playwright() as pw:
            browser=pw.chromium.connect_over_cdp(endpoint);page=browser.contexts[0].pages[0];page.set_default_timeout(10000)
            page.set_viewport_size(dict(width=1440,height=980))
            try:
                page.locator('.host-select').first.click()
                page.locator('.overview-connect').click()
                page.get_by_role('dialog').get_by_role('button',name='Connect',exact=True).click()
                expect(page.locator('.connection-state')).to_have_text('SSH connected')
                local_pane=page.locator('.local-slot');remote_pane=page.locator('.remote-slot')
                def upload():
                    local_pane.get_by_role('button',name=filename,exact=True).click()
                    local_pane.get_by_role('button',name='Upload',exact=True).click()
                def running(direction):
                    row=page.locator('.queue-row').last
                    expect(row.locator('.job-name')).to_contain_text(direction)
                    expect(row.locator('.job-transfer-details')).to_contain_text(re.compile(r'[1-9][0-9.]* (?:B|KiB|MiB)/s'))
                    value=float(row.get_by_role('progressbar').get_attribute('aria-valuenow'))
                    assert 0<value<100,value
                    assert row.locator('.job-progress').bounding_box()['height']<=5
                    expect(row.locator('.job-transfer-details')).to_contain_text('Remaining ')
                    assert 'Remaining —' not in row.locator('.job-transfer-details').inner_text()
                    page.screenshot(path=str(output/(('upload' if direction=='↑' else 'download')+'-running.png')))
                    expect(row.locator('.job-status')).to_have_text('Completed',timeout=20000)
                    expect(row.get_by_role('progressbar')).to_have_attribute('aria-valuenow','100')
                    expect(row.locator('.job-transfer-details')).to_contain_text('Average ')
                    expect(row.locator('.job-transfer-details')).to_contain_text('Transferred 6.0 MiB')
                    return row
                upload();running('↑');checks.append('Upload displays an actual advancing progress bar, throughput, elapsed time and finite ETA; completion keeps average throughput and payload bytes')
                path.unlink()
                expect(remote_pane.get_by_role('button',name=filename,exact=True)).to_be_visible()
                remote_pane.get_by_role('button',name=filename,exact=True).click()
                remote_pane.get_by_role('button',name='Download',exact=True).click()
                running('↓')
                assert hashlib.sha256(path.read_bytes()).digest()==hashlib.sha256(content).digest()
                checks.append('Download displays independent progress and throughput and matches the original file SHA256')
                upload()
                dialog=page.get_by_role('dialog')
                expect(dialog).to_be_visible()
                row=page.locator('.queue-row').last
                expect(row.locator('.job-status')).to_have_text('Waiting for a file conflict decision')
                expect(row.locator('.job-transfer-details')).to_contain_text('Remaining —')
                page.screenshot(path=str(output/'conflict-wait.png'))
                dialog.get_by_role('button',name='Skip file',exact=True).click()
                expect(row.locator('.job-status')).to_contain_text('Skipped')
                expect(row.get_by_role('progressbar')).to_have_attribute('aria-valuenow','100')
                expect(row.locator('.job-transfer-details')).to_contain_text('Transferred 0 B')
                expect(row.locator('.job-transfer-details')).to_contain_text('Average 0 B/s')
                checks.append('Conflict wait hides throughput/ETA, and skipping completes logical progress with zero transmitted bytes')
                remote_path=Path(peer['root'])/filename
                assert remote_path.parent.resolve()==Path(peer['root']).resolve()
                remote_path.unlink()
                upload()
                row=page.locator('.queue-row').last
                expect(row.locator('.job-transfer-details')).to_contain_text(re.compile(r'[1-9][0-9.]* (?:B|KiB|MiB)/s'))
                row.get_by_role('button',name='Cancel transfer',exact=True).click()
                expect(row.locator('.job-status')).to_have_text('Canceled')
                assert float(row.get_by_role('progressbar').get_attribute('aria-valuenow'))<100
                snapshot=row.locator('.job-transfer-details').inner_text()
                page.wait_for_timeout(1100)
                expect(row.locator('.job-transfer-details')).to_have_text(snapshot)
                row.get_by_role('button',name='Retry transfer',exact=True).click()
                running('↑')
                checks.append('Cancel freezes partial statistics and retry starts a new attempt which transfers the full payload')
                page.set_viewport_size(dict(width=1000,height=700));page.screenshot(path=str(output/'queue-narrow.png'))
                assert page.evaluate('document.documentElement.scrollWidth<=innerWidth')
                checks.append('Compact progress rows fit a narrow workspace')
                (output/'results.json').write_text(json.dumps(dict(passed=len(checks),checks=checks),indent=2))
                print(json.dumps(dict(passed=len(checks),checks=checks)),flush=True)
            except Exception:
                try:
                    page.screenshot(path=str(output/'failure.png'))
                    (output/'failure.txt').write_text(page.locator('body').inner_text(),encoding='utf-8')
                except Exception: pass
                raise
            finally: browser.close()
    finally:
        if app and app.poll() is None: app.terminate();app.wait(timeout=10)
        fixture.terminate();fixture.wait(timeout=10);log.close()
if __name__=='__main__':main()
