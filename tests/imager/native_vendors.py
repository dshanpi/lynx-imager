"""Exercise all eight vendor routes against the native read-only image inspector.

USB enumeration, file dialogs and write jobs are simulated by the shared desktop harness.
"""
import hashlib
from pathlib import Path
from struct import pack_into
from zlib import crc32


def _make_files(folder):
    folder.mkdir(parents=True, exist_ok=True)
    def put(name, data):
        p=folder/name
        p.write_bytes(data)
        return p

    files={'rk':put('rockchip-update.img',b'RKFW'+bytes(1020)),
           'raw':put('whole-disk.img',bytes(4096)),
           'loader':put('rk-loader.bin',b'RK loader fixture'+bytes(496)),
           'uuu':put('install.uuu',b'uuu_version 1.5.0\nSDPS: boot -f "boot image.bin"\nFB: flash -raw2sparse all system.wic\n'),
           'wic':folder/'system.wic'}
    put('boot image.bin',bytes(512))
    files['wic'].unlink(missing_ok=True)

    # Prefix + one target with one element + standard DFU suffix.
    prefix=bytearray(11)
    target=bytearray(274)
    element=bytearray(8+32)
    pack_into('<II',element,0,0x08001000,32)
    target[:6]=b'Target'
    target[6]=0
    pack_into('<II',target,266,len(element),1)
    prefix[:5]=b'DfuSe'
    prefix[5]=1
    pack_into('<I',prefix,6,len(prefix)+len(target)+len(element))
    prefix[10]=1
    suffix=bytearray(16)
    suffix[:8]=bytes.fromhex('ffff11df83041a01')
    suffix[8:12]=b'UFD\x10'
    dfu=prefix+target+element+suffix
    pack_into('<I',dfu,len(dfu)-4,crc32(dfu[:-4]) ^ 0xffffffff)
    files['dfu']=put('st-layout.dfu',dfu)
    files['bin']=put('firmware.bin',bytes(4096))

    # K230 v2 metadata. Both CRCs and the declared payload hash are genuine.
    payload=b'test-data'.ljust(512,b'\xff')
    table=bytearray(256)
    pack_into('<IIIII',table,0,0x91DF6DA4,0,512,0,512)
    pack_into('<II',table,28,1024,len(payload))
    table[36:68]=hashlib.sha256(payload).digest()
    table[68:74]=b'bootfs'
    head=bytearray(512)
    pack_into('<IIIIII',head,0,0x27CB8F93,0,0,2,1,crc32(table))
    pack_into('<I',head,4,crc32(head))
    files['kd']=put('k230-system.kdimg',head+table+bytes(256)+payload)
    return files


