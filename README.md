# LYNX Imager

LYNX Imager 是面向开发板系统镜像烧录的开源桌面软件。它用基础向导帮助普通用户完成烧录，用高级工作台保留 SPI NOR、NAND、eMMC、Loader 和结束动作等专业能力，并把 AI 作为受策略保护的调用入口。

当前仓库已经从离线 UI 原型进入 Tauri 桌面工程阶段：前端采用 React + TypeScript + Vite，桌面后端采用 Rust + Tauri 2，烧录与设备能力沿用现有 OpenixCLI、FEL/FES、USB、串口、ADB 和 MCP IPC 边界。

## 工作区

- **基础烧录**：选择板子、选择镜像、配置接口、确认后写入和校验。
- **高级功能**：打开现有专业工作台，配置 SPI NOR、NAND、eMMC、Loader、物理回读和结束动作。
- **连接 AI**：进入现有 AI 配置与监督中心；AI 只能提出动作，设备作用域、门禁和人工确认由 LYNX 决定。

## 开发

```bash
npm install
npm run typecheck
npm run test:ui-contract
npm run dev
```

`npm run dev` 只启动 Vite 前端预览。完整桌面运行需要 Tauri 环境：

```bash
npm run tauri -- dev
```

完整桌面应用必须通过 Tauri 窗口和 Rust 后端验证，不能把浏览器预览当成软件验收。

## 目录

```text
src/Components/Imager/   LYNX Imager 三工作区入口和基础烧录向导
src/Components/Workbench/现有专业工作台与硬件交互
src-tauri/src/            Rust IPC、硬件安全门禁、OpenixCLI 和任务执行
prototype/                原始离线 UI 原型和 17 页面状态机
docs/design/              产品流程、页面效果图、安全规则和验收标准
```

## 设备安全边界

任何擦除、写入、校验、重启、电源和串口动作都必须经过后端硬件安全门禁。基础向导不会自动选择其他设备、自动重试失败任务或用协议进度冒充系统启动成功。写入完成、物理回读、重启完成和启动验证分别记录。

## 当前状态

本阶段已完成真实桌面工程迁移和基础向导接入：设备列表使用 `workbench_list_devices`，烧录使用现有 `workbench_start_flash`，任务状态订阅现有 `workbench-task-event`。高级和 AI 工作区通过既有工作台进入；后续继续优化 UI 和补齐真实桌面验收。
