# IMAGER-001：基础烧录向导接入现有后端

## 用户价值

普通用户可以在一个连续流程里选择开发板、选择镜像、绑定已保存设备，并在一次明确确认后提交烧录任务；专业用户和 AI 工具仍通过既有工作台进入，不产生第二套烧录协议。

## 范围

- 基础工作区：四步向导和板型选择。
- 支持示例路线：全志厂家镜像、全志 SPI NOR 主线、RK3588 eMMC 主线。
- 只读设备读取：调用 `workbench_list_devices`，按芯片厂商过滤已保存设备。
- 烧录提交：调用 `workbench_start_flash`，保留 Loader、路线、校验和烧后重启参数。
- 任务状态：订阅 `workbench-task-event`，展示任务 ID 和终态。

## 明确不在范围内

- 不复制或改写 OpenixCLI、FEL/FES、NAND、eMMC 和 SPI NOR 底层协议。
- 不在前端模拟设备在线、烧录成功或系统启动成功。
- 不自动扫描并选择其他 USB 设备，不自动重试失败任务。
- 不把高级工作台或 AI 监督器的逻辑复制进基础向导。

## Given / When / Then

### IMAGER-001-A：路线选择

- Given 用户选择 F101，When 进入系统步骤，Then 路线固定为 `mainline_spinor`，并要求匹配 Loader。
- Given 用户选择 T113-Pro，When 进入系统步骤，Then 使用厂家镜像路线，不附加 SPI NOR 或 eMMC 参数。
- Given 用户选择 RK3588，When 进入系统步骤，Then 路线固定为 `mainline_emmc`，并要求匹配 Loader。

### IMAGER-001-B：设备作用域

- Given 用户点击只读扫描，When `workbench_list_devices` 返回设备，Then 只展示与当前板型厂商匹配的已保存设备。
- Given 没有匹配设备，When 用户继续，Then 向导不能提交烧录任务，并引导打开原有设备设置。

### IMAGER-001-C：高风险动作

- Given 用户未勾选确认框，When 点击开始烧写，Then 前端不调用 `workbench_start_flash`。
- Given 用户已确认且存在设备、镜像和 Loader，When 提交任务，Then 参数包含 `verify: true` 和 `postFlashAction: reboot`，失败不自动重试。
- Given 后端返回任务事件，When 状态变化，Then 界面展示任务状态；协议进度不被解释为启动成功。

## 受保护边界

底层硬件安全门禁、固定板卡作用域、Linux/MCU 隔离、凭据保护、AI 策略和历史 Workbench 行为保持不变。真实桌面启动、Rust 编译、硬件在环和 Windows 安装尚未由本次迁移自动证明。
