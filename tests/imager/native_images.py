"""File-selection checks against the real read-only Rust commands in the native WebView."""
import hashlib
import json
import struct
from pathlib import Path


def firmware_package(items):
    """Minimal IMAGEWTY v3 container with directory and each indexed payload in bounds."""
    out = bytearray((len(items) + 1) * 1024)
    out[:8] = b"IMAGEWTY"
    struct.pack_into("<II", out, 8, 0x300, 1024)
    struct.pack_into("<I", out, 60, len(items))
    for i, (name, main, sub, value) in enumerate(items, 1):
        start = i * 1024
        payload = len(out)
        struct.pack_into("<II", out, start, len(name) + 1, 1024)
        out[start + 8:start + 16] = main.encode().ljust(8, b" ")
        out[start + 16:start + 32] = sub.encode().ljust(16, b"\0")
        out[start + 36:start + 36 + len(name)] = name.encode()
        struct.pack_into("<QQQ", out, start + 292, len(value), len(value), payload)
        out.extend(value)
    struct.pack_into("<I", out, 24, len(out))
    return out


def image_fixtures(fixtures):
    folder = Path(fixtures) / "allwinner-inputs"
    folder.mkdir(parents=True, exist_ok=True)
    def write(name, data):
        p = folder / name
        p.write_bytes(data)
        return p
    def manifest(name, data):
        return write(name, json.dumps(data).encode())
    app = b"LYNX-QA-APPLICATION\0" * 16
    raw = bytearray(600 * 1024)
    raw[4:12] = b"eGON.BT0"
    raw[0x80000:0x80000 + len(app)] = app
    raw_path = write("simulated-f101-system.img", raw)
    reference = {"sha256":hashlib.sha256(raw).hexdigest(), "image_bytes":len(raw),
                 "physical_app_offset":"0x80000", "app_bytes":len(app)}
    fes = [("fes1.fex", "FES", "FES_1-0000000000", b"RAM-FIRST-STAGE"),
           ("u-boot.fex", "12345678", "UBOOT_0000000000", b"RAM-SECOND-STAGE")]
    mbr = ("sunxi_mbr.fex", "12345678", "1234567890___MBR", b"SIMULATED-LAYOUT")
    nand = ("boot0_nand.fex", "12345678", "BOOT0_0000000000", b"NAND-BOOT0")
    nor = ("boot0_spinor.fex", "12345678", "1234567890BNOR_0", b"NOR-BOOT0")
    data = ("system.fex", "RFSFAT16", "SYSTEM_FEX000000", b"SYSTEM-TEST-DATA")
    emmc = bytearray(24576)
    emmc[0x2004:0x200c] = b"eGON.BT0"
    emmc[0x2014:0x2017] = b"SPL"
    return dict(raw=raw_path, app=write("zephyr.bin", app), ref=manifest("BUILD-INFO.json", reference),
                wrong=manifest("wrong.json", dict(reference, sha256="bc"*32)),
                loader=write("board-ram-loader.img", firmware_package(fes)),
                factory=write("factory-nor.img", firmware_package(fes + [nor, mbr, data])),
                nand=write("components-nand.img", firmware_package([fes[1], nand, mbr, data])),
                emmc=write("mainline-emmc.img", emmc), unknown=write("unknown.img", b"unknown"*1000),
                corrupt=write("damaged-container.img", b"IMAGEWTY\0"))


def local(ui, board):
    ui.reset()
    ui.board(board)
    ui.next()
    ui.click(".v2c-tabs button", "本地文件")


def next_enabled(ui, expected):
    ui.wait(lambda:ui.js("return document.querySelector('.v2-foot-actions .v2-primary')?.disabled === arguments[0]",not expected),
            "Next " + ("enabled" if expected else "blocked"))


def route(ui, name):
    ui.click('input[name="aw-system-route"][value="' + name + '"]')


def inspector(ui, panel):
    ui.wait(lambda:ui.js("const p=document.querySelector('.imager-review-page'); return p?.dataset.inspector===arguments[0] && document.activeElement===p.querySelector('h2')",panel), 'focused inline '+panel+' workspace')
    assert ui.js("""const show=e=>!!e?.getClientRects().length;
        return [...document.querySelectorAll('.v2c-tabs,.imager-image-line,.imager-aw-route,.imager-file-details')].every(e=>!show(e)) &&
               ['.imager-app-header','nav[aria-label="烧录步骤"]','.v2-foot','.imager-review-page'].every(s=>show(document.querySelector(s))) &&
               !document.querySelector('dialog.imager-review-dialog[open]')"""), 'file picker hides while shell stays visible'

