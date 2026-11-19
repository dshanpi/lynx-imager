"""Run from dbus-run-session. Usage: native_review.py SIMULATION_BINARY OUTPUT [suite ...]."""
import json
import sys
import time
import traceback
from pathlib import Path
from native_driver import launch,wait
import native_images,native_vendors,native_results


class Review:
    def __init__(self,web,out):
        self.web=web
        self.out=out
        self.shots=[]
        self.actions=[]
        self.wait=wait
        self.js=web.js

    def reset(self):
        previous=self.js('return performance.timeOrigin')
        self.js('location.reload()')
        wait(lambda:self.js("return performance.timeOrigin!==arguments[0] && !!window.__imagerSimulation && !!document.querySelector('.v2-basic')",previous),'new native view')
        self.fixture('workbench_list_workflow_steps',[])
        self.js("window.__imagerSimulation.sequence=Number(sessionStorage.getItem('simulation-sequence')||0)+1;sessionStorage.setItem('simulation-sequence',String(window.__imagerSimulation.sequence))")

    def click(self,selector,label=None):
        self.web.click(selector,label)
        self.actions.append(('click',label or selector))

    def board(self,id,model='',medium=None):
        vendor,family,chip,default = {
            't113-pro':('全志','T','T113','SPI NAND'), 'f101':('全志','F','F101','SPI NOR'),
            't153-emmc':('全志','T','T153','eMMC'), 'rk3588':('瑞芯微','RK35','RK3588','eMMC'),
            'nxp-imx':('恩智浦','i.MX','i.MX8','eMMC'), 'stm32mp1':('意法半导体','STM32MP','STM32MP1','eMMC'),
            'canaan-k230':('嘉楠','Kendryte','K230','SD'),
        }[id]
        self.click('.target-vendors button',vendor)
        self.click('.target-families button',family+' 系列')
        self.click(f'button[data-model="{chip}"]')
        assert self.js("return document.querySelector('.v2-foot-actions .v2-primary').disabled")
        self.click(f'button[data-medium="{medium or default}"]')
        if model:
            ok=self.js("""const e=document.querySelector('.v2c-select-model input,.v2c-select-model select');
                if(!e)return false;Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value').set.call(e,arguments[0]);
                e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));return e.value===arguments[0];""",model)
            assert ok,('model',model)
    def next(self):self.click('.v2-foot-actions .v2-primary')
    def text(self,t):return self.web.shown(t)
    def pick(self,path,label):
        self.js('window.__imagerSimulation.nextFile=arguments[0]',str(path))
        self.click('button',label)
        wait(lambda:self.js('return window.__imagerSimulation.nextFile===null'),'dialog file selection')
    def fixture(self,cmd,value):self.js('window.__imagerSimulation.responses[arguments[0]]=arguments[1]',cmd,value)
    def usb(self,vendor,mode='',count=1,bound=True):
        if vendor=='none':
            self.js('window.__imagerSimulation.usb=[]');return
        kind,default,vid,pid,chip={
            'rockchip':('openrockcli','loader',0x2207,0x350b,'RK3588'),
            'allwinner':('openixcli','fel/fes',0x1f3a,0xefe8,'Allwinner'),
            'st':('dfu-util','dfu',0x0483,0xdf11,'STM32 DFU'),
            'nxp':('uuu','sdp',0x1fc9,0x0134,'MX8M'),
            'kendryte':('k230','bootrom / loader',0x29f1,0x0230,'K230 / K230D'),
        }[vendor]
        devices=[];profiles=[]
        for i in range(count):
            bus=1
            # K230 serializes a single numeric port as text. Other CLIs can report a hub path.
            port=str(2+i) if vendor=='kendryte' else f'10.{2+i}'
            k=f'location:libusb:{bus}:{port}' if vendor=='allwinner' else f'{bus}:{port}'
            devices.append(dict(key=k,chip=chip,flasherKind=kind,mode=mode or default,vid=vid,pid=pid,bus=bus,port=port,dfuAlternates=[{'number':0,'name':'@FSBL'},{'number':3,'name':'@flash'}] if vendor=='st' else []))
            profiles.append(dict(id=i+1,boardUid=f'offline-sim-{vendor}-{i}',name=f'{vendor} · USB {port}',chipVendor=vendor,usbBinding=k,flasherKind=kind,flasherPath='',flasherArgs='["-b","emmc"]' if vendor=='nxp' else '[]',serialPort=''))
        self.js('window.__imagerSimulation.usb=arguments[0];window.__imagerSimulation.profiles=arguments[1]',devices,profiles if bound else [])
    def selectUsb(self):
        self.click('.vi-device')
    def events(self,name,payload):return self.web.ipc('plugin:event|emit',{'event':name,'payload':payload})
    def task(self,details,status='running',logs=None):
        logs=logs or []
        self.js("""const s=window.__imagerSimulation;
          if(!s.tasks.length)throw Error('no simulated task');
          Object.assign(s.tasks[0],{status:arguments[0],kind:'flash',phase:arguments[1].phase||'',
            finishedAt:arguments[0]==='running'?null:Math.floor(Date.now()/1000),progressPct:arguments[1].progressPct||0,details:{...arguments[1],outputTail:arguments[2]}});
          """,status,details,logs)
    def save(self,id):
        assert self.js("return location.origin==='tauri://localhost' && !!window.__imagerSimulation")
        layout=self.js("""const n=document.documentElement;
            const field=document.querySelector('.v2-control-area');
            const rail=document.querySelector('.v2-rail');
            const title=[...field.querySelectorAll('.target-hero h3, h2')]
                .find(e=>e.getClientRects().length);
            return {w:innerWidth,h:innerHeight,overflow:n.scrollWidth-n.clientWidth,
             railRight:rail.getBoundingClientRect().right, contentX:field.getBoundingClientRect().x,
             bodyFont:getComputedStyle(field).fontFamily,
             titleSize:title ? getComputedStyle(title).fontSize : null,
             step:document.querySelector('.v2-basic')?.dataset.step};""")
        assert layout['overflow']<=1,(id,layout)
        compact = layout['w'] <= 1180
        assert layout['railRight'] == (232 if compact else 264), (id,layout)
        assert layout['contentX'] == layout['railRight'] + (16 if compact else 24), (id,layout)
        if layout['titleSize'] is not None:
            assert layout['titleSize'] == '20px', (id,layout)
        self.web.shot(self.out/(id+'.png'))
        self.shots.append(dict(id=id,**layout))
        print('SCREEN',id,flush=True)


