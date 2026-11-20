"""Supplementary UI regression with isolated in-memory IPC. Requires Playwright and a built frontend."""
import argparse
from datetime import datetime, timezone
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
from threading import Thread

from playwright.sync_api import expect, sync_playwright
from native_smoke import offline_board_fields


def test_write(page, outcome, results, capture):
    """Drive the normal vendor flow; file inspection, USB, and progress are fixtures."""
    next_step = page.locator('.v2-foot-actions .v2-primary')
    page.locator('.target-vendors').get_by_role('button', name='瑞芯微').click()
    page.locator('.target-families').get_by_role('button', name='RK35 系列').click()
    page.locator('[data-model="RK3588"]').click()
    page.locator('[data-medium="eMMC"]').click()
    page.locator('.v2c-select-model select').select_option(label='RK3588-EVB1')
    next_step.click()
    page.evaluate("""() => {
        pageFixture.nextFile='C:/qa/rockchip-update.img';
        pageFixture.devices=[{id:7,boardUid:'qa-rk',name:'QA-RK3588',chipVendor:'rockchip',
          usbBinding:'1:10.2',flasherKind:'openrockcli',flasherArgs:'[]',serialPort:'',baudRate:115200}];
    }""")
    if outcome == 'success':
        page.get_by_role('button',name='选择文件',exact=True).click()
    else:
        expect(page.get_by_role('button',name='更换文件',exact=True)).to_be_visible()
        expect(page.locator('body')).to_contain_text('rockchip-update.img')
        assert 'plugin:dialog|open' not in page.evaluate('pageFixture.calls')
        results.append('重开同一 RK3588 配置后自动恢复镜像，无需再次选文件')
    expect(next_step).to_be_enabled()
    next_step.click()
    expect(page.locator('#v2-preflight')).to_be_disabled()
    page.evaluate("""pageFixture.usb=[{key:'1:10.2',flasherKind:'openrockcli',vid:0x2207,
        pid:0x350b,bus:1,port:'10.2',mode:'loader',chip:'RK3588',dfuAlternates:[]}]""")
    page.locator('.vi-bar').get_by_role('button',name='重新检测').click()
    page.locator('.vi-device').click()
    capture('device-connection')
    next_step.click()
    drawer=page.locator('.v2w-drawer[open]')
    expect(drawer.locator('.v2w-primary')).to_be_disabled()
    capture('flash-confirm')
    drawer.locator('input[type=checkbox]').check()
    drawer.locator('.v2w-primary').click()
    expect(page.locator('.v2w-running')).to_be_visible()
    page.evaluate("""Object.assign(pageFixture.tasks[0],{phase:'writing',progressPct:37,
      details:{phase:'writing',writeState:'running',progressPct:37,outputTail:['QA_WRITE']}})""")
    expect(page.locator('.v2w-running')).to_contain_text('37%')
    capture('flash-running')
    page.evaluate("""status => Object.assign(pageFixture.tasks[0],{status,phase:'finished',
        finishedAt:150,progressPct:100,details:{writeState:'success',verifyState:status,
        bootVerifyState:'not_verified',outputTail:['QA_END']}})""", outcome)
    if outcome=='success':
        expect(page.locator('.v2w-success-actions')).to_be_visible()
        expect(page.get_by_text('烧写完成，启动待验证',exact=True)).to_be_visible()
        capture('flash-success')
        page.locator('.v2w-success-actions').get_by_role('button',name='查看任务报告').click()
        expect(page.locator('.imager-records')).to_contain_text('qa-flash')
        page.keyboard.press('Escape')
    else:
        expect(page.locator('.v2w-failure-actions')).to_be_visible()
        capture('flash-failure')
        page.locator('.v2w-failure-actions').get_by_role('button',name='重新检测设备').click()
        expect(page.locator('.v2-basic')).to_have_attribute('data-step','3')
    assert page.evaluate("pageFixture.calls.filter(c=>c==='workbench_start_flash').length")==1
    results.append('RK 厂商包：指纹、设备重检、预检、确认、写入进度与'+
                   ('完成后打开报告' if outcome=='success' else '失败后返回检测'))


