"""Driver installation UI; the privileged action is always intercepted."""
from pathlib import Path
import shutil
import subprocess
from native_images import prepare_f101


def run(ui, temporary):
    # Exercise the shipped script in a temporary rules directory. Replace the host
    # reload with the installed udev parser so tests never change host USB access.
    rules = Path(temporary) / 'rules'
    rules.mkdir()
    script = (Path(__file__).resolve().parents[2]/'src-tauri/src/imager_usb_rules.sh').read_text()
    script = script.replace('/etc/udev/rules.d/', str(rules)+'/').replace('udevadm control --reload-rules','true')
    for vendor in ['allwinner', 'rockchip', 'kendryte', 'nxp', 'st']:
        info = ui.web.ipc('imager_driver_info', dict(vendor=vendor))
        assert info['platform'] == 'linux' and info['ids']
        assert all(len(pair) == 9 and pair[4] == ':' for pair in info['ids'])
        command = ['/bin/sh', '-c', script, 'lynx-imager', vendor, *info['ids']]
        for _ in range(2):
            subprocess.run(command, check=True)
        target = rules / ('70-lynx-imager-'+vendor+'.rules')
        lines = target.read_text().splitlines()
        assert target.stat().st_mode & 0o777 == 0o644
        assert len(lines) == len(info['ids'])
        for pair,line in zip(info['ids'],lines):
            vid,pid=pair.split(':')
            assert f'ATTR{{idVendor}}=="{vid}", ATTR{{idProduct}}=="{pid}", TAG+="uaccess"' in line
        if shutil.which('udevadm'):
            subprocess.run(['udevadm','verify',str(target)],check=True)
    prepare_f101(ui, temporary)
    ui.usb('none')
    ui.next()
    ui.text('未发现设备')
    ui.click('.vi-bar button','安装驱动')
    ui.text('1f3a:efe8')
    ui.save('driver-linux')
    assert not ui.js("return window.__imagerSimulation.calls.some(c => c.cmd==='imager_install_driver')")
    ui.click('button','安装 USB 访问规则')
    ui.text('驱动安装需要模拟系统响应')
    ui.fixture('imager_install_driver','规则已安装，请重新插拔开发板。')
    ui.usb('allwinner')
    ui.click('button','安装 USB 访问规则')
    ui.text('规则已安装，请重新插拔开发板。')
    ui.wait(lambda: ui.js("return document.querySelectorAll('.vi-device').length===1"),'rescan after driver setup')
    assert not ui.js("return !!document.querySelector('.vi-device input:checked')")
    prepare_f101(ui, temporary)
    ui.usb('none')
    ui.next()
    ui.text('未发现设备')
    ui.fixture('imager_driver_info',dict(platform='windows',ids=['1f3a:efe8']))
    ui.click('.vi-bar button','安装驱动')
    ui.text('打开 WinUSB 安装器')
    ui.save('driver-windows')
    ui.fixture('imager_install_driver','安装器已关闭，正在重新检测下载口。')
    ui.click('button','打开 WinUSB 安装器')
    ui.text('安装器已关闭，正在重新检测下载口。')
    assert ui.js("return window.__imagerSimulation.calls.filter(x=>x.cmd==='imager_install_driver').every(x=>x.args.vendor==='allwinner')")

    ui.reset()
    ui.board('rk3576', 'RK3576 EVB')
    if not ui.js("return !!document.querySelector('.vendor-switch')"):
        ui.next()
    package = Path(temporary) / 'rk-driver-test.img'
    package.write_bytes(b'RKFW' + bytes(1020))
    ui.pick(str(package), '选择文件')
    ui.next()
    ui.fixture('imager_driver_info', dict(platform='windows', ids=['2207:350e']))
    ui.click('.vi-bar button', '安装驱动')
    ui.text('瑞芯微 Rockusb 驱动')
    assert not ui.js("return document.querySelector('.vi-help').innerText.includes('WinUSB')")
    ui.save('driver-windows-rockusb')
    ui.fixture('imager_install_driver', 'Rockusb 驱动包已添加')
    ui.pick('C:/Driver Assistant/rockusb.inf', '选择 Rockusb 驱动…')
    ui.text('Rockusb 驱动包已添加')
    assert ui.js("return window.__imagerSimulation.calls.some(x=>x.cmd==='imager_install_driver' && x.args.vendor==='rockchip' && x.args.infPath==='C:/Driver Assistant/rockusb.inf')")
