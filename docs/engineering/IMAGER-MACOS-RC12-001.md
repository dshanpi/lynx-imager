# IMAGER-MACOS-RC12-001：最新代码核对与 macOS 候选

日期：2026-11-26。用户要求拉取最新代码、编译测试、检查全部 UI 与功能、优化并上传 macOS 安装包。

## 来源与范围

本机从 `d6cd3d7` 快进至远端 `main` 的 `3992142`，保留 rc11 的系列布局、读卡器、eMMC、SDK 和键盘焦点改进。使用未占用的 `v0.1-rc12`。本次不修改 100ask 网站、不操作远端服务器、不更新 Windows/Linux feed，也不执行真实硬件写入。

## 修改、新增、删除

| 类型 | 内容 |
| --- | --- |
| 修改 | `HostCardWriter.tsx` 按执行／失败／成功状态显示标题；异常在当前任务内保留错误与恢复指引。 |
| 新增 | 写卡失败提示样式；重新检测入口；30 项交互检查中的写卡进行中、失败恢复和长设备名断言。 |
| 删除 | 失败后继续展示原“擦写所选卡”按钮的路径；恢复必须重新扫描并重选设备。无页面或后端能力删除。 |
| 发布 | rc12 版本元数据、README macOS 链接、发行说明与本记录。 |

本轮无业务依赖新增，Rust 业务源与 IPC 合同未修改。测试工具 Pillow 只安装在忽略的 `.verification/python` 环境，不进入应用或生产依赖。

## 验证证据

本地目录：`artifacts/macos-audit-20261126/`。公开包只包含安装文件、校验材料与经过选取的截图／报告，不包含私钥、用户数据或测试镜像。

| 检查 | 结果／证据 |
| --- | --- |
| Rust 单元测试 | 727 PASS、6 ignored，`rust-tests.log`；后续仅变更包版本，无 Rust 业务修改。 |
| 前端测试 | 20 PASS，`imager-final.log`。 |
| 发布资产测试 | 5 PASS，`release-tests-final.log`。 |
| 类型／Lint／格式／IPC | PASS，`*-final.log`；388 命令、33 事件。 |
| 生产构建 | `frontend-final.log`、`native-build.log` PASS；保留既有较大 chunk 警告。 |
| 页面回归 | `final-1090/pages.json` 与 `final-1280/pages.json`，各 30 检查／53 状态，0 页面错误。 |
| 视觉 | 检查目标、镜像、连接、确认、执行、失败、结果、擦除、读卡器、工作台、AI、设置和记录截图；焦点和主要操作按钮均在可用窗口区域。 |
| 原生 | `native-*.png`：真实打包程序 rc12；本地 1 MiB 测试文件解析，SHA-256 `30e14955ebf1352266dc2ff8067e68104607e750abb9d3b36582b8af909fcb58`；扫描无卡时继续按钮禁用。 |
| 包检查 | `package/PACKAGE-CHECK.json`；arm64、最低 macOS 26.0，16 项 Mach-O 检查、0 非系统绝对依赖、DMG 挂载与摘要 PASS。 |
| 格式限制 | `rust-format.log`：10 个既有 Rust 文件格式差异，未将全量格式／正式发布门禁标为 PASS。 |

原生验证创建了一个没有接口绑定的 `QA-macOS-rc12（）` 本地设备用于检查空状态，不代表在线板卡；未发送串口命令、SSH 请求、AI 请求、烧录或擦除。自动化使用隔离模拟 IPC，不写用户配置。深色截图为主题样式冒烟，不能替代全部主题的原生对比度验收。

## 发布与回退

沿用 macOS 手动候选流程：本地打包 → 独立分发签名 → 提交推送 → Draft 上传 → 每个附件反向下载比对 → 公开 Pre-release。`LYNX Imager CI` 保持 disabled；main 只有此工作流，不修改其他分支的工作流。未调用 CI。

用户可回退到原 rc9 安装包；本轮未作数据库 schema 迁移或 OTA 信任根变更。Apple Developer ID／公证、真实硬件、安装升级及在线服务验收仍未完成。最终提交、资产摘要和读回结果记录在 Release 附件及本地 `readback-verification.json`。
