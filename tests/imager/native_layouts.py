"""Six image formats through the native picker and the production Rust inspectors."""
import hashlib
import os
from pathlib import Path
from struct import pack_into
from zlib import crc32
from native_images import image_fixtures, firmware_package, prepare_f101, local, route, next_enabled
from native_vendors import _make_files


def fixtures(folder):
    folder.mkdir(parents=True, exist_ok=True)
    files = _make_files(folder)
    def put(name, data):
        path = folder / name
        path.write_bytes(data)
        return path

    mbr = bytearray(16384)
    pack_into('<I8sIII', mbr, 4, 512, b'softw411', 1, 0, 3)
    for i, (name, start, count) in enumerate([(b'boot', 2048, 8192), (b'rootfs', 10240, 32768), (b'data', 43008, 16384)]):
        at = 32 + i * 128
        pack_into('<IIII', mbr, at, 0, start, 0, count)
        mbr[at+32:at+32+len(name)] = name
    pack_into('<I', mbr, 0, crc32(mbr[4:]))
    files['nand'] = put('t113-nand.img', firmware_package([
        ('u-boot.fex','12345678','UBOOT_0000000000', bytes(2048)),
        ('boot0_nand.fex','12345678','BOOT0_0000000000', bytes(2048)),
        ('sunxi_mbr.fex','12345678','1234567890___MBR',mbr),
        ('system.fex','RFSFAT16','SYSTEM_FEX000000',bytes(8192))]))

    rk = bytearray(2048)
    rk[:4] = b'RKAF'
    pack_into('<I', rk, 136, 3)
    for i, (name, length) in enumerate([(b'uboot', 4096), (b'boot', 8192), (b'rootfs', 49152)]):
        at = 140+i*112
        rk[at:at+len(name)] = name
        pack_into('<I', rk, at+96, len(rk))
        pack_into('<I', rk, at+108, length)
        rk.extend(bytes(length))
    files['rk'] = put('rk3576-update.img', rk)

    disk = bytearray(1024 * 1024)
    disk[510:512] = b'\x55\xaa'
    for i, (kind, start, sectors) in enumerate([(0x0c, 64, 384),(0x83, 448, 1600)]):
        at = 446 + i * 16
        disk[at+4] = kind
        pack_into('<II', disk, at+8, start, sectors)
    files['wic'].write_bytes(disk)
    files['uuu'] = put('imx6ull-emmc.uuu', b'uuu_version 1.5.0\nSDP: boot -f "boot image.bin"\nFB: flash -raw2sparse all system.wic\n')

    dfu = files['dfu'].read_bytes()
    prefix, target, suffix = bytearray(dfu[:11]), bytearray(dfu[11:285]), bytearray(dfu[-16:])
    elements = bytearray()
    for addr, length in [(0xc0000000, 4096), (0xc0100000, 12288), (0xc0200000, 32768)]:
        e = bytearray(length+8)
        pack_into('<II', e, 0, addr, length)
        elements.extend(e)
    pack_into('<II', target, 266, len(elements), 3)
    pack_into('<I', prefix, 6, 285+len(elements))
    dfu = prefix+target+elements+suffix
    pack_into('<I', dfu, len(dfu)-4, crc32(dfu[:-4]) ^ 0xffffffff)
    files['dfu'] = put('stm32mp157-target.dfu', dfu)

    table = bytearray(3*256)
    payload = bytes(512)
    start = 0
    for i, (name, length) in enumerate([(b'loader', 65536),(b'bootfs', 524288),(b'rootfs', 2097152)]):
        at = i*256
        pack_into('<IIIII', table, at, 0x91DF6DA4, start, length, 0, 512)
        pack_into('<II', table, at+28, 512+len(table)+i*512, 512)
        table[at+36:at+68] = hashlib.sha256(payload).digest()
        table[at+68:at+68+len(name)] = name
        start += length
    head = bytearray(512)
    pack_into('<IIIIII', head, 0, 0x27CB8F93, 0, 0, 2, 3, crc32(table))
    pack_into('<I', head, 4, crc32(head))
    files['kd'] = put('k230-system.kdimg', head+table+payload*3)
    return files


