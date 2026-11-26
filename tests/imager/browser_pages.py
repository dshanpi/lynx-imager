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


def test_emmc_families(page, results, capture):
    next_step = page.locator('.v2-foot-actions .v2-primary')
    for vendor, expected_count in [('全志', 46), ('瑞芯微', 30)]:
        page.locator('.target-vendors').get_by_role('button', name=vendor, exact=True).click()
        families = page.locator('.target-families button')
        checked = set()
        for index in range(families.count()):
            families.nth(index).click()
            rows = page.locator('.target-models [data-model]')
            names = rows.evaluate_all("els => els.map(el => el.dataset.model)")
            if vendor == '瑞芯微' and index == 0:
                assert names[0] == 'RK3576', names
            if index == 0:
                capture('allwinner-emmc-models' if vendor == '全志' else 'rockchip-emmc-models')
            for model in names:
                row = page.locator(f'[data-model="{model}"]')
                media = row.locator('.target-model-media')
                expect(media if media.count() else page.locator('.target-series-spec .target-model-media')).to_contain_text('eMMC')
                row.click()
                expect(next_step).to_be_disabled()
                page.locator('[data-medium="eMMC"]').click()
                next_step.click()
                summary = page.locator('.image-target-summary')
                expect(summary).to_contain_text(model)
                expect(summary).to_contain_text('eMMC')
                if vendor == '全志':
                    expect(page.locator('input[value="mainline_emmc"]')).to_be_checked()
                    expect(page.locator('input[value="vendor"]')).to_be_enabled()
                else:
                    expect(page.locator('.vendor-switch')).to_contain_text('主线 eMMC 整盘')
                page.locator('#v2-step-1').click()
                page.get_by_role('button', name='返回型号列表').click()
                checked.add(model)
        assert len(checked) == expected_count, (vendor, checked)
        results.append(f'{vendor} {len(checked)} 款型号可手选 eMMC 并进入对应镜像路线')


