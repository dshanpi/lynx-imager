"""Production IPC and desktop smoke. Run under dbus-run-session with a normal Ubuntu binary."""
import argparse
import hashlib
import json
from pathlib import Path

from native_driver import launch, wait


def keys(web, *chord):
    actions = [{"type": "keyDown", "value": key} for key in chord]
    actions.extend({"type": "keyUp", "value": key} for key in reversed(chord))
    web.call("POST", web.session + "/actions",
             {"actions": [{"type": "key", "id": "hardware-keyboard", "actions": actions}]})


def native_window(web):
    result = web.js("""return {origin: location.origin, simulation:'__imagerSimulation' in window,
      native: typeof window.__TAURI_INTERNALS__?.invoke === 'function', w:innerWidth,h:innerHeight,
      dpr:devicePixelRatio, scroll:document.documentElement.scrollWidth,
      client:document.documentElement.clientWidth}""")
    assert result["origin"] == "tauri://localhost" and result["native"] and not result["simulation"], result
    assert result["scroll"] <= result["client"] + 1, result
    return result


def readback_ipc(web, temp):
    # Exercise actual Rust image inspection and byte comparison with known content.
    app = temp / "test-app.bin"
    image = temp / "test-system.img"
    data = bytes(range(16)) * 256
    app.write_bytes(data)
    raw = bytearray(0x80000)
    raw[4:12] = b"eGON.BT0"
    raw.extend(data)
    image.write_bytes(raw)
    digest = hashlib.sha256(raw).hexdigest()
    fingerprint = web.ipc("imager_file_identity", {"path": str(image)})
    assert fingerprint == {"sizeBytes": len(raw), "sha256": digest}, fingerprint
    inspected = web.ipc("imager_inspect_image", {"path": str(image)})
    assert inspected["sha256"] == digest and inspected["egonAtZero"] and not inspected["splAt8192"], inspected
    args = {"imagePath": str(image), "applicationPath": str(app), "offset": 0x80000,
            "expectedBytes": len(data)}
    expected = web.ipc("imager_compare_application", args)
    assert expected["matches"] and expected["comparedBytes"] == len(data), expected
    app.write_bytes(bytes([1]) + data[1:])
    actual = web.ipc("imager_compare_application", args)
    assert not actual["matches"], actual
    return {"sizeBytes": len(raw), "sha256": digest, "appOffset": args["offset"],
            "matchingSegment": expected["matches"], "modifiedSegment": actual["matches"]}


def offline_board_fields():
    return dict(id=None, boardUid="", name="QA-OFFLINE-F101", boardImagePath="", chipVendor="allwinner",
                  serialPort="", serialVid=0, serialPid=0, serialNumber="", serialInterfaceNumber=None,
                  usbBinding="", cameraBinding="", adbSerial="", baudRate=115200, serialConsoleType="auto",
                  serialLoginMode="none", serialLoginUsername="root", serialLoginPrompt="login:", serialPasswordPrompt="Password:",
                  serialShellPrompt="#", serialAutoLogin=False, flasherKind="openixcli", flasherPath="", flasherArgs="",
                  powerController="", powerControllerVid=0, powerControllerPid=0, powerControllerSerial="",
                  powerControllerInterfaceNumber=None, powerBaudRate=115200, powerProfileJson="", powerChannel="",
                  bootChannel="", resetChannel="", powerOffWaitMs=1000, powerOnWaitMs=2000,
                  serialReconnectTimeoutMs=30000, bootTimeoutMs=60000)


def offline_board(web):
    fields = offline_board_fields()
    board_id = web.ipc("workbench_save_device", {"device": fields})
    rows = web.ipc("workbench_list_devices", {})
    saved = next(d for d in rows if d["id"] == board_id)
    assert saved["name"] == fields["name"] and saved["boardUid"]
    assert all(not saved[k] for k in ("usbBinding","serialPort","adbSerial","powerController"))
    return board_id


