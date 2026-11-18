# LYNX Imager UI constraints

本文件是桌面产品设计门禁。评审时同时打开 [工作台主规范](../design-system/lynx-imager/MASTER.md)、[颜色与尺寸令牌](../design-system/lynx-imager/tokens.json) 和当前页面原型。

## Sources · 四项职责

| 工程内 Skill | 负责的决定 | 交付物与来源 |
| --- | --- | --- |
| [UI/UX Pro Max](ui-ux-pro-max/SKILL.md) | 按设备与工程任务确定信息层级、密度、控件与键盘路径 | 页面流程与布局规格。[源仓库](https://github.com/nextlevelbuilder/ui-ux-pro-max-skill) |
| [Frontend Design](frontend-design/SKILL.md) | 把规格落实为桌面布局、对齐、文案、控件及状态样张 | 每个页面的可检查设计图与之后的界面实现。[源仓库](https://github.com/anthropics/skills/tree/main/skills/frontend-design) |
| [Color System](color-system/SKILL.md) | 在原 LYNX - Light 和 LYNX - Dark 中分配颜色角色，校验状态、焦点与图表对比度 | 配色令牌、对比度检查与多重编码样例。[源仓库](https://github.com/AntJanus/skillbox/tree/main/skills/color-system) |
| [Web Design Guidelines](web-design-guidelines/SKILL.md) | 在评审时获取上游规则，核对可访问名称、表单语义、焦点、交互反馈与溢出 | 对改动文件记录检查结果及可定位的问题。[源仓库](https://github.com/vercel-labs/agent-skills/tree/main/skills/web-design-guidelines) |

## Product structure · 产品结构

- 三个顶层入口为「烧录系统」「高级功能」「连接 AI」。打开应用即进入「烧录系统」，依次完成「选择板子」「选择系统」「设备连接」「烧写系统」。
- 四步流程只展示在向导导航中。内容区直接显示当前步骤的操作。「烧写系统」中的确认、执行与结果共用同一任务与后端配置。
- 「高级功能」沿用 LYNX Linux 单板工作台。左侧选择单板，右侧依次提供「设备概览」「串口终端」「SSH 终端」「固件烧录」「AI 通道」。ADB 文件传输、属性和运行监控位于设备概览。
- 「连接 AI」按执行主机区分本机 MCP 与远端 SDK，分别展示其地址、验证和复制动作。地址与动态端口跟随服务的真实状态。
- 产品外壳只呈现上述三个工作区，开户、学习、课程与历史全局工作台导航不属于这里的页面入口。

## Layout contract · 布局

- 应用导航 44 px，左栏 208 px，设备上下文 40 px，工具标签 36 px，行高和控件高 32 px，底部状态 24 px。全局间距和字阶见主规范。
- 同一屏只展示该工作区需要的一种左栏。当前设备、目标工具、下一步操作依次分层，数据区占用其余空间；宽度优先让给文件路径、终端与日志。
- 内容左缘、标签和属性列共用对齐线；数据值与单位对齐。按钮名称说清对象，单个任务区域只突出一个主操作。
- 逐一检查 1090 × 720、1280 × 768 与宽桌面，记录窄窗口行为。文字缩放、高对比度和纯键盘操作纳入人工核对。

## Visual and content contract · 颜色与反馈

- 设计样张引用 `design-system/lynx-imager/tokens.css`，其浅／深模式逐个角色对应同目录的 `tokens.json`。主操作、选中、焦点、在线、警告与错误各有独立角色。
- 两种模式共用原版 LYNX 的背景、侧栏、表面、正文、选中底纹、品牌蓝和状态标记。终端使用原主题独立的前景、背景。每次运行门禁都从[原版色表](../src/Themes/configs/lynx.json)和[原版语义 CSS](../src/Themes/tokens.css)读取相应值。
- 浅色品牌蓝为 `#087FF5`，用于品牌、选中指示线、焦点和 CPU 曲线。主按钮的白字配原主题的 `accentHover`（`#006DDE`）。状态小标记放在 `surface` 上，旁边同时显示有独立颜色的状态文字。浅色次文字与状态文字允许按对比度加深，分别在工作区、侧栏、表面、弱底色和选中底色上核对。
- 次文字、状态字与主按钮标签至少 4.5∶1；关键控件边界、焦点环、选中指示线及数据曲线至少 3∶1。装饰分隔线和决定输入框位置的控件边界分别取值。
- 同一上下文只显示一次同一状态。连接状态在邻近对象处给出名称；选中设备有底纹和实线指示；图表的 CPU 使用实线、内存使用虚线，同时标示名称、数值与单位。
- 请求期间禁用相互冲突的操作并显示进度。阻塞、失败与断连显示具体原因、目标及已有恢复动作。写入、校验、重启和启动验证分别记录证据。

## Runtime and review contract · 验收

- 接入现有 React/Tauri、IPC、各厂商镜像和 Loader 配方。沿用后端介质、物理校验、重启和 MCP 权限语义。
- 原型评审使用标注的样例数据，逐页核对视觉、选择、键盘焦点、路径与日志的可读性。运行时验收在原生桌面复核真实行为。
- 代码变更运行 `npm run check:ui-constraints`、`npm run check:ui-scale`、`npm run typecheck` 和 `npm run test:imager`。记录命令结果，并注明仍需人工或硬件检查的项目。
