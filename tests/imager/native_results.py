"""Exercise the shipping write UI using an isolated GTK WebView and simulated flash telemetry."""
import json
import tempfile
from pathlib import Path

from native_driver import wait
from native_images import prepare_f101


def confirm_screen(ui, fixtures):
    prepare_f101(ui, fixtures)
    ui.usb('allwinner')
    ui.next()
    ui.selectUsb()
    ui.next()
    ui.text('确认烧录')
    wait(lambda: ui.js("return !!document.querySelector('.v2w-drawer[open]')"), 'confirmation drawer')
    form = '.v2w-drawer'
    assert ui.js("return document.querySelector('.v2w-drawer .v2w-primary').disabled")
    digest = ui.js("return document.querySelector('.v2w-drawer .v2w-fingerprint code').textContent")
    assert len(digest) == 64
    ui.click(form + ' input[type=checkbox]')
    assert not ui.js("return document.querySelector('.v2w-drawer .v2w-primary').disabled")
    return digest


def start(ui):
    ui.click('.v2w-drawer .v2w-primary')
    wait(lambda: ui.js("return !!document.querySelector('.v2w-running')"), 'task submitted')


def enter(ui, selector, value):
    ui.js("""const el=document.querySelector(arguments[0]);
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,arguments[1]);
      el.dispatchEvent(new Event('input',{bubbles:true}));
      """, selector, value)