def check_partition_readers(ui, folder):
    # 4 KiB and 512-byte GPT images exercise both CRCs, and truncated RKAF
    # entries must never turn magic bytes into a fabricated package directory.
    def inspect(path, route):
        return ui.web.ipc('imager_inspect_vendor_image', dict(path=str(path),route=route))['layouts']
    for sector in (512,4096):
        disk = bytearray(32*sector)
        head = memoryview(disk)[sector:2*sector]
        head[:8] = b'EFI PART'
        pack_into('<II', head, 8, 0x10000, 92)
        pack_into('<QQQQ', head, 24, 1,31,4,29)
        pack_into('<QII',head,72,2,1,128)
        entry = memoryview(disk)[sector*2:sector*2+128]
        entry[0] = 1
        pack_into('<QQ',entry,32,5,12)
        entry[56:64] = 'boot'.encode('utf-16-le')
        pack_into('<I',head,88,crc32(entry))
        pack_into('<I',head,16,crc32(head[:92]))
        path = folder/'disk.img'
        path.write_bytes(disk)
        layout = inspect(path, 'rk-emmc')[0]
        assert layout['extent'] == len(disk)
        assert layout['regions'] == [dict(name='boot',offset=5*sector,bytes=8*sector)]
        entry[0] ^= 1
        path.write_bytes(disk)
        assert inspect(path, 'rk-emmc') == []
    inner = (folder/'rk3576-update.img').read_bytes()
    outer = bytearray(512)
    outer[:4] = b'RKFW'
    pack_into('<II', outer, 33, len(outer), len(inner))
    path = folder/'nested-update.img'
    path.write_bytes(outer + inner)
    layout = inspect(path, 'rk-update')[0]
    assert [(r['name'],r['offset']) for r in layout['regions']] == [
        ('uboot',2560),('boot',6656),('rootfs',14848)]
    path = folder/'truncated.img'
    path.write_bytes(b'RKAF'+bytes(132)+bytes([8,0,0,0]))
    assert inspect(path, 'rk-update') == []


def run(ui, temp):
    f = fixtures(Path(temp) / 'images')
    check_partition_readers(ui, Path(temp) / 'images')
    ui.web.call('POST', ui.web.session+'/window/rect', {'width':1440, 'height':900})
    def check(target, wanted, count, save=True):
        ui.wait(lambda:ui.js("const p=document.querySelector('section.img-layout');return p && p.innerText.includes(arguments[0]) && p.querySelectorAll('tbody tr').length===arguments[1]", wanted, count),target+' layout')
        ui.js("document.querySelector('section.img-layout').scrollIntoView({block:'nearest'})")
        assert ui.js("const r=document.querySelector('.il-map').getBoundingClientRect();return r.width>450 && r.top>=0 && r.bottom<=innerHeight"),target
        if save:
            ui.save('layout-'+target)
    prepare_f101(ui, temp)
    check('F101','0x080000',2)
    assert ui.js("return !document.querySelector('.img-layout').innerText.includes('FES')")
    ui.click('.imager-image-remove')
    ui.wait(lambda:not ui.js("return !!document.querySelector('.img-layout')"),'cleared image layout')

    candidate = os.environ.get('LYNX_LAYOUT_F101_CANDIDATE')
    if candidate:
        folder = Path(candidate)
        local(ui,'f101')
        ui.pick(folder/'nekobox-full.img','选择镜像…')
        ui.pick(folder/'f101-original-ram-loader.img','选择引导包…')
        ui.pick(folder/'candidate.json','选择参考清单…')
        next_enabled(ui,True)
        check('F101','参数起点',3,False)
        ui.text('40 KiB')
        ui.save('layout-F101-local')
        ui.web.call('POST', ui.web.session+'/window/rect', {'width':1280,'height':720})
        check('F101-small','应用起点',3,False)
        ui.web.call('POST', ui.web.session+'/window/rect', {'width':1440,'height':900})

    local(ui,'t113-pro')
    route(ui,'mainline_nand')
    ui.pick(f['nand'],'选择镜像…')
    ui.pick(image_fixtures(temp)['loader'],'选择引导包…')
    next_enabled(ui,True)
    check('T113','rootfs',3)
    ui.js("const e=document.querySelector('.img-layout select');e.value='1';e.dispatchEvent(new Event('change',{bubbles:true}))")
    check('T113','sunxi_mbr.fex',4,False)

    for key, board, model, desired, count in [
        ('rk','rk3576','RK3576 EVB','uboot',3), ('kd','canaan-k230','CanMV-K230','bootfs',3),
        ('uuu','nxp-imx6ull','i.MX6ULL EVK','主分区 2',2),
        ('dfu','stm32mp157','STM32MP157 DK2','0xC0100000',3)]:
        ui.reset()
        ui.board(board,model)
        ui.next()
        ui.pick(f[key],'选择文件')
        next_enabled(ui,True)
        check({'rk':'RK3576','kd':'K230','uuu':'iMX6ULL','dfu':'STM32MP157'}[key],desired,count)