def test_pages(page, results, capture):
    def tab(label):
        page.locator('.imager-app-tabs').get_by_role('button', name=label, exact=True).click()

    def ai(label):
        page.locator('.im-ai-rail').get_by_role('button', name=label, exact=True).click()

    def button(label):
        return page.get_by_role('button', name=label, exact=True)

    def settings():
        page.locator('.imager-app-header [aria-label="设置"]').click()
        expect(page.get_by_test_id('settings-language')).to_be_visible()

    def save():
        page.locator('.settings-footer .settings-btn-primary').click()

    # Select actual model and medium for each vendor and return through the wizard.
    for vendor,family,model,medium in [
            ('全志','F','F101','SPI NOR'),('瑞芯微','RK35','RK3588','eMMC'),
            ('恩智浦','i.MX','i.MX8','eMMC'),('意法半导体','STM32MP','STM32MP1','eMMC'),
            ('嘉楠','Kendryte','K230','SD')]:
        page.locator('.target-vendors').get_by_role('button',name=vendor).click()
        page.locator('.target-families').get_by_role('button',name=family+' 系列').click()
        page.locator(f'[data-model="{model}"]').click()
        page.locator(f'[data-medium="{medium}"]').click()
        capture('target-'+model)
        choose=page.locator('.v2c-select-model input,.v2c-select-model select')
        if choose.count():
            if choose.evaluate("e=>e.tagName")== 'SELECT':
                choose.select_option(index=1)
            else:
                choose.fill('QA test board')
        page.locator('.v2-foot-actions .v2-primary').click()
        expect(page.locator('.v2-basic')).to_have_attribute('data-step','2')
        capture('system-'+model)
        sources=['最近使用','本地文件']
        if model=='F101':
            sources.insert(0,'板型资源')
        for source in sources:
            page.locator('.v2c-tabs').get_by_role('button',name=source).click()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
        if model=='F101':
            page.locator('input[name="aw-system-route"][value="mainline_spinor"]').check()
            for name, action in [('system.img','选择镜像…'),('loader.img','选择引导包…'),
                                 ('BUILD-INFO.json','选择参考清单…')]:
                page.evaluate('file => pageFixture.nextFile=file','C:/qa/'+name)
                button(action).click()
                if name == 'loader.img':
                    assert page.evaluate('pageFixture.lastDialog.defaultPath') == 'C:/qa/'
            for trigger, panel in [('分析系统镜像','image'),('检查引导组件','loader')]:
                button(trigger).click()
                inspector=page.locator('.imager-review-page')
                expect(inspector).to_have_attribute('data-inspector',panel)
                expect(inspector.locator('h2')).to_be_focused()
                if panel=='loader':
                    expect(inspector).to_contain_text('fes1.fex')
                else:
                    expect(inspector).to_contain_text('启动与预留区')
                capture('f101-'+panel)
                page.keyboard.press('Escape')
                expect(button(trigger)).to_be_focused()
            page.locator('.imager-image-confirm input').check()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
            page.reload()
            page.locator('.target-vendors').get_by_role('button',name=vendor).click()
            page.locator('.target-families').get_by_role('button',name=family+' 系列').click()
            page.locator('[data-model="F101"]').click()
            page.locator('[data-medium="SPI NOR"]').click()
            page.locator('.v2-foot-actions .v2-primary').click()
            expect(page.locator('body')).to_contain_text('system.img')
            expect(page.locator('body')).to_contain_text('loader.img')
            expect(page.locator('.imager-image-confirm input')).not_to_be_checked()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            assert 'plugin:dialog|open' not in page.evaluate('pageFixture.calls')
            results.append('重开 F101 后自动恢复镜像与 RAM Loader，重新校验并等待确认')
            results.append('F101 主线 SPI NOR：镜像分析、清单布局、RAM FES 组件及返回焦点')
        else:
            routes=page.locator('.vendor-switch button')
            expect(routes).to_have_count(2)
            for i in (1,0):
                routes.nth(i).click()
                expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
                capture(model+'-route-'+str(i+1))
        page.locator('.v2-foot-actions').get_by_role('button',name='上一步',exact=False).click()
        results.append(vendor+'：目标、介质、'+'、'.join(sources)+'与返回')

    test_write(page, 'success', results, capture)
    page.reload()
    test_write(page, 'failed', results, capture)
    page.reload()
    page.evaluate("window.pageFixture.calls=[]")
    settings()
    page.get_by_test_id('settings-theme-mode').select_option('dark')
    expect(page.locator('.imager-themed')).to_have_attribute('data-theme','dark')
    page.keyboard.press('Escape')
    expect(page.locator('.imager-themed')).to_have_attribute('data-theme','light')
    expect(page.locator('.imager-app-header [aria-label="设置"]')).to_be_focused()
    settings()
    page.get_by_test_id('settings-theme-mode').select_option('dark')
    page.evaluate("window.pageFixture.calls=[]")
    save()
    expect(page.get_by_test_id('settings-save-success')).to_be_visible()
    assert not set(page.evaluate('pageFixture.calls')) & {
        'cloud_sync_set_enabled','diagnostics_set_enabled','efex_set_usb_backend'}
    page.get_by_test_id('settings-close').click()
    page.reload()
    settings()
    expect(page.get_by_test_id('settings-theme-mode')).to_have_value('dark')
    page.get_by_test_id('settings-theme-mode').select_option('light')
    backend=page.get_by_role('combobox',name='USB 驱动',exact=True)
    original=backend.input_value()
    changed='winusb' if original=='libusb' else 'libusb'
    backend.select_option(changed)
    page.evaluate('pageFixture.backendChanges=[]')
    page.evaluate('pageFixture.failSettings=true')
    save()
    expect(page.get_by_test_id('settings-save-error')).to_contain_text('test settings write failed')
    assert page.evaluate('pageFixture.backendChanges')==[changed,original]
    page.evaluate('pageFixture.failSettings=false')
    page.evaluate('pageFixture.holdSave=true')
    save()
    expect(page.get_by_test_id('settings-close')).to_be_disabled()
    page.keyboard.press('Escape')
    expect(page.locator('.settings-content')).to_have_attribute('aria-busy','true')
    page.wait_for_function("typeof pageFixture.finishSave==='function'")
    page.evaluate('() => {pageFixture.holdSave=false;pageFixture.finishSave()}')
    expect(page.get_by_test_id('settings-save-success')).to_be_visible()
    page.get_by_test_id('settings-close').click()
    page.evaluate('pageFixture.failDiagnostics=true')
    settings()
    save()
    expect(page.get_by_test_id('settings-save-success')).to_be_visible()
    button('检查更新').click()
    expect(page.get_by_text('已是最新版本',exact=True)).to_be_visible()
    page.evaluate('pageFixture.updateError=true')
    button('检查更新').click()
    expect(page.locator('.settings-update-error')).to_contain_text('QA update offline')
    page.evaluate("""() => {
        pageFixture.updateError=false;
        pageFixture.update={rid:8,version:'9.0.0-test',body:'QA notes',rawJson:{strategy:'full'}};
    }""")
    button('检查更新').click()
    expect(page.locator('.settings-update-available')).to_contain_text('9.0.0-test')
    button('跳过此版本').click()
    button('检查更新').click()
    expect(button('跳过此版本')).to_be_disabled()
    page.evaluate("pageFixture.update.version='9.0.1-test'")
    button('检查更新').click()
    expect(button('跳过此版本')).to_be_enabled()
    capture('settings-update')
    page.get_by_test_id('settings-close').click()
    results.append('设置：预览、Esc 撤销与焦点、保存、重启恢复、写失败重试、诊断离线')
    results.append('软件更新：最新、离线、新版说明、跳过当前版本、发现更高版本')

    page.evaluate("""pageFixture.tasks=['completed','succeeded','failed'].map((status,i)=>({
        taskId:'qa-'+i,deviceId:1,kind:'test',startedAt:100+i,status,phase:'fixture'}))""")
    page.locator('.imager-app-header [aria-label="任务记录"]').click()
    dialog=page.locator('.imager-records')
    expect(dialog.locator('tbody tr')).to_have_count(3)
    result=dialog.get_by_label('结果')
    expect(result.locator('option')).to_have_count(3)
    result.select_option(label='完成')
    expect(dialog.locator('tbody tr')).to_have_count(2)
    dialog.locator('tbody tr').last.get_by_role('button').click()
    expect(dialog.locator('section code').first).to_have_text('qa-1')
    capture('task-records')
    page.evaluate("pageFixture.tasks=pageFixture.tasks.filter(row=>row.status==='failed')")
    dialog.get_by_role('button',name='刷新').click()
    expect(result).to_have_value('')
    expect(dialog.locator('tbody tr')).to_have_count(1)
    page.evaluate('pageFixture.failRecords=true')
    dialog.get_by_role('button',name='刷新').click()
    expect(dialog.get_by_role('alert')).to_be_visible()
    expect(dialog.get_by_role('button',name='导出选中记录')).to_be_disabled()
    page.evaluate('pageFixture.failRecords=false')
    dialog.get_by_role('button',name='刷新').click()
    expect(dialog.get_by_role('button',name='导出选中记录')).to_be_enabled()
    dialog.get_by_role('button',name='导出选中记录').click()
    page.wait_for_function("pageFixture.exported?.task?.taskId==='qa-2'")
    assert page.evaluate("pageFixture.exported.steps[0].stepName")=='qa-2'
    page.keyboard.press('Escape')
    results.append('任务记录：筛选、详情、刷新后选项失效、错误恢复、导出门禁')

    tab('高级功能')
    expect(page.get_by_label('搜索单板')).to_be_visible()
    button('添加设备').click()
    expect(page.get_by_test_id('linux-settings-dialog')).to_be_visible()
    capture('device-new')
    page.get_by_test_id('linux-settings-dialog').get_by_role('button',name='关闭',exact=True).click()
    page.evaluate('board => sessionStorage.qaBoard=JSON.stringify(board)',
                  dict(offline_board_fields(), id=7, boardUid='qa-offline'))
    page.reload()
    tab('高级功能')
    page.locator('.board-item-select').get_by_text('QA-OFFLINE-F101').click()
    page.get_by_test_id('linux-tab-overview').click()
    files=page.get_by_test_id('adb-file-transfer')
    for action in ('发送文件至设备','从设备接收','刷新目录'):
        expect(files.get_by_role('button',name=action,exact=True)).to_be_disabled()
    capture('device-overview')
    page.get_by_test_id('linux-tab-serial').click()
    expect(page.locator('.device-console-view').get_by_role('button',name='粘贴')).to_be_disabled()
    capture('device-serial')
    page.get_by_test_id('linux-tab-sdk').click()
    expect(page.get_by_test_id('linux-sdk-terminal').get_by_role('button',name='新建终端')).to_be_disabled()
    capture('device-ssh')
    page.get_by_test_id('linux-tab-flash').click()
    expect(page.locator('.lynx-device-tool-content').get_by_role('button',name='确认写入…')).to_be_disabled()
    capture('device-flash')
    page.get_by_test_id('linux-header-device-settings').click()
    expect(page.get_by_test_id('linux-board-name')).to_have_value('QA-OFFLINE-F101')
    page.get_by_test_id('linux-settings-dialog').get_by_role('button',name='关闭',exact=True).click()
    results.append('高级功能：设备列表、概览、ADB、串口、SSH、固件烧录离线状态及设备设置')

    page.evaluate("""() => {
      pageFixture.projects=[{id:3,deviceId:7,name:'QA SDK',mcpServerName:'qa-sdk',hostId:1,
        localGatewayPort:8770,remoteForwardPort:25756,sdkDir:'/sdk',aiProvider:'codex'}];
      pageFixture.channel={projectId:3,deviceId:7,running:true,verified:true,
         port:8770,endpoint:'http://127.0.0.1:8770/mcp'};
      pageFixture.tunnels=[{projectId:3,state:'connected',localPort:8770,remotePort:25756}];
    }""")
    tab('连接 AI')
    for label,title in [('本机 MCP','本机 MCP 通道'),('远端 SDK','远端 SDK 通道'),
                        ('请求确认','请求确认'),('通知','通知')]:
        ai(label)
        expect(page.locator('.im-ai-body h1')).to_have_text(title)
        if label=='本机 MCP':
            expect(button('复制地址')).to_be_enabled()
            expect(page.locator('.im-ai-body')).to_contain_text('http://127.0.0.1:8770/mcp')
            page.evaluate('pageFixture.channel.deviceId=99')
            button('检查服务').click()
            expect(button('复制地址')).to_be_disabled()
        if label=='远端 SDK':
            expect(button('复制远端地址')).to_be_enabled()
            expect(page.locator('.im-ai-body')).to_contain_text('http://127.0.0.1:25756/mcp')
            expect(button('复制本机地址')).to_be_disabled()
        capture('ai-'+label)
    expect(button('保存通知设置')).to_be_disabled()
    ai('AI 服务')
    name=page.get_by_label('连接名称',exact=True)
    name.fill('QA service')
    page.get_by_label('Base URL').fill('http://127.0.0.1:1/?wrong=1')
    button('保存连接').click()
    expect(page.locator('.im-ai-body')).to_contain_text('Base URL 应为服务地址')
    page.get_by_label('Base URL').fill('http://127.0.0.1:1')
    button('保存连接').click()
    button('测试连接').click()
    expect(page.locator('.im-ai-body')).to_contain_text('连接测试通过')
    capture('ai-provider')
    ai('策略与安全')
    page.get_by_label('策略名称').fill('QA policy')
    button('保存策略与权限').click()
    expect(page.locator('.im-ai-body')).to_contain_text('策略与动作权限已保存')
    capture('ai-policy')
    ai('通知')
    page.get_by_label('发送方式').select_option('none')
    button('保存通知设置').click()
    expect(page.locator('.im-ai-body')).to_contain_text('通知设置已保存')
    ai('AI 服务')
    expect(name).to_have_value('QA service')
    ai('通知')
    expect(page.get_by_label('发送方式')).to_have_value('none')
    page.evaluate("""() => {
        pageFixture.runs=[{id:'qa-run',boardId:'linux:7',taskId:'qa-task',policyId:1,status:'waiting_approval'}];
        pageFixture.rounds=[{id:1,runId:'qa-run',roundIndex:1,createdAt:100,redactedContext:{},
            decision:{decision:'collect_logs',parameters:{}},policyResult:'approval_required',actionResult:'not_executed'}];
    }""")
    ai('请求确认')
    approve=button('允许本次动作')
    expect(approve).to_be_disabled()
    page.locator('.im-ai-check input').check()
    expect(approve).to_be_enabled()
    page.evaluate("pageFixture.rounds[0].id=2")
    approve.click()
    expect(page.locator('.im-ai-body')).to_contain_text('请求已更新')
    expect(approve).to_be_disabled()
    button('拒绝请求并终止监督').click()
    expect(page.locator('.im-ai-body')).to_contain_text('当前设备没有需要确认的请求')
    assert 'supervisor_execute_action' not in page.evaluate('pageFixture.calls')
    results.append('AI 请求确认：核对前禁用、回合变化撤销授权、拒绝测试请求')
    results.append('高级功能新建与取消，AI 六页、服务地址校验、连接测试、策略与通知保存重开')
    button('使用帮助').click()
    assert page.evaluate('pageFixture.url')=='https://github.com/dshanpi/lynx-imager'
    tab('烧录系统')
    expect(page.locator('.v2-basic')).to_be_visible()
    results.append('使用帮助目标与跨工作区返回')


