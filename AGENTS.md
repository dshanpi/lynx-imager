# 桌面界面工作流

设计和审查界面时，从[项目设计约束](skills/LYNX-UI-CONSTRAINTS.md)和[工作台规范](design-system/lynx-imager/MASTER.md)开始。两种主题共用同一套语义角色，尺寸和颜色以 [tokens.json](design-system/lynx-imager/tokens.json) 为准。

| 阶段 | 工程入口 | 交付证据 |
| --- | --- | --- |
| 布局与交互 | [UI/UX Pro Max](skills/ui-ux-pro-max/SKILL.md) | 结合板卡选择、设备工具或 AI 通道的实际任务确定信息优先级、布局和各阶段反馈。 |
| 色彩 | [Color System](skills/color-system/SKILL.md) | 为浅色、深色和终端背景分配颜色角色，状态用文字或图形表达，遥测曲线补充线型和图例。 |
| 界面实现 | [Frontend Design](skills/frontend-design/SKILL.md) | 依据已确定的尺寸和组件组织实现页面，在原生窗口检查控件对齐、信息密度、滚动范围和主要操作。 |
| 质量审查 | [Web Design Guidelines](skills/web-design-guidelines/SKILL.md) | 审查改动文件，记录本次使用的上游规则版本和带文件行号的发现，验证焦点、输入、错误与空状态。 |

`npm run check:ui-constraints` 在本地构建和 CI 中离线核对四个 Skill 的锁定副本、两种主题的 JSON 与 CSS、文字及图形对比度，以及现有导航和厂商入口。`npm run check:ui-scale` 检查应用样式中的字体尺度。截图评审、键盘操作和设备验收分别记录实际观察到的结果。

[来源锁](skills/sources.lock.json)记录各副本的上游路径、提交和许可位置。随仓库分发的作者署名与许可正文见[第三方设计技能许可](skills/THIRD-PARTY-NOTICES.md)。更新 Skill 时先对照该提交，再更新摘要。摘要按相对路径排序，对每个文本文件统一 LF 后计算 SHA-256，再对各行“路径 + NUL + 文件摘要 + LF”计算一次 SHA-256。Python 运行缓存沿用工程忽略规则。