def advanced(web, capture):
    def tab(name):
        selector = '[data-testid="linux-tab-' + name + '"]'
        web.click(selector)
        wait(lambda: web.js('return document.querySelector(arguments[0])?.getAttribute("aria-current")==="page"',selector),
             'active ' + name)

    def disabled(root, label):
        wait(lambda: web.js("""return [...document.querySelectorAll(arguments[0]+' button')].some(
          e=>e.getClientRects().length && e.innerText.trim()===arguments[1] && e.disabled)""",root,label),
             'offline button disabled: ' + label)

    web.click(".board-item-select")
    web.shown("QA-OFFLINE-F101")
    web.shown("设备离线")
    tab("overview")
    wait(lambda:web.find(".lynx-device-overview"), "saved device overview")
    capture(web,"production-advanced")
    files = '[data-testid="adb-file-transfer"]'
    web.js("document.querySelector(arguments[0]).scrollIntoView({block:'center'})",files)
    web.shown("尚未绑定 ADB 设备")
    for label in ("发送文件至设备","从设备接收","刷新目录"):
        disabled(files,label)
    capture(web,"production-advanced-files")

    tab("serial")
    disabled(".device-console-view","粘贴")
    disabled(".device-console-view","导出完整日志")
    capture(web,"production-advanced-serial")
    tab("sdk")
    disabled('[data-testid="linux-sdk-terminal"]',"新建终端")
    disabled('[data-testid="linux-sdk-terminal"]',"重连")
    capture(web,"production-advanced-ssh")
    tab("flash")
    disabled(".lynx-device-tool-content","确认写入…")
    assert web.js("return document.querySelector('[data-testid=linux-tab-flash]').getAttribute('aria-description')")== "未检测到烧录设备"
    capture(web,"production-advanced-flash")
    web.click('[data-testid="linux-header-device-settings"]')
    modal = '[data-testid="linux-settings-dialog"]'
    wait(lambda:web.find(modal), "saved board settings")
    assert web.js("return document.querySelector('[data-testid=linux-board-name]').value")=="QA-OFFLINE-F101"
    capture(web,"production-device-settings")
    web.click(modal+" header button","关闭")
    wait(lambda:not web.find(modal),"device settings dismissed")


