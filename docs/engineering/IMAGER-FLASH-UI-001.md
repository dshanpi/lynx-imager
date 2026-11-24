# IMAGER-FLASH-UI-001：烧录系统绿色浅色 UI

- 日期：2026-11-24
- 源码基线：`0cbc269c82ac9e3d32fc999fca90b83e54ddb371`，应用版本 `0.1.0-rc.6`
- 用户范围：只调整烧录系统 UI，绿色主色、浅色主题；不修改已有后端业务逻辑；编译运行并展示实际窗口。
- 当前结果：前端实现、回归测试和原生窗口走查完成。用户于 2026-11-24 确认设计并要求整理上传；源码与本记录纳入提交，安装包仍为本地预览，不构成正式发布。

## 变更清单

| 文件 | 变更 |
| --- | --- |
| `src/Components/Imager/FlashLight.css` | 新增烧录工作区专用浅色样式。绿色主操作、步骤导航、厂商与系列筛选、型号卡片、介质选中和不可用状态、镜像表单及底部固定操作区。覆盖设备连接、确认、执行与结果页的视觉样式。 |
| `src/main.tsx` | 引入上述样式。 |
| `src/Components/Imager/TargetPicker.tsx` | 调整展示文案及卡片 class；为未适配型号显示明确的路线和介质提示。 |
| `src/Components/Imager/ImagerPage.tsx` | 增加四步导航说明和当前步骤的继续条件提示。 |

将烧录页原有深色列表呈现调整为浅色横向筛选和卡片；没有删除功能、设备型号、烧录路线或后端接口。所有业务处理函数、状态流转和确认门禁保持原样。样式限定在 `v2-basic-shell`，没有重做高级功能或连接 AI 页面。为了展示浅色原生窗口，通过现有设置入口将本机主题偏好设为浅色。

工作树中此前用户要求的 UI skills 清理属于前一任务，不算作本次功能增删，未覆盖或回退。

## 验证与证据

证据根目录：`artifacts/flash-green-ui-20261124/`。

| 检查 | 结果与证据 |
| --- | --- |
| 后端与 IPC 变更范围 | PASS。`scope-check.json`：后端/IPC 未改；两个 TSX 文件的业务逻辑前缀与基线相同。 |
| TypeScript、生产前端构建 | PASS。`typecheck.log`、`build.log`；构建保留既有大 chunk 提示。 |
| ESLint | 0 error，见 `lint.log`。 |
| 单元与组件检查 | 20/20 PASS，见 `unit-tests.log`。 |
| IPC 契约门禁 | PASS，见 `ipc.log`。 |
| 1280×768、1090×720 页面回归 | 两种尺寸各 27 项检查、41 个界面状态，页面异常均为 0；见 `final-1280/pages.json` 与 `final-1090/pages.json`。 |
| 原生 macOS 编译 | PASS，见 `native-build-sdk26.log`。使用本机 SDK 26.5 避开 SDK 27 与现有链接器的不兼容。 |
| 本地预览打包 | PASS，见 `package.log`、`macos-preview/PACKAGE-CHECK.json`。使用项目已有打包脚本嵌入 libusb 并 ad-hoc 签名，解决原始 app 的动态库签名启动错误。 |
| 原生窗口走查 | PASS：实际启动 Tauri app，检查厂商/型号切换、未选介质禁止继续、选定 RK3588/eMMC 后进入镜像页、未选镜像禁止继续、主线整盘切换后保留 Loader 要求。 |

页面回归使用 Chromium 和隔离内存 IPC 数据，覆盖文件选择、USB 状态、预检、人工确认、写入进度、成功报告及失败恢复，也回归其他工作区入口。它不是硬件烧录结果。

原生截图：

- `native-home.png`：烧录首页。
- `native-rk-catalog.png`：瑞芯微型号卡片。
- `native-rk-target.png`：RK3588/eMMC 目标选择。
- `native-image-picker.png`：厂商镜像选择。
- `native-raw-image.png`：主线整盘及 Loader 要求。

预览应用：`macos-preview/stage/LYNX Imager.app`。预览 DMG：`macos-preview/LYNX-Imager_0.1.0-rc.6_aarch64.dmg`，校验见同目录 `SHA256SUMS.txt`。该本机产物最低 macOS 版本为 26.0。

## 验证边界与后续

没有执行真实设备烧录、清洁安装或升级；没有完成 Developer ID 签名、公证和更新器签名；不声明正式发布通过。打包脚本生成的 `nativeDesktopSmoke: NOT VERIFIED` 是打包时记录，此后的有限原生走查由本文和上述截图单独记录，不将其提升为完整硬件验收。

已知轻微文案残留：型号列表末尾的旧说明仍以“—”解释无适配路线，而卡片现在使用“尚无适配路线”文字；不影响型号禁用逻辑，下一轮文案整理统一处理。

回退本次设计只需撤销上述四个文件中的本次变更；不要一并恢复先前按用户要求删除的 UI skills。

## 源码交付范围

目标为原源码仓库 `dshanpi/lynx-imager` 的 `main`。将设计约束清理与绿色烧录 UI 分成两个提交，便于独立审查和回退。保留版本 `0.1.0-rc.6`，不创建 Release 或版本标签。

按仓库 `.gitignore` 约定，截图、原始日志、安装包不提交到 Git；上述测试结论和证据目录记录随源码提交，本机完整证据继续保留。推送前另执行 `npm run verify:changed`，结果见本地 `pre-push-verify.log`。

提交前复核：Prettier、380 个命令/31 个事件的 IPC 契约、ESLint 和 TypeScript 均通过。聚合脚本随后在 `cargo fmt --check` 停止，报告 9 个既有 Rust 文件的格式差异；这些文件及整个 `src-tauri` 与原始基线完全一致。本次不修改后端来消除历史格式差异，也不将该聚合检查标为通过。源码上传不代表发布门禁全通过。