def run(ui, fixtures):
    digest = confirm_screen(ui, fixtures)
    ui.save('flash-confirm')
    # Invalidate the chosen target at the final read-only check, before starting a task.
    ui.usb('none')
    ui.click('.v2w-drawer .v2w-primary')
    ui.text('提交前检查未通过')
    ui.text('未提交烧录任务')
    ui.text('预检时失去目标设备')
    for value in ('simulated-f101-system.img', 'board-ram-loader.img', '1f3a:efe8', 'location:libusb:1:10.2'):
        ui.text(value)
    assert ui.js("""return document.querySelectorAll('.v2w-preflight-error button').length===1 &&
        !document.querySelector('#v2-preflight, .vi-bar') &&
        !window.__imagerSimulation.calls.some(c=>c.cmd==='workbench_start_flash')""")
    ui.save('flash-preflight-offline')
    ui.click('.v2w-preflight-error button','重新检测设备')
    ui.text('未发现设备')
    ui.usb('allwinner')
    ui.click('.vi-bar button','重新检测')
    ui.selectUsb()
    ui.text('核对已连接的下载设备')
    assert ui.js("return !document.querySelector('#v2-preflight').disabled")

    digest = confirm_screen(ui, fixtures)
    start(ui)
    active = {'phase': 'writing', 'storageType': 'spinor', 'writeState': 'running',
              'progressPct': 37, 'totalBytes': 614400, 'writtenBytes': 131072,
              'transferredBytes': 131072, 'bootVerifyState': 'not_verified'}
    ui.task(active, status='running', logs=['RAW_WRITE_START:medium=spinor,physicalStart=0'])
    ui.text('37%')
    ui.text('RAW_WRITE_START')
    ui.save('flash-active')
    ui.click('.v2w-logs label input')
    with tempfile.TemporaryDirectory(prefix='imager-write-tests-') as tmp:
        log_path = Path(tmp) / 'flash-log.txt'
        ui.fixture('plugin:dialog|save', str(log_path))
        ui.click('.v2w-logs button','导出日志')
        wait(log_path.exists,'native log export')
        assert 'RAW_WRITE_START' in log_path.read_text()
        ui.click('.v2w-logs button','清空')
        ui.text('视图已清空')
        verify = 'RAW_SPINOR_PHYSICAL_VERIFY_SUCCESS:imageSha256=' + digest + ',imageBytes=614400,fullFlashBytes=16777216,outsideImage=unchanged'
        success = dict(active, phase='spinor_awaiting_boot', progressPct=100, writeState='success',
                       verifyState='success', physicalVerifyState='success', physicalImageSha256=digest,
                       outsideImageState='unchanged', postFlashAction='reboot', postFlashState='success')
        ui.task(success, status='success', logs=[verify])
        ui.text('烧写完成，启动待验证')
        ui.text('16.0 MiB')
        ui.save('flash-await-boot')
        ui.click('.v2w-boot-form summary')
        marker = 'F101-FLASH-PROBE-SIM'
        serial = marker + ' HEARTBEAT 1 uptime_ms=5018\n' + marker + ' HEARTBEAT 2 uptime_ms=10023'
        ui.js("""const dt=new DataTransfer();dt.items.add(new File([arguments[0]],'coldboot.log',{type:'text/plain'}));
          const el=document.querySelector('.v2w-boot-form input[type=file]');el.files=dt.files;
          el.dispatchEvent(new Event('change',{bubbles:true}));""", serial)
        enter(ui, '.v2w-boot-form input[placeholder]', 'SOME-OTHER-BUILD')
        ui.text('串口输出中未找到该标识')
        assert ui.js("return document.querySelector('.v2w-boot-form button').disabled")
        enter(ui, '.v2w-boot-form input[placeholder]', marker)
        ui.text('匹配 2 行')
        enter(ui, '.v2w-boot-form input[maxlength="80"]','Ubuntu 模拟验收')
        ui.click('.v2w-boot-form label:nth-last-of-type(2) input')
        ui.click('.v2w-boot-form label:last-of-type input')
        ui.click('.v2w-boot-form button','保存本次验收')
        ui.text('独立上电验收通过')
        ui.text('写入完成，启动已验证')
        ui.save('flash-success')
        ui.fixture('workbench_list_workflow_steps', [])
        ui.click('.v2w-success-actions button','查看任务报告')
        ui.text('人工断电上电验收')
        ui.click('.imager-records footer button','关闭')
        ui.click('.v2w-success-actions button','烧录另一块板')

        confirm_screen(ui, fixtures)
        start(ui)
        policy = ['RAW_SPINOR_CONTRACT:capacity=16777216,eraseAlignment=65536,writeBytes=655360',
                  'RAW_SPINOR_EVIDENCE:simulated-backup',
                  'RAW_SPINOR_FES_STEP:erase_policy_download:start,bytes=16,attempt=1',
                  'RAW_SPINOR_ERASE_POLICY_DOWNLOAD_FAILED:elapsedMs=0,source=UsbTransfer']
        ui.task({'phase':'erase', 'storageType':'spinor', 'postFlashAction':'reboot','postFlashState':'not_run',
                 'diagnosticErrorCode':'RAW_SPINOR_ERASE_POLICY_DOWNLOAD_FAILED'},status='failed',logs=policy)
        ui.text('擦除策略下发失败')
        ui.text('策略请求 16 B')
        ui.text('物理备份已完成')
        ui.save('flash-policy-transfer')
        ui.click('.v2w-logs button','展开任务日志')
        ui.click('.v2w-logs button','收起日志')
        diag = Path(tmp) / 'diag.json'
        ui.fixture('plugin:dialog|save', str(diag))
        ui.click('.v2w-failure-actions button','导出诊断记录')
        ui.text('已导出 diag.json')
        data = json.loads(diag.read_text())
        assert data['status'] == 'failed' and any('UsbTransfer' in l for l in data['logTail'])
        assert data['parameters']['imageSha256'] == digest
        ui.click('.v2w-failure-actions button','重新检测设备')
        assert not ui.js("return !!document.querySelector('.v2w-running')")

        confirm_screen(ui, fixtures)
        start(ui)
        ui.task(dict(active, phase='spinor_physical_verify', physicalVerifyState='failed', verifyState='failed',
                     writeState='success', postFlashAction='reboot', postFlashState='not_run',
                     diagnosticErrorCode='RAW_SPINOR_PHYSICAL_VERIFY_FAILED'),
                status='failed',logs=['RAW_WRITE_COMPLETE:medium=spinor',
                                     'RAW_SPINOR_PHYSICAL_VERIFY_FAILED:SPINOR_PHYSICAL_CONTENT_MISMATCH: first byte Some(524288)'])
        ui.text('物理回读与预期不一致')
        ui.text('后端已确认镜像写入完成')
        ui.text('未执行')
        ui.save('flash-mismatch')
        ui.click('.v2w-failure-actions button','重新检测设备')

        confirm_screen(ui, fixtures)
        ui.js("""Object.defineProperty(window.__imagerSimulation.responses,'workbench_start_flash',{
          get() { throw new Error('模拟提交后的连接中断'); }, configurable:true
        });""")
        ui.click('.v2w-drawer .v2w-primary')
        ui.text('提交未取得任务编号，请先在任务记录核对设备状态')
        assert ui.js("""return !document.querySelector('.v2w-preflight-error') &&
           window.__imagerSimulation.calls.filter(c=>c.cmd==='workbench_start_flash').length===1""")
        ui.save('flash-submission-unknown')