def run(binary, out):
    report = {"binary": str(binary.resolve()), "checks": [], "screens": []}
    def checked(name):
        report["checks"].append(name)
        print("PASS", name, flush=True)

    def capture(web, name):
        shape = native_window(web)
        web.shot(out / (name + ".png"))
        report["screens"].append(dict(id=name, **shape))

    def assert_scope(web, page):
        wait(lambda: web.js("""return [...document.querySelectorAll('nav[aria-label="工作区"] button[aria-current="page"]')]
          .some(e=>e.getClientRects().length && e.innerText.trim()===arguments[0])""", page), page)

    try:
        with launch(binary) as (web, temp):
            native_window(web)
            initial_tasks = web.ipc("workbench_list_tasks", {"limit": 100})
            report["fileInspection"] = readback_ipc(web, temp)
            checked("生产 Rust IPC · 文件指纹、镜像头及应用片段匹配与不匹配")

            assert_scope(web, "烧录系统")
            assert web.js("return document.querySelector('.v2-foot-actions .v2-primary').disabled")
            web.click(".target-vendors button", "瑞芯微")
            web.click(".target-families button", "RK35 系列")
            assert web.js("return document.querySelectorAll('button[data-model]').length===9")
            web.click(".target-chip-help summary")
            web.shown("查看主控芯片表面的丝印")
            web.click('button[data-model="RK3506"]')
            assert web.js("return document.querySelectorAll('[data-medium]:disabled').length===2")
            assert web.js("return document.querySelector('.v2-foot-actions .v2-primary').disabled")
            web.click("button", "返回型号列表")
            web.js('const s=document.querySelector("select[aria-label=搜索范围]"); s.value="series"; s.dispatchEvent(new Event("change",{bubbles:true}));')
            web.click('input[aria-label="筛选当前系列型号"]')
            for letter in "3588":
                keys(web, letter)
            assert web.js("return document.querySelectorAll('button[data-model]').length===2")
            web.click('button[data-model="RK3588S"]')
            web.click('button[data-medium="eMMC"]')
            assert web.js("""return [...document.querySelector('select[aria-label="具体 RK 板型"]').options]
              .some(o=>o.value==='LubanCat-4 (RK3588S)') && !document.querySelector('.v2-foot-actions .v2-primary').disabled""")
            checked("RK 系列内筛选、主控识别帮助、未适配介质和选填板型")
            web.click(".target-vendors button", "恩智浦")
            assert web.js("""return [...document.querySelectorAll('[data-model]')].map(e=>e.dataset.model).sort().join(',')==='i.MX6ULL,i.MX8,i.MX9'""")
            keys(web, "\ue009", "k")  # Control+K via real WebDriver keyboard events.
            wait(lambda: web.js("return document.activeElement.matches('.imager-board-search input')"), "search focus")
            for letter in "f101":
                keys(web, letter)
            wait(lambda: web.js("return document.querySelectorAll('[data-model]').length===1"), "F101 search")
            web.click('button[data-model="F101"]')
            assert web.js("return document.querySelectorAll('[data-medium]:disabled').length===3 && document.querySelector('.v2-foot-actions .v2-primary').disabled")
            web.click('button[data-medium="SPI NOR"]')
            capture(web, "production-board")
            checked("厂商筛选、Ctrl+K、输入搜索、选择 F101 与继续按钮")
            web.click(".v2-foot-actions .v2-primary")
            web.click(".v2c-tabs button", "本地文件")
            web.shown("选择镜像…")
            assert web.js("return document.querySelector('.v2-basic').dataset.step==='2'")
            assert web.js("return document.querySelector('.v2-foot-actions .v2-primary').disabled")
            capture(web, "production-image")
            checked("本地镜像入口与缺文件禁用下一步")

            web.click('button[aria-label="任务记录"]')
            wait(lambda: web.find(".imager-records[open]"), "task history")
            web.shown("尚无任务记录")
            capture(web, "production-task-records")
            keys(web, "\ue00c")  # Escape
            wait(lambda: not web.find(".imager-records[open]"), "close history by keyboard")
            web.click('button[aria-label="设置"]')
            wait(lambda: web.find('[data-testid="settings-panel"]'), "settings")
            wait(lambda: web.find('[data-testid="settings-theme-style"]'), "theme settings")
            web.js("""const language=document.querySelector('[data-testid=settings-language]');
              language.value='zh-CN';language.dispatchEvent(new Event('change',{bubbles:true}));""")
            wait(lambda:web.js("""return document.querySelector('#imager-settings-title')?.innerText==='设置' &&
              ['settings-overlay','settings-panel'].every(k=>
                Number(getComputedStyle(document.querySelector('[data-testid="'+k+'"]')).opacity)>=0.999)"""),
              'Chinese settings and completed dialog transition')
            capture(web, "production-settings")
            theme = '[data-testid="settings-theme-mode"]'
            def select_theme(value):
                web.js("""const select=document.querySelector(arguments[0]);
                  select.value=arguments[1];select.dispatchEvent(new Event('change',{bubbles:true}));""",theme,value)
            select_theme("dark")
            web.click(".settings-footer .settings-btn-primary")
            wait(lambda: web.find('[data-testid="settings-save-success"]'), "save native preferences")
            files = list((temp / "HOME").glob(".lynx-imager*/settings.json"))
            assert len(files) == 1 and json.loads(files[0].read_text())["themeMode"] == "dark"
            web.click('[data-testid="settings-close"]')
            wait(lambda: not web.find('[data-testid="settings-panel"]'), "close settings")
            web.click('button[aria-label="设置"]')
            wait(lambda: web.js("return document.querySelector(arguments[0])?.value==='dark'",theme),
                 "reopen saved native preferences")
            select_theme("light")
            keys(web, "\ue00c")
            wait(lambda: not web.find('[data-testid="settings-panel"]'), "cancel preference preview")
            assert web.js("return document.querySelector('.imager-themed').dataset.theme==='dark'")
            assert json.loads(files[0].read_text())["themeMode"] == "dark"
            checked("任务记录 Esc 关闭、设置保存至磁盘、重新打开与撤销主题预览")

            report["temporaryBoardId"] = offline_board(web)
            web.click('nav[aria-label="工作区"] button', "高级功能")
            assert_scope(web, "高级功能")
            wait(lambda: web.find('input[aria-label="搜索单板"]'), "advanced board search")
            advanced(web, capture)
            checked("临时离线板：设备概览、ADB 文件传输、串口、SSH、固件、设备设置与禁用操作")
            web.click('nav[aria-label="工作区"] button', "连接 AI")
            assert_scope(web, "连接 AI")
            for key, label, heading in [("local","本机 MCP","本机 MCP 通道"), ("remote","远端 SDK","远端 SDK 通道"),
                                       ("services","AI 服务","AI 服务"), ("policy","策略与安全","策略与安全"),
                                       ("approval","请求确认","请求确认"), ("notifications","通知","通知")]:
                web.click(".im-ai-rail button", label)
                wait(lambda: web.js("return document.querySelector('.im-ai-body h1')?.innerText===arguments[0]", heading), heading)
                wait(lambda: web.js("return !document.querySelector('.im-ai-body').innerText.includes('正在读取配置')"), heading+' settings loaded')
                capture(web, "production-ai-"+key)
            checked("高级功能单板入口、连接 AI 的六个子页")
            web.click('nav[aria-label="工作区"] button', "烧录系统")
            assert_scope(web, "烧录系统")
            assert web.js("return document.querySelector('.v2-basic').dataset.step==='2'")
            web.click(".v2-foot-actions button", "← 上一步")

            before = native_window(web)
            web.call("POST", web.session + "/window/rect", {"width": 1090, "height": 720, "x": 0, "y": 0})
            wait(lambda: web.js("return innerWidth < arguments[0]", before["w"]), "native resize")
            capture(web, "production-compact")
            checked("切回烧录保留已选板型、紧凑窗口无水平溢出")
            final_tasks = web.ipc("workbench_list_tasks", {"limit": 100})
            assert {item["taskId"] for item in initial_tasks} == {item["taskId"] for item in final_tasks}
            checked("检查期间任务记录无新增")

        with launch(binary, desktop_scale=2) as (web, _temp):
            assert_scope(web, "烧录系统")
            shape = native_window(web)
            assert shape["dpr"] >= 1.9, shape
            web.click(".target-vendors button", "意法半导体")
            assert web.js("return [...document.querySelectorAll('[data-model]')].map(e=>e.dataset.model).sort().join(',')==='STM32MP1,STM32MP157'")
            capture(web, "production-hidpi")
            checked("GTK 原生 2× HiDPI、可操作筛选与无水平溢出")
    finally:
        (out / "production-smoke.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    run(args.binary, args.output)
