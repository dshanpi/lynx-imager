# IMAGER-UI-RESET-001：解除 UI 设计 Skill 约束

日期：2026-11-24。起点：`0cbc269c82ac9e3d32fc999fca90b83e54ddb371`，初始工作区干净。
用户要求：移除 lynx-imager 工程中所有 UI 设计相关 Skill 约束，为重新生成页面准备。

## 删除

- 仓库内 UI/UX Pro Max、Color System、Frontend Design、Web Design Guidelines 四套设计 Skill，包含其数据、脚本、参考资料和随附许可证。
- 这些副本的来源锁和第三方设计技能清单；移除的是整套副本，没有保留副本却删除署名。
- `skills/LYNX-UI-CONSTRAINTS.md`、`design-system/lynx-imager/MASTER.md`。
- `scripts/validate-ui-constraints.mjs`、`scripts/check-ui-scale.mjs`。
- npm、构建、CI、verify-changed 和当前 macOS 构建说明中的对应检查入口。
- 浏览器测试里品牌文字高度上限和固定 44px 导航断言；启动布局检查中固定 56/224px 侧栏宽度断言。

## 修改与新增

- `AGENTS.md` 改为工程说明，明确不再绑定设计 Skill 或旧视觉规范。
- SerialTool 的 `SKILL.md` 转为普通组件 `README.md`，去掉固定布局配置段，保留功能、数据流与接口说明。
- 新增样式目录说明；旧效果图、原型和设计资料标记为历史参考，修复删除主规范后的链接。
- 保留视口溢出、操作按钮可见性、业务状态等通用检查，它们不限定配色或具体布局尺寸。

## 保留范围

本次不重绘页面、不删除 CSS/主题实现，不更改硬件操作和后端业务。
运行时 Linux/MCU 技能、技能资源校验、IPC、输入安全、类型检查、业务测试和发布签名检查继续保留。
历史 RC6 的验收与发布回执保留当时事实，不追改旧发布。
仅移除本仓库中的 UI 设计技能，不修改本机全局 Skill 配置。

## 验证

- 残留引用扫描：PASS。四套设计 Skill 及其缓存已移除；活动构建、CI、工程说明与预览链接不再引用被删规范或检查命令。
- `npm run typecheck`：PASS。
- `npm run test:imager`：20/20 PASS。
- `npm run build`：PASS，包含输入安全、7 个内置运行时技能资源索引、macOS 包配置及 Vite 生产前端构建。保留原有大 chunk 警告。
- 修改过的 JavaScript、Python 和 shell 脚本语法检查、Prettier 与 `git diff HEAD --check`：PASS。
- 应用 CSS、主题变量、Rust 后端、IPC 和运行时设备技能文件无变更。未重新执行原生桌面/HIL 测试。

日志保存在本地 `artifacts/ui-skill-reset-20261124/`。初次验收时变更留在本地；2026-11-24 用户确认整理上传后，将此清理作为独立提交纳入源码交付。
本次不重新生成安装包，不改写已有 Release。