def run(ui, fixtures):
    f=_make_files(Path(fixtures)/'vendor-routes')
    def board(board_id, model, vendor, medium=None):
        ui.reset()
        ui.board(board_id,model,medium)
        if not ui.js("return !!document.querySelector('.vendor-switch')"):
            ui.next()
        ui.text('烧录路线')
        ui.usb(vendor,{'rockchip':'loader','nxp':'sdp','st':'dfu','kendryte':'bootrom / loader'}[vendor])
    def route(name):
        ui.click('.vendor-switch button',name)
    def pick(file,replace=False):
        ui.pick(str(file),'更换文件' if replace else '选择文件')
    def enabled():
        return bool(ui.js("const b=document.querySelector('.v2-foot-actions button.v2-primary');return b&&!b.disabled"))
    def ready():
        ui.wait(enabled,'vendor route can continue')
    def blocked():
        assert not enabled(), 'page incorrectly allows continuation'
    def target():
        ready();ui.next();ui.selectUsb()
    def field(sel,val):
        ui.js("""const e=document.querySelector(arguments[0]);if(!e)throw Error('field not found');
        const setter=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value').set;
        setter.call(e,arguments[1]);e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));""",sel,val)

    board('rk3588','RK3588-EVB1','rockchip')
    pick(f['rk']);ui.text('RKFW');ready();ui.save('rk-update')
    route('主线 eMMC 整盘');pick(f['raw']);ui.text('缺少配套 Loader');blocked()
    ui.save('rk-emmc-loader-required')
    ui.pick(str(f['loader']),'选择 Loader')
    target();ui.text('核对 eMMC 覆盖范围');blocked()
    ui.click('.vendor-check input');ready();ui.save('rk-emmc-confirm')
    ui.click('.vendor-check input');blocked()

    board('nxp-imx','i.MX8MP-EVK','nxp')
    pick(f['uuu']);ui.text('缺失或为空');blocked();ui.save('nxp-uuu-missing')
    f['wic'].write_bytes(bytes(4096))
    pick(f['uuu'],True);ready();ui.save('nxp-uuu-ready')
    route('单板已保存配置');pick(f['wic']);ready();target()
    ui.text('单板保存参数');ui.save('nxp-configured')

    board('stm32mp1','STM32MP157F-DK2','st')
    pick(f['dfu']);ui.text('0x08001000');ready();ui.save('st-dfuse-layout')
    target();ui.text('DFU 目标接口');blocked()
    sel='.vendor-detail .vendor-field select'
    ui.wait(lambda:ui.js("const s=document.querySelector(arguments[0]);return s&&[...s.options].some(o=>o.value==='0')",sel),'DFU alt 0 enumeration')
    field(sel,'0');ready();ui.save('st-dfu-alternate')

    board('stm32mp1','STM32MP157F-DK2','st')
    route('RAW 二进制');pick(f['bin']);target();field(sel,'0');blocked()
    field('.vendor-detail .vendor-field input','0x07zz');blocked()
    field('.vendor-detail .vendor-field input','0x08001000');ready();ui.save('st-raw-address')

    board('canaan-k230','CanMV-K230','kendryte')
    pick(f['kd']);ui.text('bootfs');ready();ui.save('k230-kdimage')
    route('RAW 整盘');pick(f['kd']);ui.text('请选择未封装');blocked()
    pick(f['raw'],True);target();ui.text('选择实际存储介质');blocked()
    field(sel,'sdcard');ready();ui.save('k230-raw-medium')
    field(sel,'');blocked()

    # Submit every configuration through the real confirmation UI. The isolated
    # test build intercepts the write IPC before any board adapter can run.
    cases = [
        ('rk-update','rk3576','','rockchip','厂商 update.img','rk'),
        ('rk-emmc','rk3588','RK3588-EVB1','rockchip','主线 eMMC 整盘','raw'),
        ('nxp-uuu','nxp-imx6ull','i.MX6ULL EVK','nxp','UUU 脚本','uuu'),
        ('nxp-configured','nxp-imx','i.MX8MP-EVK','nxp','单板已保存配置','wic'),
        ('st-dfuse','stm32mp157','STM32MP157F-DK2','st','DfuSe 容器','dfu'),
        ('st-raw','stm32mp1','STM32MP157F-DK2','st','RAW 二进制','bin'),
        ('k230-kdimage','canaan-k230','CanMV-K230','kendryte','KDImage 分区包','kd'),
        ('k230-raw','canaan-k230','CanMV-K230','kendryte','RAW 整盘','raw'),
    ]

    def confirm(case):
        name,board_id,model,family,choice,artifact=case
        board(board_id,model,family,'SPI NOR' if name=='k230-kdimage' else None)
        route(choice)
        pick(f[artifact])
        if name=='rk-emmc':
            ui.pick(str(f['loader']),'选择 Loader')
        target()
        if name=='rk-emmc':
            ui.click('.vendor-check input')
        if name.startswith('st-'):
            field(sel,'0' if name=='st-dfuse' else '3')
            if name=='st-raw':
                field('.vendor-detail .vendor-field input','0x08001000')
        if name.startswith('k230-'):
            field(sel,'spi_nor' if name=='k230-kdimage' else 'sdcard')
        ready()
        ui.next()
        ui.text('确认烧录')
        ui.wait(lambda:ui.js("return !!document.querySelector('.v2w-drawer[open]')"),'vendor confirmation drawer')
        assert ui.js("return document.querySelector('.v2w-drawer .v2w-primary').disabled")
        ui.click('.v2w-drawer input[type=checkbox]')

    def start_calls():
        return ui.js("return JSON.parse(JSON.stringify(window.__imagerSimulation.calls.filter(x=>x.cmd==='workbench_start_flash').map(x=>x.args)))")

    for case in cases:
        name,_,_,_,_,artifact=case
        confirm(case)
        assert not start_calls()
        ui.click('.v2w-drawer .v2w-primary')
        ui.wait(lambda:len(start_calls())==1,'simulated start '+name)
        call=start_calls()[0]
        assert call['deviceId']==1 and call['firmwarePath']==str(f[artifact]),(name,call)
        key=ui.js('return window.__imagerSimulation.usb[0].key')
        cfg={'route':name}
        opts={'systemType':'vendor','vendorSetup':cfg}
        if name=='rk-emmc':
            cfg.update(targetKey=key,eraseConfirmed=True)
            opts.update(systemType='mainline',loaderPath=str(f['loader']),mainlineRoute='rockchip_raw_emmc',mode='full_erase',verify=True,postFlashAction='reboot')
        elif name=='nxp-uuu':
            cfg['dependencies']=[]
            for filename in ('boot image.bin','system.wic'):
                payload=(f['uuu'].parent/filename).read_bytes()
                cfg['dependencies'].append(dict(path=filename,bytes=len(payload),sha256=hashlib.sha256(payload).hexdigest()))
        elif name.startswith('st-'):
            cfg.update(alt=0 if name=='st-dfuse' else 3,targetKey=key)
            if name=='st-raw':
                cfg['address']='0x08001000'
        elif name.startswith('k230-'):
            cfg.update(medium='spi_nor' if name=='k230-kdimage' else 'sdcard',targetKey=key)
        assert call['options']==opts,(name,call['options'],opts)
        print('VENDOR_SUBMISSION',name,flush=True)

    # Replace a referenced file with different bytes of exactly the same size
    # after accepting the review. The original script and USB device stay fixed.
    confirm(cases[2])
    original=f['wic'].read_bytes()
    try:
        f['wic'].write_bytes(bytes([original[0]^255])+original[1:])
        ui.click('.v2w-drawer .v2w-primary')
        ui.text('提交前检查未通过')
        ui.text('厂商镜像或依赖文件已变化')
        assert not start_calls(), 'changed companion file reached the flasher'
        ui.save('nxp-uuu-changed-after-confirm')
    finally:
        f['wic'].write_bytes(original)
