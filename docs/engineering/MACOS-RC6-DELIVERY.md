# macOS RC6 发布回执

日期：2026-11-24。

- Release：https://github.com/dshanpi/lynx-imager/releases/tag/v0.1-rc6
- 状态：公开 **Pre-release**，不是 Draft，不声明为公证正式版。
- 标签提交：`f8a438487ae6a6c6290a0bef6c6baa67c8083f44`。
- 平台：macOS 26.0+ / Apple Silicon arm64；本机系统 26.6.2。
- 源码 CI：https://github.com/dshanpi/lynx-imager/actions/runs/45321192975 ，结果 success。
- 包名：`LYNX-Imager_0.1.0-rc.6_aarch64.dmg`。
- 大小：32,791,369 字节。
- SHA-256：`ef1139f630a6609c8c43f7e71a2f976814f4f5f9d4abd44310a63e1144b2415f`。
- 最终安装主程序 SHA-256：`7d8cf1dc2aba78698c06c2b7e78403f0dff1dacc1fe20a1743aba8345f4a7685`。

## 上传与读回

9 个附件先上传 Draft；全部反向下载、逐字节比较、SHA-256 检查通过；DMG 独立分离签名使用仓库公钥验证通过。
公开后再次使用无认证 curl 下载所有 9 个附件，逐字节与本地交付文件比较，9/9 通过。
没有替换旧 Release、没有上传私钥、没有改变 Windows/Linux 的 `updates/latest.json` 或 OTA 公钥。

附件包含 DMG、DMG.sig、公钥、SHA256SUMS、PACKAGE-CHECK、VALIDATION、使用说明、变更记录和 UI 证据 zip。
UI zip 共 328 张截图，均为隔离模拟 IPC；说明与每个尺寸的 pages.json 一起提供，不作为设备烧录证据。

## 安装及工具检查

最终 DMG 只读挂载后复制安装到用户 Applications；签名复核、主程序摘要一致、原生启动成功。
最终安装版的 OpenixCLI 0.2.11、OpenRockCLI 1.3.0、UUU 1.5.243、dfu-util 0.11 和 OpenOCD 均以版本/帮助命令启动成功，退出码 0。
未传入镜像或设备写入参数，没有执行实体烧录。

## 仍未通过的正式发布条件

Apple Developer ID / 公证、干净系统安装、跨版本升级、macOS OTA、实体板卡烧录/回读/冷启动，以及 Intel/旧 macOS 均未验证。
本轮仅完成明确上述边界的 macOS 手动安装候选发布。
如需回退，在确认任务空闲并退出应用后恢复之前保留的应用副本；不要删除学习/设备配置或正在使用的数据。

原始构建、测试、原生交互和读回记录保留在本机 `artifacts/macos-ui-20261124`；公开附件不包含系统凭据或私有项目源码。