def return_to_files(ui, label, escape=False):
    if escape:
        ui.web.call('POST', ui.web.session+'/actions', {'actions':[{'type':'key','id':'inspector-keys',
            'actions':[{'type':'keyDown','value':'\uE00C'},{'type':'keyUp','value':'\uE00C'}]}]})
    else:
        ui.click(".imager-review-page button","返回文件选择")
    ui.wait(lambda:ui.js("return !document.querySelector('.imager-review-page') && !!document.querySelector('.v2c-tabs')?.getClientRects().length && document.activeElement?.textContent.trim()===arguments[0]",label), 'restore picker and trigger focus')
    assert ui.js("return !document.querySelector('.imager-image-review input[type=checkbox]')")


def prepare_f101(ui, fixtures):
    """Return on step 2 with checked real files; no USB access."""
    paths = image_fixtures(fixtures)
    local(ui, "f101")
    ui.pick(paths["raw"], "选择镜像…")
    ui.text(paths["raw"].name)
    ui.wait(lambda: ui.web.find("button", "分析系统镜像"), "image identity ready")
    ui.pick(paths["loader"], "选择引导包…")
    ui.text(paths["loader"].name)
    ui.wait(lambda: ui.web.find("button", "检查引导组件"), "loader identity ready")
    ui.pick(paths["ref"], "选择参考清单…")
    ui.wait(lambda:ui.js("return document.querySelector('.imager-image-reference small')?.innerText==='BUILD-INFO.json'"), "parsed build reference")
    next_enabled(ui, True)
    return paths


def review_target(ui):
    ui.usb("allwinner")
    ui.next()
    ui.selectUsb()
    ui.next()
    ui.text("确认烧录")
    ui.click(".v2w-drawer input[type=checkbox]")


def submissions(ui):
    return ui.js("return window.__imagerSimulation.calls.filter(x=>x.cmd==='workbench_start_flash'||x.cmd==='workbench_firmware_start')")


def submit(ui, image, flow, loader=None):
    review_target(ui)
    assert not submissions(ui)
    ui.click(".v2w-drawer .v2w-primary")
    ui.wait(lambda: len(submissions(ui)) == 1,"one flash command after explicit confirmation")
    command=submissions(ui)[0]
    assert command["cmd"] == "workbench_start_flash"
    args=command["args"]
    assert args["firmwarePath"] == str(image)
    opts=args["options"]
    if flow=="vendor":
        assert opts==dict(systemType="vendor",mode="full_erase",verify=True,postFlashAction="reboot"),opts
    else:
        assert opts["systemType"]=="mainline" and opts["mainlineRoute"]==flow,opts
        assert opts["loaderPath"]==str(loader),opts