def connections(ui,fixtures):
    ui.reset()
    assert ui.js("return document.querySelector('.v2-primary').disabled")
    assert ui.js("return [2,3,4].every(n=>document.querySelector('#v2-step-'+n).disabled)")
    ui.js("document.querySelector('#v2-step-4').click()")
    assert ui.js("return document.querySelector('.v2-basic').dataset.step==='1'")
    ui.click('.target-vendors button','恩智浦')
    assert ui.js("return document.querySelectorAll('button[data-model]').length===2")
    ui.click('.target-vendors button','全志')
    ui.board('f101')
    ui.save('choose-board')
    native_images.prepare_f101(ui,fixtures)
    ui.usb('none')
    ui.next()
    ui.text('未发现设备')
    assert ui.js("return document.querySelector('#v2-preflight').disabled")
    ui.save('device-waiting')
    ui.usb('allwinner',count=2)
    ui.click('.vi-bar button','重新检测')
    ui.text('选定这次要写入的物理端口')
    assert ui.js("return document.querySelectorAll('.vi-device').length===2 && document.querySelector('#v2-preflight').disabled")
    ui.save('device-multiple')
    ui.selectUsb()
    ui.text('核对已连接的下载设备')
    ui.save('device-ready')
    ui.next()
    ui.text('确认烧录')
    # Same VID/PID on another port or bus, as well as an unrelated USB identity,
    # must not discard the confirmed selection. Wait for each event's DOM delivery.
    for vid,bus,port in [(0x1f3a,1,3),(0x1f3a,2,2),(0x2207,1,2)]:
        ui.events('usb-hotplug',{'event':'left','vendorId':vid,'productId':0xefe8,'busId':bus,'port':port})
        ui.web.js("const done=arguments[arguments.length-1]; setTimeout(done,200)",asynchronous=True)
        assert ui.js("return document.querySelector('.v2-basic').dataset.step==='4' && !!document.querySelector('.v2w-drawer[open]')")
    ui.events('usb-hotplug',{'event':'left','vendorId':0x1f3a,'productId':0xefe8,'busId':1,'port':2})
    ui.text('目标 USB 已断开')
    assert ui.js("""return document.querySelector('.v2-basic').dataset.step==='3' &&
        document.querySelectorAll('.v2w-preflight-error button').length===1 &&
        !document.querySelector('#v2-preflight, .vi-bar') && !window.__imagerSimulation.lastId""")
    ui.click('.v2w-preflight-error button','重新检测设备')
    ui.text('选定这次要写入的物理端口')
    assert ui.js("return document.querySelector('#v2-preflight').disabled")

    native_images.prepare_f101(ui,fixtures)
    # An RK downloader is physically present, but the F101 scan must not offer it.
    ui.usb('rockchip')
    ui.next()
    ui.text('未发现设备')
    assert ui.js("return !document.querySelector('.vi-device') && document.querySelector('#v2-preflight').disabled && document.querySelector('#v2-step-4').disabled")
    ui.usb('allwinner',count=2,bound=False)
    ui.js("window.__imagerSimulation.usb.push({key:'3:1',flasherKind:'openrockcli',chip:'RK3588',mode:'loader',vid:0x2207,pid:0x350b,bus:3,port:'1'})")
    ui.click('.vi-bar button','重新检测')
    ui.text('选定这次要写入的物理端口')
    assert ui.js("return document.querySelectorAll('.vi-device').length===2 && ![...document.querySelectorAll('.vi-device')].some(e=>e.innerText.includes('2207'))")
    second='.vi-device:has([title$="10.3"])'
    ui.click(second)
    ui.text('将此接口绑定到 F101')
    assert ui.js("return document.querySelector('#v2-preflight').disabled && !document.querySelector('#v2-step-2').disabled")
    ui.save('device-unbound')
    ui.click('.vi-binding button','将此接口绑定到 F101')
    ui.text('核对已连接的下载设备')
    def saves():
        return ui.js("return window.__imagerSimulation.calls.filter(c=>c.cmd==='workbench_save_device').map(c=>c.args.device)")
    assert len(saves())==1
    saved=saves()[0]
    assert (saved['usbBinding'],saved['name'],saved['chipVendor'],saved['flasherKind'])==('location:libusb:1:10.3','F101','allwinner','openixcli'),saved
    assert ui.js("return !document.querySelector('#v2-preflight').disabled")
    ui.save('device-bound')
    # Re-enumerate with reversed order. The saved physical location must remain selected
    # only after an explicit choice, and the other port must still require its own binding.
    ui.js("window.__imagerSimulation.usb.reverse()")
    ui.click('.vi-bar button','重新检测')
    ui.text('选定这次要写入的物理端口')
    assert ui.js("return !document.querySelector('.vi-device.chosen') && document.querySelector('#v2-preflight').disabled")
    ui.click('.vi-device:has([title$="10.2"])')
    ui.text('将此接口绑定到 F101')
    assert ui.js("return document.querySelector('#v2-preflight').disabled")
    ui.click(second)
    ui.text('核对已连接的下载设备')
    ui.next()
    ui.text('确认烧录')
    assert len(saves())==1 and not native_images.submissions(ui)


