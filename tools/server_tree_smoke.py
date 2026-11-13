from pathlib import Path
import re,argparse,json,os,socket,subprocess,sys,time,urllib.request
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'artifacts/python'))
from playwright.sync_api import sync_playwright,expect

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--exe',type=Path);args=parser.parse_args()
    meta=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=ROOT))
    exe=args.exe.resolve() if args.exe else Path(meta['target_directory'])/'debug/arcnix.exe'
    out=ROOT/'artifacts/server-tree';out.mkdir(exist_ok=True,parents=True)
    config=out/'config.json'
    def profile(id,name,group):return dict(id=id,name=name,group_id=group,host='127.0.0.1',port=22,username='fixture',auth='Password',initial_remote_path='/')
    config.write_text(json.dumps(dict(version=1,config=dict(groups=[dict(id='a',name='Production'),dict(id='b',name='Staging'),dict(id='empty',name='Empty')],servers=[profile('one','API server','a'),profile('two','Web server','b'),profile('three','Local server',None)],trusted_hosts=[]))),encoding='utf-8')
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    env=os.environ.copy();env['WEBVIEW2_USER_DATA_FOLDER']=str(out/('webview-'+str(time.time_ns())));env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']=f'--remote-debugging-port={port} --remote-debugging-address=127.0.0.1'
    log=(out/'runtime.log').open('w',encoding='utf-8');process=subprocess.Popen([str(exe),'--config',str(config)],cwd=ROOT,env=env,stdout=log,stderr=log)
    try:
        deadline=time.monotonic()+20
        while True:
            try:urllib.request.urlopen(f'http://127.0.0.1:{port}/json/version',timeout=1).close();break
            except OSError:
                if process.poll() is not None or time.monotonic()>deadline:raise RuntimeError((out/'runtime.log').read_text())
                time.sleep(.1)
        with sync_playwright() as pw:
            browser=pw.chromium.connect_over_cdp(f'http://127.0.0.1:{port}');page=browser.contexts[0].pages[0];page.set_default_timeout(8000);page.set_viewport_size(dict(width=1440,height=980));errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
            tree=page.locator('.server-tree');folders=page.locator('.tree-folder');prod=folders.filter(has=page.get_by_role('button',name='Production',exact=True));stage=folders.filter(has=page.get_by_role('button',name='Staging',exact=True))
            expect(folders).to_have_count(3);expect(tree.locator('.host-row')).to_have_count(3)
            expect(prod.locator('.tree-chevron')).to_have_text('')
            assert 'data:image/svg+xml' in prod.locator('.tree-chevron').evaluate("el=>getComputedStyle(el).maskImage")
            geometry=prod.locator('.group-button').evaluate("el=>[...el.children].filter(x=>x.getClientRects().length).map(x=>{const r=x.getBoundingClientRect();return {x:r.x,y:r.y,w:r.width,h:r.height}})")
            assert abs((geometry[0]['y']+geometry[0]['h']/2)-(geometry[1]['y']+geometry[1]['h']/2))<1
            assert abs(geometry[0]['w']-18)<1 and abs(geometry[1]['w']-22)<1,geometry
            empty_folder=folders.filter(has=page.get_by_role('button',name='Empty',exact=True))
            expect(prod.locator('.group-button')).to_have_attribute('draggable','true')
            prod.locator('.group-button').drag_to(empty_folder.locator('.group-button'),target_position=dict(x=60,y=36))
            expect(tree.locator('.tree-group-id')).to_have_text(['b','empty','a'])
            assert [g['id'] for g in json.loads(config.read_text())['config']['groups']]==['b','empty','a']
            # A second real app reads the saved ordering without sharing any UI state.
            with socket.socket() as sock:sock.bind(('127.0.0.1',0));reload_port=sock.getsockname()[1]
            reload_env=env.copy();reload_env['WEBVIEW2_USER_DATA_FOLDER']=str(out/('reload-'+str(time.time_ns())));reload_env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS']=f'--remote-debugging-port={reload_port} --remote-debugging-address=127.0.0.1'
            reload_process=subprocess.Popen([str(exe),'--config',str(config)],cwd=ROOT,env=reload_env,stdout=log,stderr=log)
            try:
                deadline=time.monotonic()+20
                while True:
                    try:urllib.request.urlopen(f'http://127.0.0.1:{reload_port}/json/version',timeout=1).close();break
                    except OSError:
                        if reload_process.poll() is not None or time.monotonic()>deadline:raise RuntimeError('Reload failed')
                        time.sleep(.1)
                reload_browser=pw.chromium.connect_over_cdp(f'http://127.0.0.1:{reload_port}');reload_page=reload_browser.contexts[0].pages[0]
                expect(reload_page.locator('.tree-group-id')).to_have_text(['b','empty','a']);reload_browser.close()
            finally:reload_process.terminate();reload_process.wait(timeout=10)
            prod.locator('.group-button').drag_to(stage.locator('.group-button'),target_position=dict(x=60,y=2))
            expect(tree.locator('.tree-group-id')).to_have_text(['a','b','empty'])

            expect(prod.locator('.host-name')).to_have_text('API server');expect(stage.locator('.host-name')).to_have_text('Web server')
            expect(tree.locator(':scope > .host-row .host-name')).to_have_text('Local server')
            prod.locator('.group-button').click();expect(prod.locator('.host-row')).to_have_count(0)
            stage.locator('.group-button').click();expect(stage.locator('.host-row')).to_have_count(0)
            prod.locator('.group-button').click();expect(prod.locator('.host-row')).to_have_count(1);expect(stage.locator('.host-row')).to_have_count(0)
            prod.locator('.group-button').hover();expect(prod.locator('.group-more')).to_have_css('opacity','1')
            prod.locator('.group-more').click();menu=page.locator('.context-popover');expect(menu).to_be_visible();labels=menu.locator('button').all_text_contents();page.keyboard.press('Escape');expect(menu).to_have_count(0);expect(menu).to_have_count(0)
            prod.locator('.group-button').click(button='right');expect(menu).to_be_visible();assert menu.locator('button').all_text_contents()==labels
            box=menu.bounding_box();assert box['y']>=0 and box['x']>=0 and box['y']+box['height']<=980
            page.screenshot(path=str(out/'group-menu-dark.png'));menu.get_by_role('button',name='New server',exact=True).click()
            dialog=page.get_by_role('dialog');expect(dialog.get_by_role('button',name='Production',exact=True)).to_have_class(re.compile(r'choice active'))
            dialog.get_by_role('textbox',name='Name',exact=True).fill('New child');dialog.get_by_role('textbox',name='Host address',exact=True).fill('127.0.0.1');dialog.get_by_role('textbox',name='Username',exact=True).fill('fixture');dialog.get_by_role('button',name='Save server',exact=True).click()
            expect(dialog).to_have_count(0);expect(prod.locator('.host-row')).to_have_count(2)
            assert json.loads(config.read_text())['config']['servers'][-1]['group_id']=='a'
            host=prod.locator('.host-row').first;host.locator('.host-more').click();expect(menu).to_be_visible();host_labels=menu.locator('button').all_text_contents();page.keyboard.press('Escape');expect(menu).to_have_count(0);host.locator('.host-select').click(button='right');expect(menu).to_be_visible();assert menu.locator('button').all_text_contents()==host_labels
            menu.get_by_role('button',name='Edit server',exact=True).click();dialog=page.get_by_role('dialog');dialog.get_by_role('button',name='Staging',exact=True).click();dialog.get_by_role('button',name='Save server',exact=True).click();expect(dialog).to_have_count(0)
            expect(stage.locator('.host-row')).to_have_count(2)
            prod.locator('.group-button').click(button='right');menu.get_by_role('button',name='Rename group',exact=True).click();dialog=page.get_by_role('dialog');dialog.get_by_role('textbox',name='Group name',exact=True).fill('Production renamed');dialog.get_by_role('button',name='Save group',exact=True).click();expect(dialog).to_have_count(0)
            assert json.loads(config.read_text())['config']['groups'][0]['name']=='Production renamed'
            page.locator('.tree-more').click();menu.get_by_role('button',name='Collapse all',exact=True).click();expect(tree.locator('.tree-children')).to_have_count(0)
            page.get_by_role('textbox',name='Search',exact=True).fill('API');expect(tree.locator('.host-name')).to_have_text('API server');expect(tree.locator('.tree-folder-name')).to_have_text('Staging')
            page.get_by_role('textbox',name='Search',exact=True).fill('Staging');expect(tree.locator('.host-row')).to_have_count(2)
            page.get_by_role('textbox',name='Search',exact=True).fill('absent');expect(page.get_by_text('No matching servers or groups',exact=True)).to_be_visible()
            page.get_by_role('textbox',name='Search',exact=True).fill('');expect(tree.locator('.tree-children')).to_have_count(0)
            tree.click(button='right',position=dict(x=60,y=300));expect(menu).to_have_class('context-popover tree-root-menu pointer-menu');menu.get_by_role('button',name='Expand all',exact=True).click();expect(tree.locator('.tree-children')).to_have_count(3)
            page.locator('.tree-new-group').click();dialog=page.get_by_role('dialog');dialog.get_by_role('textbox',name='Group name',exact=True).fill('New folder');dialog.get_by_role('button',name='Save group',exact=True).click();expect(dialog).to_have_count(0)
            folder=folders.filter(has=page.get_by_role('button',name='New folder',exact=True));folder.locator('.group-button').focus();page.keyboard.press('ArrowLeft');expect(folder.locator('.tree-children')).to_have_count(0);page.keyboard.press('ArrowRight');expect(folder.locator('.tree-children')).to_have_count(1)
            page.keyboard.press('Shift+F10');expect(menu).to_be_visible();expect(menu.locator('button').first).to_be_focused();page.keyboard.press('ArrowDown');expect(menu.locator('button').nth(1)).to_be_focused();page.keyboard.press('Escape')
            # Context menus can switch targets even while the common dismiss scrim is active.
            stage.locator('.group-button').click(button='right');page.locator('.host-row').last.click(button='right',force=True);expect(page.locator('.host-row.menu-open')).to_have_count(1);page.keyboard.press('Escape')
            page.locator('.settings-link').click();page.locator('.context-sidebar').get_by_role('button',name='Appearance',exact=True).click();page.get_by_role('button',name='Light',exact=True).click();page.locator('.rail-servers').click();page.screenshot(path=str(out/'tree-light.png'))
            page.set_viewport_size(dict(width=900,height=700));page.locator('.group-more').last.click();box=menu.bounding_box();assert box['y']>=0 and box['y']+box['height']<=700;page.screenshot(path=str(out/'tree-narrow-menu.png'))
            page.keyboard.press('Escape');expect(menu).to_have_count(0);page.set_viewport_size(dict(width=1440,height=980))
            stage.locator('.group-more').click();menu.get_by_role('button',name='Dissolve group (keep servers)',exact=True).click()
            dialog=page.get_by_role('dialog');expect(dialog).to_contain_text('Staging');dialog.locator('.modal-footer button').last.click();expect(dialog).to_have_count(0)
            saved=json.loads(config.read_text())['config'];assert len(saved['servers'])==4
            assert all(p['group_id'] is None for p in saved['servers'] if p['id'] in ('one','two'))
            expect(tree.locator(':scope > .host-row')).to_have_count(3)
            page.locator('.tree-more').click();menu.get_by_role('button',name='New server',exact=True).click()
            dialog=page.get_by_role('dialog');expect(dialog.get_by_role('button',name='Ungrouped',exact=True)).to_have_class(re.compile(r'choice active'));page.keyboard.press('Escape')

            renamed=folders.filter(has=page.get_by_role('button',name='Production renamed',exact=True))
            renamed.locator('.host-select').click()
            renamed.locator('.group-more').click();menu.get_by_role('button',name='Delete group & servers',exact=True).click()
            dialog=page.get_by_role('dialog');expect(dialog).to_contain_text('New child');expect(dialog).to_contain_text('No remote servers or files will be deleted.')
            before=config.read_bytes();page.keyboard.press('Escape');expect(dialog).to_have_count(0);assert config.read_bytes()==before
            renamed.locator('.group-button').click(button='right');menu.get_by_role('button',name='Delete group & servers',exact=True).click();dialog.locator('.modal-footer button').last.click();expect(dialog).to_have_count(0)
            saved=json.loads(config.read_text())['config'];assert len(saved['servers'])==3 and all(g['id']!='a' for g in saved['groups'])
            expect(page.get_by_text('Choose a server',exact=True)).to_be_visible()
            empty_folder.locator('.group-button').click(button='right');menu.get_by_role('button',name='Delete group & servers',exact=True).click();expect(dialog).to_contain_text('0 saved server profiles');dialog.locator('.modal-footer button').last.click();expect(dialog).to_have_count(0)
            assert len(json.loads(config.read_text())['config']['servers'])==3
            assert not errors,errors
            print('Passed: SVG alignment; native drag ordering/reload; dissolve/purge/cancel; tree nesting; independent/global collapse; inherited server creation; group rename; server reassignment; search; matching right-click/More menus; keyboard navigation; theme and menu bounds.')
            browser.close()
    finally:
        if process.poll() is None:process.terminate();process.wait(timeout=10)
        log.close()
if __name__=='__main__':main()