def run(ui, fixtures):
    files = image_fixtures(fixtures)
    # Simulate the published catalog and transport only. The downloaded files still go
    # through the same Rust hash, container and manifest checks as manually chosen paths.
    ui.reset()
    uid="sim-f101-resource"
    profile={"id":19,"boardUid":uid,"name":"F101 simulated","chipVendor":"allwinner",
             "usbBinding":"location:libusb:1:10.2","flasherKind":"openixcli","flasherPath":"","flasherArgs":"[]"}
    ui.fixture("workbench_list_devices",[])
    file_ref=lambda p:dict(fileName=p.name,sizeBytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest())
    manifest=dict(applicationId="nekobox",version="qa.1",boardModel="yuzukineko",chip="f101",route="xip_spinor_full",layout={"bootOffset":0,"applicationOffset":524288,"parametersOffset":131072},image=file_ref(files["raw"]),loader=file_ref(files["loader"]))
    entry=dict(releaseId=1,indexSha256="ab"*32,entry=dict(status="active",manifest=manifest))
    ui.fixture("imager_firmware_catalog",[entry])
    ui.fixture("imager_stage_firmware",dict(releaseId=1,indexSha256=entry["indexSha256"],manifest=manifest,imagePath=str(files["raw"]),loaderPath=str(files["loader"])))
    bound_candidate=dict(id="qa-candidate",deviceId=19,boardUid=uid,manifest=manifest,imagePath=str(files["raw"]),loaderPath=str(files["loader"]))
    ui.board("f101")
    ui.next()
    ui.text("玄铁 C907 · RISC-V")
    assert ui.js("return document.querySelector('.v2c-tabs [aria-pressed=true]').innerText.trim()==='本地文件'")
    assert ui.js("return !document.querySelector('.v2c-device-select,.image-desk-checklist')")
    geometry = """return ['.image-desk-title','.image-source-bar','.image-target-summary'].map(s=>{
        const r=document.querySelector(s).getBoundingClientRect();return [r.x,r.y,r.width]})"""
    anchored = ui.js(geometry)
    for source in ('本地文件','最近使用','板型资源'):
        ui.click('.v2c-tabs button',source)
        assert ui.js(geometry) == anchored, source
        assert not ui.js("return !!document.querySelector('.image-manifest')")
    assert ui.js("return !window.__imagerSimulation.calls.some(c=>c.cmd==='workbench_firmware_download')")
    ui.click("button","查看 qa.1 资源详情")
    ui.save("f101-choose-image")
    ui.pick(files["factory"],"选择本地镜像…")
    ui.wait(lambda:ui.js("return document.querySelector('.v2c-tabs button[aria-pressed=true]')?.innerText.trim()==='本地文件'"),"local image opened directly")
    ui.text("factory-nor.img")
    ui.click(".v2c-tabs button","板型资源")
    ui.click("button","查看 qa.1 资源详情")
    ui.text("清单声明的适用板型")
    ui.pick(files["loader"],"获取并核对 nekobox qa.1")
    next_enabled(ui,True)
    ui.text("下载的镜像与发布清单的指纹一致")
    original = ui.web.call("GET", ui.web.session+"/window/rect")
    ui.web.call("POST", ui.web.session+"/window/rect", {"width":1090, "height":720})
    ui.wait(lambda:ui.js("return innerWidth === 1090"), "narrow window")
    assert ui.js("""const p=document.querySelector('.v2c-primary').getBoundingClientRect();
        const buttons=[...document.querySelectorAll('.imager-image-line button')].filter(b=>b.getClientRects().length);
        return buttons.length>0 && buttons.every(b=>b.getBoundingClientRect().right<=p.right+1)""")
    ui.web.call("POST", ui.web.session+"/window/rect", {"width":original["width"], "height":original["height"]})
    assert ui.js("return [...document.querySelectorAll('.imager-aw-route input')].every(e=>e.disabled)")
    assert ui.js("return !window.__imagerSimulation.calls.some(c=>c.cmd==='workbench_firmware_download')")
    ui.fixture("workbench_list_devices",[profile])
    ui.fixture("workbench_firmware_prepare", {"id":"qa-confirmation", "deviceId":19, "candidateId":"qa-candidate"})
    ui.fixture("workbench_firmware_download", dict(bound_candidate,boardUid="different-board"))
    ui.usb("allwinner")
    ui.next()
    ui.selectUsb()
    ui.next()
    ui.text("设备或发布文件已变化，请重新预检。")
    assert ui.js("return !window.__imagerSimulation.calls.some(c=>c.cmd==='workbench_firmware_prepare')")
    assert not submissions(ui)
    ui.fixture("workbench_firmware_download", bound_candidate)
    ui.click('.v2w-preflight-error button','重新检测设备')
    ui.selectUsb()
    ui.next()
    ui.text("确认烧录")
    ui.click(".v2w-drawer input[type=checkbox]")
    assert ui.js("return window.__imagerSimulation.calls.filter(c=>c.cmd==='workbench_firmware_download').every(c=>c.args.deviceId===19 && c.args.request.indexSha256===arguments[0])",entry["indexSha256"])
    assert not submissions(ui)
    ui.click(".v2w-drawer .v2w-primary")
    ui.wait(lambda: len(submissions(ui)) == 1, "catalog candidate submitted with its own confirmation")
    assert submissions(ui)[0] == {"cmd":"workbench_firmware_start","args":{"deviceId":19,"confirmationId":"qa-confirmation","confirmed":True}}

    local(ui, "f101")
    route(ui, "vendor")
    ui.pick(files["factory"], "选择镜像…")
    ui.text("将这个包改作 RAM 引导来源")
    next_enabled(ui, True)
    ui.save("aw-source-role")
    ui.click("button", "将这个包改作 RAM 引导来源")
    ui.text("factory-nor.img")
    next_enabled(ui, False)
    ui.pick(files["raw"], "选择镜像…")
    next_enabled(ui, True)

    files = prepare_f101(ui, fixtures)
    assert ui.js("""
      const x = [...document.querySelectorAll('.imager-file-destination')].map(e=>e.getBoundingClientRect().left);
      return x.length === 2 && Math.abs(x[0] - x[1]) < 1;
    """), "Flash and RAM destinations must share the same column"
    ui.save("aw-f101-files")
    ui.click("button", "检查引导组件")
    inspector(ui,"loader")
    ui.text("fes1.fex")
    ui.text("u-boot.fex")
    ui.click(".imager-review-page summary")
    ui.text("包内文件目录")
    ui.save("aw-fes-source")
    return_to_files(ui,"检查引导组件",escape=True)
    next_enabled(ui,True)
    ui.click("button", "分析系统镜像")
    inspector(ui,"image")
    ui.text("清单声明的文件布局")
    ui.text(hashlib.sha256(files["raw"].read_bytes()).hexdigest())
    next_enabled(ui,True)
    ui.pick(files["app"], "选择应用文件并比对…")
    ui.text("与镜像应用区逐字节一致")
    ui.save("aw-f101-analysis")
    return_to_files(ui,"分析系统镜像")
    next_enabled(ui,True)
    ui.click(".imager-file-details summary")
    ui.text(hashlib.sha256(files["raw"].read_bytes()).hexdigest())
    ui.click(".imager-file-details summary")
    next_enabled(ui, True)
    # The recent-source picker must re-read the path and request its separate RAM source.
    ui.click(".v2c-tabs button:nth-child(3)")
    ui.click(".v2c-recent-row")
    ui.text(files["raw"].name)
    next_enabled(ui, False)

    files=prepare_f101(ui, fixtures)
    ui.pick(files["wrong"], "更换参考清单…")
    ui.text("镜像 SHA-256 与参考值不同")
    next_enabled(ui, False)
    ui.save("aw-hash-conflict")
    assert not submissions(ui)
    ui.pick(files["ref"], "更换参考清单…")
    ui.wait(lambda:ui.js("return document.querySelector('.imager-image-reference small')?.innerText==='BUILD-INFO.json'"), "recovered build reference")
    next_enabled(ui, True)

    local(ui, "f101")
    ui.pick(files["unknown"], "选择镜像…")
    ui.text("完整 SPI NOR 镜像")
    ui.click("button", "分析系统镜像")
    inspector(ui,"image")
    ui.text("未识别到全志容器或启动标记")
    ui.text("尚未提供参考值")
    ui.save("aw-unknown-image")
    return_to_files(ui,"分析系统镜像")
    next_enabled(ui, False)
    assert not submissions(ui)

    local(ui, "f101")
    route(ui, "vendor")
    ui.pick(files["factory"], "选择镜像…")
    next_enabled(ui, True)
    ui.save("aw-factory")
    submit(ui,files["factory"],"vendor")

    local(ui, "t113-pro")
    route(ui, "mainline_nand")
    ui.pick(files["nand"], "选择镜像…")
    ui.pick(files["loader"], "选择引导包…")
    next_enabled(ui, True)
    ui.save("aw-mainline-nand")
    submit(ui,files["nand"],"mainline_nand",files["loader"])

    local(ui, "t113-pro")
    route(ui, "mainline_nand")
    ui.pick(files["nand"], "选择镜像…")
    ui.pick(files["raw"], "更换系统镜像…")
    ui.text("需要 IMAGEWTY 组件包")
    next_enabled(ui, False)
    assert not submissions(ui)

    local(ui, "t153-emmc")
    ui.pick(files["emmc"], "选择镜像…")
    ui.pick(files["loader"], "选择引导包…")
    next_enabled(ui, True)
    ui.save("aw-mainline-emmc")
    submit(ui,files["emmc"],"mainline_emmc",files["loader"])

    local(ui, "t153-emmc")
    ui.pick(files["emmc"], "选择镜像…")
    ui.pick(files["corrupt"], "更换系统镜像…")
    ui.text("IMAGEWTY 文件头截断")
    next_enabled(ui, False)
    assert not submissions(ui)

    files=prepare_f101(ui,fixtures)
    review_target(ui)
    with files["ref"].open("ab") as f:
        f.write(b"\n")
    ui.click(".v2w-drawer .v2w-primary")
    ui.text("镜像参考清单已改变")
    assert not submissions(ui)

    files=prepare_f101(ui,fixtures)
    ui.click("button","分析系统镜像")
    ui.pick(files["app"],"选择应用文件并比对…")
    ui.text("与镜像应用区逐字节一致")
    return_to_files(ui,"分析系统镜像")
    next_enabled(ui, True)
    review_target(ui)
    content=bytearray(files["app"].read_bytes())
    content[-1]^=1
    files["app"].write_bytes(content)
    ui.click(".v2w-drawer .v2w-primary")
    ui.text("应用文件或镜像中的应用区已改变")
    assert not submissions(ui)