def main():
    out=Path(sys.argv[2]).resolve()
    out.mkdir(parents=True,exist_ok=True)
    suites={'connection':connections,'images':native_images.run,'vendors':native_vendors.run,'results':native_results.run}
    selected=sys.argv[3:] or list(suites)
    report = dict(suites=selected, passed=[], failures={}, actions=[], screens=[])
    for name in selected:
        try:
            with launch(sys.argv[1]) as (web, temp):
                ui = Review(web, out)
                assert web.js('return !!window.__imagerSimulation'), 'A simulation binary is required'
                try:
                    suites[name](ui, temp)
                except Exception:
                    try:
                        web.shot(out / ('error-' + name + '.png'))
                        (out / ('error-' + name + '.txt')).write_text(web.js('return document.body.innerText'), encoding='utf8')
                    except (OSError, RuntimeError):
                        pass  # The WebView may have already exited. Preserve the initial failure.
                    raise
                finally:
                    report['actions'].extend(ui.actions)
                    report['screens'].extend(ui.shots)
            report['passed'].append(name)
            print('PASS', name, flush=True)
        except Exception:
            report['failures'][name] = traceback.format_exc()
            print('FAIL', name, report['failures'][name], flush=True)
        (out / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf8')
    return int(bool(report['failures']))

if __name__=='__main__':sys.exit(main())