def test_target_matrix(page, results):
    expected = {
        'T113': ('SPI NAND', 'eMMC'), 'T153': ('eMMC',), 'F101': ('SPI NOR', 'eMMC'),
        'RK3576': ('eMMC',), 'RK3588': ('eMMC',), 'RK3588S': ('eMMC',),
        'i.MX6ULL': ('eMMC', 'SD'), 'i.MX8': ('eMMC', 'SD'), 'i.MX9': ('eMMC', 'SD'),
        'STM32MP1': ('eMMC', 'SD'), 'STM32MP157': ('eMMC', 'SD'),
        'K230': ('SPI NOR', 'SPI NAND', 'eMMC', 'SD'),
    }
    next_step = page.locator('.v2-foot-actions .v2-primary')
    blocked = []
    expected['RK3568'] = ('eMMC',)
    for chip, supported in expected.items():
        supported = tuple(set(supported) | {'SD'})
        page.get_by_role('searchbox', name='搜索芯片型号或板名').fill(chip)
        page.locator(f'[data-model="{chip}"]').click()
        expect(next_step).to_be_disabled()
        for medium in ('SPI NOR', 'SPI NAND', 'eMMC', 'SD'):
            option = page.locator(f'[data-medium="{medium}"]')
            if medium not in supported:
                expect(option).to_be_disabled()
                continue
            option.click()
            if next_step.is_disabled():
                blocked.append((chip, medium))
                continue
            next_step.click()
            summary = page.locator('.image-target-summary')
            expect(summary).to_contain_text(medium)
            expect(summary.locator('strong')).to_contain_text(chip)
            if page.locator('.card-writer').count():
                reader_next = page.get_by_role('button', name='下一步：选择读卡器')
                if chip == 'RK3576':
                    expect(page.locator('.card-writer')).to_contain_text('card.img.xz')
                    expect(reader_next).to_be_enabled()
                else:
                    expect(reader_next).to_be_disabled()
            else:
                expect(next_step).to_be_disabled()
            page.locator('#v2-step-1').click()
            expect(option).to_have_attribute('aria-pressed', 'true')
    assert not blocked, f'已选择受支持介质却不能继续：{blocked}'
    results.append(f'{len(expected)} 个芯片、{sum(map(len, expected.values()))} 种可用组合：板名留空进入选镜像，返回后目标一致，其余介质禁用')


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

    # Passive detection, manual override, stand-alone erase and direct SD write.
    page.evaluate("pageFixture.attached=[{vendor:'rockchip',key:'1:2',pid:0x350e,loader:true}]")
    expect(button('瑞芯微')).to_have_attribute('aria-pressed','true')
    button('全志').click()
    page.wait_for_timeout(2100)
    expect(button('全志')).to_have_attribute('aria-pressed','true')
    button('瑞芯微').click()
    page.locator('[data-model="RK3576"]').click()
    page.locator('[data-medium="eMMC"]').click()
    dlg=page.get_by_role('dialog')
    page.evaluate("pageFixture.attached.push({vendor:'rockchip',key:'1:3',pid:0x350e,loader:false})")
    button('擦除 eMMC…').click()
    expect(dlg.locator('.emmc-port').filter(has_text='USB 1:3')).to_be_disabled()
    capture('emmc-erase-devices')
    port=dlg.locator('.emmc-port').filter(has_text='USB 1:2')
    port.click()
    expect(dlg).to_contain_text('8,589,934,592')
    dlg.get_by_role('button',name='更换设备').click()
    port.click()
    dlg.get_by_role('button',name='取消').click()
    expect(dlg).to_have_count(0)
    assert 'imager_emmc_cancel' in page.evaluate('pageFixture.calls')
    button('擦除 eMMC…').click()
    port.click()
    expect(dlg.locator('.emmc-device')).to_contain_text('USB 1:2')
    capture('emmc-erase-confirm')
    rect=dlg.bounding_box()
    viewport=page.viewport_size
    for position,size in [('x','width'),('y','height')]:
        assert abs(rect[position]*2+rect[size]-viewport[size])<3, rect
    assert 'imager_emmc_erase' not in page.evaluate('pageFixture.calls')
    page.evaluate('pageFixture.holdErase=true')
    dlg.get_by_role('button',name='确认擦除').click()
    expect(dlg.get_by_role('progressbar')).to_be_visible()
    page.keyboard.press('Escape')
    expect(dlg.get_by_role('button',name='关闭')).to_be_disabled()
    page.evaluate('pageFixture.finishErase()')
    expect(dlg).to_contain_text('擦除完成')
    capture('emmc-erase-done')
    dlg.get_by_role('button',name='关闭').click()
    page.evaluate('pageFixture.holdErase=false; pageFixture.failErase=true')
    button('擦除 eMMC…').click()
    port.click()
    dlg.get_by_role('button',name='确认擦除').click()
    expect(dlg.get_by_role('alert')).to_contain_text('连接中断')
    dlg.get_by_role('button',name='重新检测').click()
    expect(port).to_be_visible()
    dlg.get_by_role('button',name='关闭').click()
    page.evaluate('pageFixture.attached.pop(); pageFixture.failErase=false')
    page.locator('[data-medium="SD"]').click()
    page.locator('.v2-foot-actions .v2-primary').click()
    page.evaluate("pageFixture.nextFile='C:/qa/card.img.xz'")
    button('选择镜像').click()
    expect(page.locator('.card-image-slot')).to_contain_text('card.img.xz')
    expect(page.locator('.v2-status-facts')).to_contain_text('电脑读卡器')
    button('上一步').click()
    expect(page.locator('[data-medium="SD"]')).to_have_attribute('aria-pressed','true')
    page.locator('.v2-foot-actions .v2-primary').click()
    expect(page.locator('.card-image-slot')).to_contain_text('card.img.xz')
    capture('reader-image')
    button('下一步：选择读卡器').click()
    page.locator('.reader-row input').check()
    capture('reader-selected')
    button('下一步：确认写卡').click()
    page.evaluate('pageFixture.readerDisconnected=true')
    page.locator('#v2-step-3').click()
    expect(page.locator('.reader-empty')).to_contain_text('未发现可写')
    expect(button('下一步：确认写卡')).to_be_disabled()
    page.evaluate('pageFixture.readerScanError=true')
    button('重新检测').click()
    expect(page.locator('.reader-empty')).to_contain_text('存储服务不可用')
    page.evaluate('pageFixture.readerScanError=false; pageFixture.readerDisconnected=false')
    button('重新检测').click()
    expect(page.locator('.reader-row input')).not_to_be_checked()
    page.locator('.reader-row input').check()
    button('下一步：确认写卡').click()
    capture('reader-before-write')
    expect(page.locator('.card-destination')).to_contain_text('QA Reader')
    assert 'imager_card_write' not in page.evaluate('pageFixture.calls')
    page.evaluate('pageFixture.holdCardWrite=true; pageFixture.failCardWrite=true')
    button('擦写所选卡').click()
    expect(page.locator('.card-writer h2')).to_have_text('正在写卡与校验')
    expect(button('上一步')).to_be_disabled()
    expect(button('擦写所选卡')).to_be_disabled()
    capture('reader-writing')
    page.evaluate('pageFixture.finishCardWrite()')
    expect(page.locator('.card-writer [role="alert"]')).to_contain_text('连接中断')
    expect(button('擦写所选卡')).to_have_count(0)
    expect(page.locator('.card-writer')).not_to_contain_text('SHA-256 与源镜像一致')
    capture('reader-failed')
    page.evaluate("pageFixture.readerName='USB-SD-Reader-'+'very-long-device-name-'.repeat(12)")
    button('重新检测存储卡').click()
    expect(page.locator('.reader-row input')).not_to_be_checked()
    expect(button('下一步：确认写卡')).to_be_disabled()
    expect(page.locator('.reader-row strong')).to_contain_text('very-long-device-name-')
    capture('reader-long-name')
    page.evaluate('pageFixture.holdCardWrite=false; pageFixture.failCardWrite=false; pageFixture.readerName=null')
    button('重新检测').click()
    page.locator('.reader-row input').check()
    button('下一步：确认写卡').click()
    button('擦写所选卡').click()
    expect(page.locator('.card-writer')).to_contain_text('已回读写入范围')
    capture('reader-verified')
    page.locator('#v2-step-1').click()
    page.evaluate("pageFixture.attached=[]")
    results.append('只读厂商识别、手动切换、RK Loader 独立擦除和 TF 读卡器写后校验（模拟 IPC）')
    test_emmc_families(page, results, capture)
    test_target_matrix(page, results)
    # Reproduce the RC3 picker path without entering an EVB or board alias.
    next_step = page.locator('.v2-foot-actions .v2-primary')
    page.locator('.target-vendors').get_by_role('button', name='瑞芯微').click()
    page.locator('.target-families').get_by_role('button', name='RK35 系列').click()
    page.locator('[data-model="RK3576"]').click()
    expect(next_step).to_be_disabled()
    for medium in ('SPI NOR', 'SPI NAND'):
        expect(page.locator(f'[data-medium="{medium}"]')).to_be_disabled()
    page.locator('[data-medium="eMMC"]').click()
    expect(page.get_by_role('combobox', name='具体板型', exact=True)).to_have_value('')
    expect(next_step).to_be_enabled()
    capture('rk3576-emmc-ready')
    next_step.click()
    expect(page.locator('.v2-basic')).to_have_attribute('data-step', '2')
    expect(next_step).to_be_disabled()
    page.locator('.vendor-switch').get_by_role('button', name='主线 eMMC 整盘').click()
    page.evaluate("pageFixture.nextFile='C:/qa/disk.img'")
    button('选择文件').click()
    expect(next_step).to_be_disabled()
    page.evaluate("pageFixture.nextFile=pageFixture.unreadableFile='C:/qa/missing-loader.bin'")
    page.get_by_role('button', name='选择 Loader').click()
    expect(page.get_by_role('alert')).to_contain_text('Loader 读取失败')
    expect(next_step).to_be_disabled()
    page.evaluate("pageFixture.nextFile='C:/qa/loader.bin'")
    page.get_by_role('button', name='更换 Loader').click()
    expect(next_step).to_be_enabled()
    page.locator('.vendor-switch').get_by_role('button', name='厂商 update.img').click()
    page.evaluate("pageFixture.nextFile='C:/qa/rockchip-update.img'")
    button('选择文件').click()
    expect(next_step).to_be_enabled()
    page.evaluate("""() => {
        const invoke = window.__TAURI_INTERNALS__.invoke;
        pageFixture.failUsbWatch = true;
        window.__TAURI_INTERNALS__.invoke = (cmd,args,...rest) => {
            if (pageFixture.failUsbWatch && cmd === 'plugin:event|listen' && args.event === 'usb-hotplug')
                return Promise.reject('QA hotplug unavailable');
            return invoke(cmd,args,...rest);
        };
        pageFixture.usb=[{key:'1:2',flasherKind:'openrockcli',vid:0x2207,pid:0x3576,bus:1,
                          port:'2',mode:'loader',chip:'RK3576',dfuAlternates:[]}];
    }""")
    next_step.click()
    expect(page.locator('.v2-basic')).to_have_attribute('data-step', '3')
    page.locator('.vi-device').click()
    alert = page.get_by_role('alert').filter(has_text='无法监听设备连接变化')
    expect(alert).to_be_visible()
    expect(next_step).to_be_disabled()
    page.evaluate('pageFixture.failUsbWatch=false')
    alert.get_by_role('button', name='重新检测设备').click()
    page.locator('.vi-device').click()
    expect(next_step).to_be_enabled()
    results.append('USB 监听失败后选择设备仍显示原因；重新检测可恢复预检')
    results.append('RK3576：板型留空，选择 eMMC 后进入选镜像；主线整盘需要 Loader，厂商包可进入设备连接')
    page.reload()

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
            page.evaluate("pageFixture.nextFile='C:/qa/zephyr.bin'")
            button('选择镜像…').click()
            expect(page.get_by_role('alert')).to_contain_text('文件开头缺少 eGON.BT0')
            expect(page.get_by_text('必选 · 请选择含 FES 和 U-Boot 的板卡配套引导包。')).to_be_visible()
            expect(page.get_by_text('连续写入 SPI NOR',exact=False)).to_have_count(0)
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            assemble=button('选择启动来源并合成…')
            page.evaluate('pageFixture.nextFile=null')
            assemble.click()
            expect(assemble).to_be_enabled()
            page.evaluate("pageFixture.nextFile='C:/qa/loader.img'")
            assemble.click()
            expect(page.get_by_text('未找到此 F101 XIP 配置的启动段。')).to_be_visible()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            page.evaluate("pageFixture.nextFile='C:/qa/xip-boot_spi.bin'")
            assemble.click()
            expect(page.locator('.imager-image-line').first).to_contain_text('system.img')
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            for name, action in [('loader.img','选择引导包…'),
                                 ('BUILD-INFO.json','选择参考清单…')]:
                page.evaluate('file => pageFixture.nextFile=file','C:/qa/'+name)
                button(action).click()
                if name == 'loader.img':
                    expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
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
            expect(page.locator('.imager-image-review input[type=checkbox]')).to_have_count(0)
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
            page.locator('.v2c-tabs').get_by_role('button',name='最近使用').click()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            page.locator('.v2c-tabs').get_by_role('button',name='本地文件').click()
            expect(button('更换参考清单…')).to_be_visible()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
            page.locator('input[name="aw-system-route"][value="vendor"]').check()
            expect(button('选择镜像…')).to_be_visible()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            page.locator('input[name="aw-system-route"][value="mainline_spinor"]').check()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
            page.locator('.v2c-tabs').get_by_role('button',name='最近使用').click()
            page.locator('.v2c-recent-row').filter(has_text='system.img').click()
            expect(page.locator('.image-desk-loader,.imager-image-line').filter(has_text='loader.img')).to_be_visible()
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
            results.append('F101 厂商与主线路线独立恢复；手动点最近镜像同时恢复并复核该条目的 RAM Loader')
            page.reload()
            page.locator('.target-vendors').get_by_role('button',name=vendor).click()
            page.locator('.target-families').get_by_role('button',name=family+' 系列').click()
            page.locator('[data-model="F101"]').click()
            page.locator('[data-medium="SPI NOR"]').click()
            page.locator('.v2-foot-actions .v2-primary').click()
            expect(page.locator('body')).to_contain_text('system.img')
            expect(page.locator('body')).to_contain_text('loader.img')
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_enabled()
            assert 'plugin:dialog|open' not in page.evaluate('pageFixture.calls')
            page.evaluate("""pageFixture.usb=[3,4].map(port=>({
               key:'location:pciroot(0)#pci(1400)#usbroot(0)#usb(9)#usb('+port+')',
               flasherKind:'openixcli',vid:0x1f3a,pid:0xefe8,bus:1,port:'9.'+port,
               mode:'fel/fes',chip:'Allwinner',dfuAlternates:[]}))""")
            page.locator('.v2-foot-actions .v2-primary').click()
            rows=page.locator('.vi-device')
            expect(rows).to_have_count(2)
            for i in (1,0):
                rows.nth(i).click()
                expect(rows.nth(i).get_by_role('radio')).to_be_checked()
                expect(page.locator('.vi-device input:checked')).to_have_count(1)
                expect(page.locator('#v2-preflight')).to_be_enabled()
                expect(page.locator('.r3-connect-facts')).to_contain_text('USB 端口 9.'+str(i+3))
            expect(page.locator('.vi-port-path code')).to_be_hidden()
            capture('windows-f101-selected')
            page.locator('.vi-bar').get_by_role('button',name='重新检测').click()
            expect(page.locator('.vi-device input:checked')).to_have_count(0)
            expect(page.locator('#v2-preflight')).to_be_disabled()
            assert 'workbench_save_device' not in page.evaluate('pageFixture.calls')
            page.locator('.v2-foot-actions').get_by_role('button',name='上一步').click()
            results.append('F101 两个同型号 USB 设备按端口单选，重检后清除选择')
            results.append('重开 F101 后自动恢复镜像与 RAM Loader，重新校验后可直接进入设备连接')
            results.append('F101 主线 SPI NOR：镜像分析、清单布局、RAM FES 组件及返回焦点')
        else:
            routes=page.locator('.vendor-switch button')
            expect(routes).to_have_count(2)
            for i in (1,0):
                routes.nth(i).click()
                expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
                capture(model+'-route-'+str(i+1))
            if model == 'STM32MP1':
                page.evaluate("pageFixture.fileAlternates=[0,2]; pageFixture.nextFile='C:/qa/multi.dfu'")
                button('选择文件').click()
                expect(page.get_by_role('alert')).to_contain_text('一个明确的 DFU Alternate')
                expect(next_step).to_be_disabled()
                page.evaluate("pageFixture.fileAlternates=[2]; pageFixture.nextFile='C:/qa/single.dfu'")
                button('更换文件').click()
                expect(next_step).to_be_enabled()
                page.evaluate("""pageFixture.usb=[{key:'1:3',flasherKind:'dfu-util',vid:0x0483,pid:0xdf11,
                    bus:1,port:'3',mode:'dfu',chip:'STM32MP1',dfuAlternates:[
                    {number:0,name:'boot'},{number:2,name:'root'}]}]""")
                next_step.click()
                page.locator('.vi-device').click()
                alternate = page.locator('.vendor-field select')
                alternate.select_option('0')
                expect(page.get_by_role('status').filter(has_text='与容器内目标不一致')).to_be_visible()
                expect(next_step).to_be_disabled()
                alternate.select_option('2')
                expect(next_step).to_be_enabled()
                page.locator('#v2-step-2').click()
                results.append('STM32 多目标镜像及接口不匹配均显示原因，更换镜像并选对接口后恢复')
            if model == 'i.MX8':
                routes.nth(1).click()
                page.evaluate("""pageFixture.nextFile='C:/qa/disk.wic';
                    pageFixture.usb=[{key:'1:3',flasherKind:'uuu',vid:0x1fc9,pid:1,bus:1,
                    port:'3',mode:'sdp',chip:'i.MX8',dfuAlternates:[]}]""")
                button('选择文件').click()
                next_step.click()
                page.locator('.vi-device').click()
                expect(next_step).to_be_disabled()
                expect(page.get_by_role('status').filter(has_text='UUU 内置 -b 配方')).to_be_visible()
                page.locator('#v2-step-2').click()
                routes.nth(0).click()
                page.evaluate("pageFixture.nextFile='C:/qa/flash.uuu'")
                button('选择文件').click()
                next_step.click()
                page.locator('.vi-device').click()
                expect(next_step).to_be_enabled()
                page.locator('#v2-step-2').click()
                results.append('NXP 缺保存配方时提示所需参数，改用独立 UUU 脚本可以预检')
            if model == 'K230':
                routes.nth(1).click()
                page.evaluate("""pageFixture.nextFile='C:/qa/k230.img';
                    pageFixture.usb=[3,4].map(p=>({key:'1:'+p,flasherKind:'k230',vid:0x29f1,pid:1,
                    bus:1,port:String(p),mode:'loader',chip:'K230',dfuAlternates:[]}))""")
                button('选择文件').click()
                next_step.click()
                expect(page.locator('.vi-device')).to_have_count(2)
                for row in page.locator('.vi-device').all():
                    row.click()
                    expect(next_step).to_be_disabled()
                    page.locator('.vendor-field select').select_option('sdcard')
                    expect(next_step).to_be_enabled()
                expect(page.locator('.vi-device').nth(1).get_by_role('radio')).to_be_checked()
                page.locator('#v2-step-2').click()
                results.append('K230 两台设备依次选择，同一介质的确认只对当前端口生效')
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
    available=backend.locator('option').evaluate_all('(options)=>options.map(o=>o.value)')
    changed=next((value for value in available if value != original),original)
    backend.select_option(changed)
    page.evaluate('pageFixture.backendChanges=[]')
    page.evaluate('pageFixture.failSettings=true')
    save()
    expect(page.get_by_test_id('settings-save-error')).to_contain_text('test settings write failed')
    assert page.evaluate('pageFixture.backendChanges')==([changed,original] if changed != original else [])
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
                  dict(offline_board_fields(), id=7, boardUid='qa-offline', serialPort='COM77'))
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
    serial = page.locator('.device-console-view')
    expect(serial.get_by_role('button',name='连接串口')).to_be_enabled()
    # Cover the old automatic three-second reclaim loop.
    page.wait_for_timeout(3300)
    background = set(page.evaluate('pageFixture.calls'))
    assert not background.intersection({'serial_wait_and_open','ch347_list_devices',
        'workbench_scan_flasher_devices','gateway_start','workbench_start_tunnel'}), background
    serial.get_by_role('button',name='连接串口').click()
    expect(serial.get_by_role('button',name='断开串口')).to_be_enabled()
    assert page.evaluate('pageFixture.openPort') == 'COM77'
    serial.get_by_role('button',name='断开串口').click()
    expect(serial.get_by_role('button',name='连接串口')).to_be_enabled()
    page.wait_for_timeout(3300)
    assert page.evaluate("!pageFixture.openPort && pageFixture.calls.filter(x=>x==='serial_wait_and_open').length===1")
    results.append('浏览高级功能和串口页保持空闲；显式连接、断开后不再抢占端口')
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
    cli.add_argument('--height',type=int,default=768)
    cli.add_argument('--platform',choices=['macos','windows'],default='macos')
    cli.add_argument('--theme-screens',action='store_true')
    args=cli.parse_args()
    tests=Path(__file__).resolve().parent
    mocks=(tests.parents[1]/'node_modules/@tauri-apps/api/mocks.js').read_text(encoding='utf8').split('export {')[0]
    fixtures=(tests/'page-fixtures.js').read_text(encoding='utf8')
    report={'runtime':'Chromium, in-memory IPC','platform':args.platform,'width':args.width,
            'height':args.height,'checks':[], 'pageErrors':[],'screens':[], 'geometry':[]}
    args.output.mkdir(parents=True,exist_ok=True)
    with ThreadingHTTPServer(('127.0.0.1',0),partial(SimpleHTTPRequestHandler,directory=args.dist)) as server, sync_playwright() as pw:
        Thread(target=server.serve_forever,daemon=True).start()
        url=f'http://127.0.0.1:{server.server_port}'
        browser=pw.chromium.launch(headless=True,executable_path=args.executable)
        agent=('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)' if args.platform=='macos'
               else 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)')
        page=browser.new_page(locale='zh-CN',user_agent=agent,
                              viewport={'width':args.width,'height':args.height})
        page.set_default_timeout(6000)
        page.add_init_script(script='(()=>{'+mocks+'\n'+fixtures+'})()')
        page.on('pageerror',lambda e:report['pageErrors'].append(str(e)))
        page.on('dialog',lambda d:d.dismiss())
        page.route('**/*',lambda route:route.continue_() if route.request.url.startswith(url+'/') else route.abort())
        def capture(name):
            expect(page.locator('.target-picker img, .vendor-context img, .v2w-board img, .v2c-chip-package')).to_have_count(0)
            if name in report['screens']:
                return
            overflow=page.evaluate('document.documentElement.scrollWidth - document.documentElement.clientWidth')
            assert overflow <= 1, (name, 'horizontal overflow', overflow)
            geometry=page.evaluate('''() => {
                const brand=document.querySelector('.imager-brand strong');
                const rect=brand.getBoundingClientRect();
                const header=document.querySelector('.imager-app-header').getBoundingClientRect();
                const actions=[...document.querySelectorAll('.v2-foot-actions button,.card-writer footer button,dialog[open] footer button')]
                    .filter(e=>e.getClientRects().length).map(e=>{
                      const r=e.getBoundingClientRect();
                      return {text:e.textContent.trim(),x:r.x,y:r.y,right:r.right,bottom:r.bottom};
                    });
                const rail=[...document.querySelectorAll('.v2-rail,.board-list,.im-ai-rail')].find(e=>e.getClientRects().length);
                return {brandHeight:rect.height,headerHeight:header.height,railWidth:rail?.getBoundingClientRect().width,actions};
            }''')
            for action in geometry['actions']:
                assert action['x'] >= -1 and action['right'] <= args.width+1, (name,action)
                assert action['y'] >= 0 and action['bottom'] <= args.height+1, (name,action)
            report['geometry'].append({'page':name,**geometry})
            folder=args.output/'screens'
            folder.mkdir(exist_ok=True)
            page.screenshot(path=folder/(name+'.png'),animations='disabled')
            if name.startswith('emmc-erase-'):
                page.get_by_role('dialog').screenshot(path=folder/(name+'-detail.png'),animations='disabled')
            if args.theme_screens:
                themes=page.evaluate('''() => [...document.querySelectorAll('[data-theme]')].map(e=>{
                    const theme=e.dataset.theme;
                    e.dataset.theme=theme==='lynx-light'?'lynx-dark':theme==='light'?'dark':theme;
                    return theme;
                })''')
                page.screenshot(path=folder/(name+'-dark.png'),animations='disabled')
                page.evaluate('themes => [...document.querySelectorAll("[data-theme]")].forEach((e,i)=>e.dataset.theme=themes[i])',themes)
            report['screens'].append(name)
        try:
            page.goto(url)
            page.locator('.imager-app-tabs button').first.click()
            page.keyboard.press('Meta+k' if args.platform=='macos' else 'Control+k')
            expect(page.get_by_role('searchbox',name='搜索芯片型号或板名')).to_be_focused()
            report['checks'].append('当前平台搜索快捷键将焦点移至可见目标搜索框')
            expect(page.locator('.v2-foot-actions .v2-primary')).to_be_disabled()
            expect(page.locator('[data-medium][aria-pressed=true]')).to_have_count(0)
            capture('target-home')
            scope=page.get_by_role('combobox',name='搜索范围')
            scope.select_option('series')
            search=page.get_by_role('searchbox',name='筛选当前系列型号')
            search.fill('RK3576')
            expect(page.locator('[data-model]')).to_have_count(0)
            search.press('Meta+k' if args.platform=='macos' else 'Control+k')
            expect(scope).to_have_value('all')
            expect(page.locator('[data-model="RK3576"]')).to_be_visible()
            page.reload()

            test_pages(page,report['checks'],capture)
            assert not report['pageErrors'], report['pageErrors']
            shells=[row for row in report['geometry'] if row['page'] in ('target-home','device-overview')]
            assert len(shells)==2 and all(abs(shells[0][k]-shells[1][k])<=1 for k in ('headerHeight','brandHeight','railWidth')), shells
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
