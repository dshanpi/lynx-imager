# LYNX Imager 发布与更新

## macOS 手动安装候选通道

`v0.1-rc6` 增加 macOS 26.0+ Apple Silicon 候选包，按 [macOS 构建说明](imager-macos.md) 执行。
该候选包未 Apple 公证，采用 ad-hoc 包签名与独立 macOS 分发文件签名；不调用下述 Windows/Linux feed 生成器，不修改其公钥或 `latest.json`。
先上传 Draft，再逐资产反向下载校验后公开为 Pre-release；不覆盖 rc5，不宣称其为正式公证版或本轮已验证 Windows/Linux 二进制。
只有补齐 Apple 签名、公证、安装升级及硬件门禁后，才能转入正式 macOS 发布流程。

## 烧录目标

厂商横栏依次提供全志、瑞芯微、恩智浦、意法半导体和嘉楠。选中厂商后，左栏显示系列，右侧列表显示该系列的型号、烧录路线和目标介质。全局搜索匹配芯片与板名，系列内搜索只筛选当前系列。支持矩阵由 `targetCatalog.ts` 维护。

选择型号后按板上实际存储选择介质，并查看该路线需要准备的镜像、Loader 或板级配方。灰色项表示当前缺少对应的执行配方。更换目标会清除原镜像、设备绑定选择和烧录确认。K230 在设备连接阶段再次确认同一介质。NXP、ST 的实际写入区域由板级 UUU 脚本或 DFU Alternate 决定。

## 产品身份

| 项目 | 值 |
| --- | --- |
| 仓库 | `dshanpi/lynx-imager` |
| 应用标识 | `com.dshanpi.lynximager` |
| Windows 程序 | `lynx-imager.exe` |
| Windows 安装包 | x64 NSIS，安装到当前用户目录 |
| Linux 安装包 | amd64 Debian 包，依赖 GTK 3 和 WebKitGTK 4.1 |
| OTA 入口 | `https://raw.githubusercontent.com/dshanpi/lynx-imager/main/updates/latest.json` |
| 发布标签示例 | `v0.1-rc0` |
| 程序及 OTA 版本 | `0.1.0-rc.0` |
| Debian 包版本 | `0.1.0~rc.0`，排序早于正式版 `0.1.0` |

## 发布次序

1. 确定版本并运行 `npm run sync-version` 和 `npm run validate:release -- <tag>`。保留此产品的应用标识和 OTA 公钥。私钥由发布者在工程目录外保管。
2. 运行 TypeScript、Imager 单元测试、输入安全、UI 尺寸与颜色、IPC 检查。Ubuntu 下在 Tauri 窗口操作目标选择、镜像分析、设备连接、确认、失败恢复、成功、高级功能和连接 AI。记录模拟设备、原生 IPC 和实机分别覆盖的范围。
3. 用发布配置构建 amd64 Debian 和 x64 NSIS。Debian 预发行包先用 `scripts/imager-debian-rc.sh` 规范包管理器版本。解包确认产品名、主程序、安装类型标记及随附工具，并核对 Windows OpenixCLI 的已验收摘要。用解出的 Ubuntu 程序进行原生窗口回归。
4. 使用同一把 Imager OTA 密钥对两份最终安装包执行 `tauri signer sign`。使用 `node scripts/imager-ota-feed.mjs <tag> <目录> <exe 文件名> <deb 文件名>` 复核签名并生成 `latest.json` 和 `SHA256SUMS.txt`。修改包后须重新签名。
5. 在隔离环境中用 `check_imager_ota` 运行真实 Tauri 更新器，分别检查较低版本、同版本、Windows 和 Linux 目标下载，并用篡改包验证拒绝行为。首版可以使用测试版本号模拟较低版本。真实覆盖安装与下一版对上一版的升级另行记录。
6. 将版本来源推送到此仓库。在绑定该提交的 GitHub Release 上传两份安装包、两份签名、清单和校验值。RC 勾选 **Pre-release**。公开后验证匿名下载可用且文件摘要相符，再将清单原样提交到 `main/updates/latest.json`。该地址也能分发预发行版。
7. 用内置更新源再次运行更新器。已安装版本只接受严格更高的 SemVer。后续从 RC 到正式版沿用同一信任密钥、公网入口和平台类型。Windows Authenticode 发布者证书与此处的 OTA 密钥分别管理。