if __name__=='__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    cli.add_argument('dist',type=Path)
    cli.add_argument('output',type=Path)
    cli.add_argument('--executable')
    cli.add_argument('--width',type=int,default=1280)
    args=cli.parse_args()
    tests=Path(__file__).resolve().parent
    mocks=(tests.parents[1]/'node_modules/@tauri-apps/api/mocks.js').read_text(encoding='utf8').split('export {')[0]
    fixtures=(tests/'page-fixtures.js').read_text(encoding='utf8')
    report={'runtime':'Chromium, in-memory IPC','width':args.width,'checks':[], 'pageErrors':[],'screens':[]}
    args.output.mkdir(parents=True,exist_ok=True)
    with ThreadingHTTPServer(('127.0.0.1',0),partial(SimpleHTTPRequestHandler,directory=args.dist)) as server, sync_playwright() as pw:
        Thread(target=server.serve_forever,daemon=True).start()
        url=f'http://127.0.0.1:{server.server_port}'
        browser=pw.chromium.launch(headless=True,executable_path=args.executable)
        page=browser.new_page(locale='zh-CN',viewport={'width':args.width,'height':768})
        page.set_default_timeout(6000)
        page.add_init_script(script='(()=>{'+mocks+'\n'+fixtures+'})()')
        page.on('pageerror',lambda e:report['pageErrors'].append(str(e)))
        page.on('dialog',lambda d:d.dismiss())
        page.route('**/*',lambda route:route.continue_() if route.request.url.startswith(url+'/') else route.abort())
        def capture(name):
            if name in report['screens']:
                return
            overflow=page.evaluate('document.documentElement.scrollWidth - document.documentElement.clientWidth')
            assert overflow <= 1, (name, 'horizontal overflow', overflow)
            folder=args.output/'screens'
            folder.mkdir(exist_ok=True)
            page.screenshot(path=folder/(name+'.png'),animations='disabled')
            report['screens'].append(name)
        try:
            page.goto(url)
            test_pages(page,report['checks'],capture)
            assert not report['pageErrors'], report['pageErrors']
            report['passed']=True
            for old in ('failure.png','failure.txt'):
                (args.output/old).unlink(missing_ok=True)
        except Exception:
            page.screenshot(path=args.output/'failure.png')
            (args.output/'failure.txt').write_text(page.locator('body').inner_text(),encoding='utf8')
            report['passed']=False
            raise
        finally:
            report['finishedUtc']=datetime.now(timezone.utc).isoformat()
            (args.output/'pages.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf8')
            browser.close()
            server.shutdown()
