/* LYNX Imager (V5) — Interactive UI Prototype & 17-Screen Canonical State Machine */

(function () {
  "use strict";

  // =========================================================================
  // 1. INLINE VECTOR ICONS & ENGINEERING BOARD ILLUSTRATIONS (ZERO EMOJIS)
  // =========================================================================
  const SVG = {
    brand: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="12 2 21 7 21 17 12 22 3 17 3 7"/><polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/></svg>`,
    history: `<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><polyline points="12 7 12 12 15.5 14"/></svg>`,
    help: `<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M9.5 9.25a2.6 2.6 0 0 1 5 .85c0 1.75-2.5 2.15-2.5 3.65"/><circle cx="12" cy="17.2" r="0.6" fill="currentColor"/></svg>`,
    settings: `<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.08a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.08a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.08a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>`,
    search: `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><line x1="20" y1="20" x2="16.2" y2="16.2"/></svg>`,
    check: `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>`,
    checkSm: `<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12"/></svg>`,
    alert: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>`,
    info: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>`,
    dot: `<svg width="10" height="10" viewBox="0 0 12 12" aria-hidden="true"><circle cx="6" cy="6" r="4" fill="currentColor"/></svg>`,
    hollowDot: `<svg width="12" height="12" viewBox="0 0 14 14" aria-hidden="true"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="2"/></svg>`,
    arrowRight: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="5" y1="12" x2="19" y2="12"/><polyline points="13 6 19 12 13 18"/></svg>`,
    arrowLeft: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="19" y1="12" x2="5" y2="12"/><polyline points="11 18 5 12 11 6"/></svg>`,
    external: `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="7" y1="17" x2="17" y2="7"/><polyline points="9 7 17 7 17 15"/></svg>`,
    file: `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>`,
    usb: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="10" cy="7" r="1.5"/><circle cx="18" cy="12" r="1.5"/><circle cx="12" cy="20" r="2"/><path d="M12 18V5"/><polyline points="9 7 12 4 15 7"/><path d="M12 15l6-3"/></svg>`,
    refresh: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>`,
    copy: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`,
    close: `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>`,
    chevronRight: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>`,
    chevronDown: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9"/></svg>`,
    lock: `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/></svg>`,
    pc: `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2"/><line x1="8" y1="20" x2="16" y2="20"/><line x1="12" y1="16" x2="12" y2="20"/></svg>`,
    chip: `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1.5"/><path d="M9 3v4M15 3v4M9 17v4M15 17v4M3 9h4M3 15h4M17 9h4M17 15h4"/></svg>`,
    server: `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="6" rx="1.5"/><rect x="3" y="14" width="18" height="6" rx="1.5"/><circle cx="7" cy="7" r="0.8" fill="currentColor"/><circle cx="7" cy="17" r="0.8" fill="currentColor"/></svg>`,
    winMin: `<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" stroke-width="1.2"/></svg>`,
    winMax: `<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="1.5" y="1.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>`,
    winClose: `<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><line x1="1.8" y1="1.8" x2="8.2" y2="8.2" stroke="currentColor" stroke-width="1.2"/><line x1="8.2" y1="1.8" x2="1.8" y2="8.2" stroke="currentColor" stroke-width="1.2"/></svg>`
  };

  function boardThumbSvg(kind) {
    if (kind === "f101") {
      return `<svg width="34" height="34" viewBox="0 0 36 36" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">
        <rect x="6" y="7" width="24" height="22" rx="3"/>
        <rect x="12" y="12" width="12" height="10" rx="1.5"/>
        <line x1="10" y1="26" x2="26" y2="26"/>
        <circle cx="9" cy="10" r="1" fill="currentColor"/>
        <circle cx="27" cy="10" r="1" fill="currentColor"/>
      </svg>`;
    }
    if (kind === "rk3588") {
      return `<svg width="34" height="34" viewBox="0 0 36 36" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">
        <rect x="5" y="6" width="26" height="24" rx="3"/>
        <rect x="11" y="11" width="14" height="14" rx="2"/>
        <path d="M14 11V8M18 11V8M22 11V8M14 28v-3M18 28v-3M22 28v-3"/>
        <rect x="5" y="14" width="3" height="8" rx="1"/>
      </svg>`;
    }
    return `<svg width="34" height="34" viewBox="0 0 36 36" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">
      <rect x="5" y="7" width="26" height="22" rx="3"/>
      <rect x="13" y="12" width="10" height="10" rx="1.5"/>
      <rect x="5" y="11" width="4" height="5" rx="0.8"/>
      <rect x="5" y="20" width="4" height="5" rx="0.8"/>
      <line x1="26" y1="11" x2="26" y2="25" stroke-dasharray="2 2"/>
    </svg>`;
  }

  function boardDiagramSvg(board) {
    const label = board.id === "f101" ? "SPI NOR · FEL" : board.vendor === "瑞芯微" ? "eMMC · Loader / Maskrom" : "NAND / eMMC · FEL";
    return `<svg width="248" height="72" viewBox="0 0 248 72" fill="none" aria-hidden="true">
      <rect x="18" y="8" width="212" height="56" rx="7" stroke="var(--border-strong)" stroke-width="1.4" fill="var(--bg-panel)"/>
      <rect x="14" y="20" width="10" height="14" rx="2" stroke="var(--ink-muted)" stroke-width="1.2" fill="var(--bg-app)"/>
      <rect x="14" y="38" width="10" height="14" rx="2" stroke="var(--ink-muted)" stroke-width="1.2" fill="var(--bg-app)"/>
      <rect x="86" y="18" width="42" height="36" rx="4" stroke="var(--ink-secondary)" stroke-width="1.4" fill="var(--bg-app)"/>
      <rect x="94" y="26" width="26" height="20" rx="2" stroke="var(--indigo)" stroke-width="1.2" fill="var(--indigo-tint)"/>
      <rect x="146" y="20" width="32" height="16" rx="2.5" stroke="var(--border-strong)" stroke-width="1.2" fill="var(--bg-app)"/>
      <rect x="146" y="40" width="54" height="12" rx="2" stroke="var(--border-strong)" stroke-width="1.2" fill="var(--bg-app)"/>
      <circle cx="214" cy="18" r="3" fill="var(--success)"/>
      <text x="148" y="31" font-size="7.5" fill="var(--ink-muted)" font-family="var(--font-mono)">${board.storageDefault}</text>
      <text x="149" y="48.5" font-size="7" fill="var(--ink-muted)" font-family="var(--font-mono)">${label}</text>
    </svg>`;
  }

  // =========================================================================
  // 2. STATIC CATALOGS & 17 CANONICAL PRESETS
  // =========================================================================
  const BOARDS = [
    {
      id: "t113-pro",
      name: "T113-Pro",
      vendor: "全志",
      chip: "T113-S3",
      subtitle: "全志 · T113",
      thumbKind: "t113",
      connectMethod: "USB 烧录",
      modeHint: "FEL",
      storageDefault: "SPI NAND",
      defaultRoute: "全志厂家镜像",
      hasOfficialResource: false,
      needsLoader: false,
      defaultImage: "t113-linux.img",
      defaultSize: "1.20 GB",
      defaultPath: "D:\\Images\\t113-linux.img",
      specSummary: "支持全志 FEL USB 烧录与厂家镜像、主线 NAND 组件路线；板载 SPI NAND / eMMC。"
    },
    {
      id: "f101",
      name: "F101",
      vendor: "全志",
      chip: "F1C100s / SPI NOR",
      subtitle: "全志 · SPI NOR",
      thumbKind: "f101",
      connectMethod: "USB 烧录",
      modeHint: "FEL",
      storageDefault: "SPI NOR",
      defaultRoute: "主线系统 · SPI NOR",
      hasOfficialResource: true,
      resourceName: "NekoBox",
      resourceSub: "Zephyr · SPI NOR 完整镜像",
      resourceFile: "nekobox-full.img",
      resourceSource: "GitHub Releases",
      needsLoader: true,
      loaderLabel: "RAM FES Loader",
      defaultLoader: "fes1.fex",
      defaultImage: "nekobox-full.img",
      defaultSize: "16.0 MB",
      defaultPath: "D:\\Images\\nekobox-full.img",
      specSummary: "板载 SPI NOR 闪存，主线系统烧录需先载入匹配板型的 RAM FES Loader（fes1.fex），并受物理布局保护。"
    },
    {
      id: "rk3588-series",
      isSeries: true,
      name: "RK3588 系列",
      vendor: "瑞芯微",
      chip: "RK3588 / RK3588S",
      subtitle: "选择具体板型后继续",
      thumbKind: "rk3588",
      children: [
        {
          id: "rk3588-evb1",
          name: "RK3588 · 已选板型",
          displayLabel: "RK3588-EVB1",
          vendor: "瑞芯微",
          chip: "RK3588",
          subtitle: "瑞芯微 · eMMC · Maskrom / Loader",
          thumbKind: "rk3588",
          connectMethod: "USB 烧录",
          modeHint: "Maskrom",
          storageDefault: "eMMC",
          defaultRoute: "完整镜像",
          hasOfficialResource: false,
          needsLoader: true,
          loaderLabel: "RAM Loader",
          defaultLoader: "rk3588_spl_loader_v1.15.bin",
          defaultImage: "rk3588-linux.img",
          defaultSize: "2.40 GB",
          defaultPath: "D:\\Images\\rk3588-linux.img",
          specSummary: "标准 RK3588 开发板；Maskrom 模式下须先加载匹配的 RAM Loader 后才可执行 eMMC 完整擦除与写入。"
        },
        {
          id: "lubancat-4",
          name: "LubanCat-4 (RK3588S)",
          displayLabel: "LubanCat-4 (RK3588S)",
          vendor: "瑞芯微",
          chip: "RK3588S",
          subtitle: "瑞芯微 · eMMC · Maskrom / Loader",
          thumbKind: "rk3588",
          connectMethod: "USB 烧录",
          modeHint: "Maskrom",
          storageDefault: "eMMC",
          defaultRoute: "完整镜像",
          hasOfficialResource: false,
          needsLoader: true,
          loaderLabel: "RAM Loader",
          defaultLoader: "rk3588s_spl_loader.bin",
          defaultImage: "lubancat4-ubuntu.img",
          defaultSize: "2.80 GB",
          defaultPath: "D:\\Images\\lubancat4-ubuntu.img",
          specSummary: "RK3588S 核心板型，使用专属 DDR/SPL Loader 初始化 eMMC 后执行完整镜像写入。"
        },
        {
          id: "rock-5b",
          name: "Rock 5B (RK3588)",
          displayLabel: "Rock 5B (RK3588)",
          vendor: "瑞芯微",
          chip: "RK3588",
          subtitle: "瑞芯微 · SPI NOR + eMMC",
          thumbKind: "rk3588",
          connectMethod: "USB 烧录",
          modeHint: "Maskrom",
          storageDefault: "eMMC",
          defaultRoute: "完整镜像",
          hasOfficialResource: false,
          needsLoader: true,
          loaderLabel: "RAM Loader",
          defaultLoader: "rock5b_spl_loader.bin",
          defaultImage: "rock5b-debian.img",
          defaultSize: "2.15 GB",
          defaultPath: "D:\\Images\\rock5b-debian.img",
          specSummary: "支持 Rockchip 厂家包与 eMMC 完整镜像路线；请务必选用对应板型的 Loader 文件。"
        }
      ]
    }
  ];

  function findBoard(id) {
    for (const item of BOARDS) {
      if (item.id === id) return item;
      if (item.children) {
        const sub = item.children.find((c) => c.id === id);
        if (sub) return sub;
      }
    }
    return BOARDS[0];
  }

  const PRESETS = [
    { code: "B01", group: "基础烧录", title: "选择板子", desc: "默认首页、板型筛选与右侧已选板型详情" },
    { code: "B02", group: "基础烧录", title: "板型系统", desc: "F101 官方示例资源 NekoBox 与 Loader 提示" },
    { code: "B03", group: "基础烧录", title: "本地镜像", desc: "拖入文件、文件可读与板型适配分开提示" },
    { code: "B04", group: "基础烧录", title: "接口就绪", desc: "Allwinner FEL 发现设备与只读模式核对" },
    { code: "B05", group: "基础烧录", title: "RK 缺少 Loader", desc: "Maskrom 缺少必填 RAM Loader 时禁用主按钮" },
    { code: "B06", group: "基础烧录", title: "烧录确认", desc: "基于接口就绪页叠加右侧确认抽屉" },
    { code: "B07", group: "基础烧录", title: "写入进度", desc: "五阶段进度条、锁定的工作区标签与等待完成" },
    { code: "B08", group: "基础烧录", title: "完成结果", desc: "四项独立状态报告与顶部对齐紧凑提示卡" },
    { code: "B09", group: "基础烧录", title: "失败恢复", desc: "写入中断提示、未自动重启与恢复步骤" },
    { code: "A01", group: "高级烧录", title: "SPI NOR 工作台", desc: "F101 主线 SPI NOR 集中配置与执行计划" },
    { code: "A02", group: "高级烧录", title: "RK 工作台", desc: "RK3588 Loader 会话、eMMC 与完整擦除警告" },
    { code: "A03", group: "高级烧录", title: "预检问题", desc: "缺少 RAM FES Loader 定位与未通过阻止擦写" },
    { code: "C01", group: "连接 AI", title: "本机通道", desc: "动态 MCP 地址、ZCode JSON 配置与状态分栏" },
    { code: "C02", group: "连接 AI", title: "远端通道", desc: "Windows 本机与远端 SSH 隧道独立状态卡片" },
    { code: "C03", group: "连接 AI", title: "请求确认", desc: "AI 调用记录与单次授权确认抽屉" },
    { code: "S01", group: "共享页面", title: "任务记录", desc: "全源任务列表、筛选与独立日志报告" },
    { code: "S02", group: "共享页面", title: "设置", desc: "外观主题切换、动效控制与自动保存底栏" }
  ];

  // =========================================================================
  // 3. APPLICATION STATE
  // =========================================================================
  const state = {
    viewMode: "canvas", // "canvas" | "responsive" | "gallery"
    pureMode: false,
    activePreset: "B01",

    // Global shell
    workspace: "basic", // "basic" | "advanced" | "ai" | "records" | "settings"
    prevWorkspace: "basic",
    modal: null, // null | { kind, ... }
    toast: "",
    toastTimer: null,

    // Settings (S02)
    settingsNav: "general", // "general" | "updates" | "logs" | "about"
    language: "zh-CN",
    theme: "light", // "light" | "dark" | "system"
    motionEnabled: true,
    detailedLogs: false,

    // Basic workflow (B01 - B09)
    basicStep: 1, // 1..4
    basicSubView: "normal", // "normal" | "progress" | "result" | "failed"
    boardSearch: "",
    boardFilter: "全部", // "全部" | "全志" | "瑞芯微"
    rkSeriesExpanded: false,
    boardId: "t113-pro",

    imageSourceTab: "local", // "resource" | "local"
    selectedResourceId: "nekobox",
    localImageName: "t113-linux.img",
    localImageSize: "1.20 GB",
    localImagePath: "D:\\Images\\t113-linux.img",
    localImageReadable: true,
    localImageFingerprint: "SHA-256 4a9e…c812",
    localImageFormat: "全志厂家镜像",

    usbScenario: "single", // "single" | "multi" | "none"
    selectedUsbId: "usb-1-3",
    usbList: [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }],
    loaderFileName: "",
    verifyEnabled: true,
    postAction: "重启",

    confirmDrawerOpen: false,
    confirmChecked: false,

    progressPct: 62,
    progressBytes: "744 MB / 1.20 GB",
    progressSpeed: "18 MB/s",
    progressStage: "写入",
    progressTimer: null,

    // Advanced workbench (A01 - A03)
    advBoardId: "f101",
    advPreset: "unsaved", // "unsaved" | "f101-default"
    advPlatform: "全志",
    advRoute: "主线系统 · SPI NOR",
    advImageName: "nekobox-full.img",
    advLoaderName: "fes1.fex",
    advStorage: "SPI NOR",
    advEraseMode: "物理布局擦除",
    advVerify: true,
    advPostAction: "重启",
    advPreflightStatus: "idle", // "idle" | "issues" | "passed"
    advLoaderFocusPulse: false,
    advModifiedFromBasic: false,

    // Connect AI (C01 - C03)
    aiNav: "local", // "local" | "remote" | "history"
    aiClientTab: "ZCode", // "Codex" | "ZCode" | "Claude Code" | "通用"
    aiMcpUrl: "http://127.0.0.1:8769/mcp",
    aiLocalHealthy: true,
    aiClientConnected: false,
    aiRemoteConnected: false,
    aiRemoteUrl: "",
    aiRemoteError: "SSH 连接被拒绝",
    aiDrawerOpen: false,
    aiConfirmChecked: false,
    aiCalls: [
      { id: "req-1", time: "10:24", client: "ZCode", op: "烧录 F101", status: "待确认", target: "F101 · USB 1-3", image: "nekobox-full.img", fingerprint: "SHA-256 8f2c…a910", route: "SPI NOR", action: "写入 → 物理回读 → 重启" },
      { id: "req-2", time: "10:23", client: "ZCode", op: "扫描设备", status: "完成" },
      { id: "req-3", time: "10:22", client: "ZCode", op: "检查镜像", status: "完成" }
    ],

    // Task Records (S01)
    recordsFilter: "全部任务",
    recordsSearch: "",
    recordsStatusFilter: "全部状态",
    records: [
      { id: "#F1024", board: "T113-Pro", image: "t113-linux.img", source: "基础烧录", time: "今天 10:24", statusType: "pass-pending-boot", statusText: "写入校验通过", secondaryStatus: "启动待确认", actionText: "查看报告" },
      { id: "#F1021", board: "F101", image: "nekobox-full.img", source: "高级烧录", time: "昨天 16:08", statusType: "interrupted", statusText: "写入中断", actionText: "查看日志" },
      { id: "#A0318", board: "RK3588", image: "rk3588-linux.img", source: "AI 调用", time: "昨天 14:30", statusType: "preflight-fail", statusText: "预检未通过", actionText: "查看详情" }
    ]
  };

  function stopProgressTimer() {
    if (state.progressTimer) {
      clearInterval(state.progressTimer);
      state.progressTimer = null;
    }
  }

  function showToast(msg) {
    state.toast = msg;
    const toastEl = document.getElementById("toastHost");
    if (toastEl) {
      toastEl.innerHTML = `<div class="toast-banner">${SVG.info}<span>${escapeHtml(msg)}</span></div>`;
    }
    if (state.toastTimer) clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => {
      state.toast = "";
      const el = document.getElementById("toastHost");
      if (el) el.innerHTML = "";
    }, 2600);
  }

  function escapeHtml(str) {
    return String(str ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // =========================================================================
  // 4. APPLY CANONICAL 17 PRESETS
  // =========================================================================
  function applyPreset(code) {
    stopProgressTimer();
    state.activePreset = code;
    state.modal = null;
    state.confirmDrawerOpen = false;
    state.confirmChecked = false;
    state.aiDrawerOpen = false;
    state.aiConfirmChecked = false;
    state.advLoaderFocusPulse = false;

    if (code.startsWith("B")) {
      state.workspace = "basic";
      state.basicSubView = "normal";
      state.usbScenario = "single";
    }

    switch (code) {
      case "B01":
        state.basicStep = 1;
        state.boardId = "t113-pro";
        state.boardFilter = "全部";
        state.boardSearch = "";
        state.rkSeriesExpanded = false;
        state.localImageName = "";
        break;
      case "B02":
        state.basicStep = 2;
        state.boardId = "f101";
        state.imageSourceTab = "resource";
        state.selectedResourceId = "nekobox";
        state.localImageName = "nekobox-full.img";
        state.localImageSize = "16.0 MB";
        state.localImageFormat = "SPI NOR 完整镜像";
        break;
      case "B03":
        state.basicStep = 2;
        state.boardId = "t113-pro";
        state.imageSourceTab = "local";
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.localImagePath = "D:\\Images\\t113-linux.img";
        state.localImageReadable = true;
        state.localImageFormat = "全志厂家镜像";
        break;
      case "B04":
        state.basicStep = 3;
        state.boardId = "t113-pro";
        state.imageSourceTab = "local";
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.localImageFormat = "全志厂家镜像";
        state.usbList = [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }];
        state.selectedUsbId = "usb-1-3";
        state.loaderFileName = "";
        state.verifyEnabled = true;
        state.postAction = "重启";
        break;
      case "B05":
        state.basicStep = 3;
        state.boardId = "rk3588-evb1";
        state.rkSeriesExpanded = true;
        state.imageSourceTab = "local";
        state.localImageName = "rk3588-linux.img";
        state.localImageSize = "2.40 GB";
        state.localImageFormat = "Rockchip 完整镜像";
        state.usbList = [{ id: "usb-2-1", label: "Rockchip Maskrom", port: "USB 2-1", mode: "Maskrom" }];
        state.selectedUsbId = "usb-2-1";
        state.loaderFileName = "";
        state.verifyEnabled = true;
        state.postAction = "重启";
        break;
      case "B06":
        state.basicStep = 3;
        state.boardId = "t113-pro";
        state.imageSourceTab = "local";
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.localImageFormat = "全志厂家镜像";
        state.usbList = [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }];
        state.selectedUsbId = "usb-1-3";
        state.verifyEnabled = true;
        state.postAction = "重启";
        state.confirmDrawerOpen = true;
        state.confirmChecked = false;
        break;
      case "B07":
        state.basicStep = 4;
        state.basicSubView = "progress";
        state.boardId = "t113-pro";
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.usbList = [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }];
        state.selectedUsbId = "usb-1-3";
        state.progressPct = 62;
        state.progressBytes = "744 MB / 1.20 GB";
        state.progressSpeed = "18 MB/s";
        state.progressStage = "写入";
        break;
      case "B08":
        state.basicStep = 4;
        state.basicSubView = "result";
        state.boardId = "t113-pro";
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.usbList = [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }];
        state.selectedUsbId = "usb-1-3";
        break;
      case "B09":
        state.basicStep = 4;
        state.basicSubView = "failed";
        state.boardId = "t113-pro";
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.usbList = [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }];
        state.selectedUsbId = "usb-1-3";
        break;
      case "A01":
        state.workspace = "advanced";
        state.advBoardId = "f101";
        state.advPreset = "unsaved";
        state.advPlatform = "全志";
        state.advRoute = "主线系统 · SPI NOR";
        state.advImageName = "nekobox-full.img";
        state.advLoaderName = "fes1.fex";
        state.advStorage = "SPI NOR";
        state.advEraseMode = "物理布局擦除";
        state.advVerify = true;
        state.advPostAction = "重启";
        state.advPreflightStatus = "idle";
        state.advModifiedFromBasic = false;
        break;
      case "A02":
        state.workspace = "advanced";
        state.advBoardId = "rk3588-evb1";
        state.advPreset = "unsaved";
        state.advPlatform = "瑞芯微";
        state.advRoute = "完整镜像 · eMMC";
        state.advImageName = "rk3588-linux.img";
        state.advLoaderName = "已运行 (当前会话)";
        state.advStorage = "eMMC";
        state.advEraseMode = "完整擦除";
        state.advVerify = true;
        state.advPostAction = "重启";
        state.advPreflightStatus = "idle";
        state.advModifiedFromBasic = false;
        break;
      case "A03":
        state.workspace = "advanced";
        state.advBoardId = "f101";
        state.advPreset = "unsaved";
        state.advPlatform = "全志";
        state.advRoute = "主线系统 · SPI NOR";
        state.advImageName = "nekobox-full.img";
        state.advLoaderName = "";
        state.advStorage = "SPI NOR";
        state.advVerify = true;
        state.advPostAction = "重启";
        state.advPreflightStatus = "issues";
        state.advModifiedFromBasic = false;
        break;
      case "C01":
        state.workspace = "ai";
        state.aiNav = "local";
        state.aiClientTab = "ZCode";
        state.aiLocalHealthy = true;
        state.aiClientConnected = false;
        break;
      case "C02":
        state.workspace = "ai";
        state.aiNav = "remote";
        state.aiLocalHealthy = true;
        state.aiRemoteConnected = false;
        state.aiRemoteUrl = "";
        state.aiRemoteError = "SSH 连接被拒绝";
        break;
      case "C03":
        state.workspace = "ai";
        state.aiNav = "history";
        state.aiCalls[0].status = "待确认";
        state.aiDrawerOpen = true;
        state.aiConfirmChecked = false;
        break;
      case "S01":
        state.workspace = "records";
        state.recordsFilter = "全部任务";
        state.recordsSearch = "";
        state.recordsStatusFilter = "全部状态";
        break;
      case "S02":
        state.workspace = "settings";
        state.settingsNav = "general";
        break;
    }

    if (state.viewMode === "gallery") {
      state.viewMode = "canvas";
    }
    render();
  }

  // =========================================================================
  // 5. SHELL RENDERING (TITLEBAR, HEADER, SIDEBAR, FOOTER)
  // =========================================================================
  function isTaskLocked() {
    return state.workspace === "basic" && state.basicStep === 4 && state.basicSubView === "progress";
  }

  function getSelectedUsb() {
    if (!state.usbList.length) return null;
    return state.usbList.find((u) => u.id === state.selectedUsbId) || null;
  }

  function getFooterRightText() {
    if (state.workspace === "basic" && state.basicStep === 2 && state.imageSourceTab === "resource") {
      return "UI 设计稿 · 示例资源";
    }
    return "UI 设计稿 · 示例数据";
  }

  function renderHeader() {
    const locked = isTaskLocked();
    const activeTab =
      state.workspace === "advanced"
        ? "advanced"
        : state.workspace === "ai"
        ? "ai"
        : "basic";

    const pendingAiCount = state.aiCalls.filter((c) => c.status === "待确认").length;

    return `
      <header class="app-header">
        <div class="app-header-left">
          <button class="brand-lockup" data-action="go-home" title="返回基础烧录">
            <span class="brand-mark">${SVG.brand}</span>
            <span class="brand-name">LYNX Imager</span>
          </button>
          <nav class="workspace-tabs" aria-label="工作区导航">
            <button class="ws-tab ${activeTab === "basic" ? "is-active" : ""}" data-action="switch-ws" data-ws="basic">
              基础烧录
            </button>
            <button class="ws-tab ${activeTab === "advanced" ? "is-active" : ""} ${locked ? "is-subdued" : ""}"
              data-action="switch-ws" data-ws="advanced" ${locked ? 'disabled title="任务执行期间工作区切换已锁定"' : ""}>
              高级烧录
            </button>
            <button class="ws-tab ${activeTab === "ai" ? "is-active" : ""} ${locked ? "is-subdued" : ""}"
              data-action="switch-ws" data-ws="ai" ${locked ? 'disabled title="任务执行期间工作区切换已锁定"' : ""}>
              连接 AI
              ${pendingAiCount > 0 ? `<span class="ws-tab-badge tabular">${pendingAiCount}</span>` : ""}
            </button>
          </nav>
        </div>
        <div class="app-header-tools">
          <button class="header-tool-btn ${state.workspace === "records" ? "is-active" : ""}"
            data-action="open-records" title="任务记录" aria-label="任务记录" ${locked ? "disabled" : ""}>
            ${SVG.history}
          </button>
          <button class="header-tool-btn" data-action="open-help" title="使用帮助" aria-label="使用帮助">
            ${SVG.help}
          </button>
          <button class="header-tool-btn ${state.workspace === "settings" ? "is-active" : ""}"
            data-action="open-settings" title="设置" aria-label="设置" ${locked ? "disabled" : ""}>
            ${SVG.settings}
          </button>
        </div>
      </header>
    `;
  }

  function renderSidebar() {
    if (state.workspace === "basic") {
      return renderBasicSidebar();
    }
    if (state.workspace === "advanced") {
      return renderAdvancedSidebar();
    }
    if (state.workspace === "ai") {
      return renderAiSidebar();
    }
    if (state.workspace === "records") {
      return renderRecordsSidebar();
    }
    return renderSettingsSidebar();
  }

  function renderBasicSidebar() {
    const board = findBoard(state.boardId);
    const usb = getSelectedUsb();
    const isFinalBoard = board && !board.isSeries;

    const step1Sub = isFinalBoard ? board.name : "";
    const step2Sub = state.basicStep >= 3 && state.localImageName ? state.localImageName : "";
    const step3Sub = state.basicStep >= 4 && usb ? `${usb.port} · ${usb.mode}` : "";

    const steps = [
      { num: 1, label: "选择板子", sub: step1Sub },
      { num: 2, label: "选择系统", sub: step2Sub },
      { num: 3, label: "配置接口", sub: step3Sub },
      { num: 4, label: "烧写系统", sub: "" }
    ];

    const stepHtml = steps
      .map((s) => {
        let cls = "";
        let circleContent = String(s.num);
        if (s.num < state.basicStep) {
          cls = "is-completed";
          circleContent = SVG.checkSm;
        } else if (s.num === state.basicStep) {
          if (s.num === 4 && state.basicSubView === "result") {
            cls = "is-active is-completed";
            circleContent = SVG.checkSm;
          } else if (s.num === 4 && state.basicSubView === "failed") {
            cls = "is-active is-failed";
            circleContent = "!";
          } else {
            cls = "is-active";
          }
        }

        const canClick =
          !isTaskLocked() &&
          s.num < state.basicStep &&
          state.basicSubView !== "result" &&
          state.basicSubView !== "failed";

        return `
          <button class="step-item ${cls}" ${canClick ? `data-action="jump-basic-step" data-step="${s.num}"` : "disabled"}>
            <span class="step-circle-col">
              <span class="step-circle tabular">${circleContent}</span>
            </span>
            <span class="step-text">
              <span class="step-label">${s.label}</span>
              ${s.sub ? `<span class="step-sub">${escapeHtml(s.sub)}</span>` : ""}
            </span>
          </button>
        `;
      })
      .join("");

    const summaryValue = isFinalBoard ? board.name : "未选择具体板型";

    return `
      <aside class="app-sidebar" aria-label="基础烧录步骤">
        <div class="sidebar-top">
          <div class="sidebar-title-wrap">
            <h2 class="sidebar-title">基础烧录</h2>
            <p class="sidebar-subtitle">四步完成系统写入</p>
          </div>
          <div class="step-list">${stepHtml}</div>
        </div>
        <div class="sidebar-bottom">
          <div class="sidebar-selection-box">
            <div class="sidebar-selection-kicker">当前选择</div>
            <div class="sidebar-selection-val">${escapeHtml(summaryValue)}</div>
          </div>
          <button class="sidebar-footer-link" data-action="open-records" ${isTaskLocked() ? "disabled" : ""}>
            ${SVG.history}<span>烧录记录</span>
          </button>
          <button class="sidebar-footer-link" data-action="open-help">
            ${SVG.help}<span>使用帮助</span>
          </button>
        </div>
      </aside>
    `;
  }

  function renderAdvancedSidebar() {
    const isRk = state.advBoardId === "rk3588-evb1";
    const devName = isRk ? "RK3588 · 已选板型" : "F101";
    const devStatus = isRk ? "Loader · USB 2-1" : "FEL · USB 1-3";

    return `
      <aside class="app-sidebar" aria-label="高级设备与预设">
        <div class="sidebar-top">
          <div class="sidebar-title-wrap">
            <h2 class="sidebar-title">目标设备</h2>
          </div>
          <div class="adv-device-card">
            <div class="adv-device-name">${devName}</div>
            <div>
              <span class="status-pill is-success">${SVG.dot}<span>${devStatus}</span></span>
            </div>
            <button class="btn btn-secondary btn-sm" data-action="adv-redetect">
              ${SVG.refresh}<span>重新检测</span>
            </button>
          </div>
          <div>
            <div class="adv-section-label">配置预设</div>
            <div class="side-nav-list">
              <button class="side-nav-item ${state.advPreset === "unsaved" ? "is-active" : ""}" data-action="adv-preset" data-preset="unsaved">
                <span>未保存的配置</span>
              </button>
              <button class="side-nav-item ${state.advPreset === "f101-default" ? "is-active" : ""}" data-action="adv-preset" data-preset="f101-default">
                <span>F101 · SPI NOR</span>
              </button>
              <button class="side-nav-item ${state.advPreset === "rk-emmc" ? "is-active" : ""}" data-action="adv-preset" data-preset="rk-emmc">
                <span>RK3588 · eMMC</span>
              </button>
            </div>
          </div>
        </div>
        <div class="sidebar-bottom">
          <button class="sidebar-footer-link" data-action="open-records">${SVG.history}<span>烧录记录</span></button>
          <button class="sidebar-footer-link" data-action="open-help">${SVG.help}<span>使用帮助</span></button>
        </div>
      </aside>
    `;
  }

  function renderAiSidebar() {
    const pendingCount = state.aiCalls.filter((c) => c.status === "待确认").length;
    const navs = [
      { id: "local", label: "本机通道", badge: 0 },
      { id: "remote", label: "远端通道", badge: 0 },
      { id: "history", label: "调用记录", badge: pendingCount }
    ];

    return `
      <aside class="app-sidebar" aria-label="AI 通道菜单">
        <div class="sidebar-top">
          <div class="sidebar-title-wrap">
            <h2 class="sidebar-title">连接 AI</h2>
            <p class="sidebar-subtitle">配置外部工具通道</p>
          </div>
          <div class="side-nav-list">
            ${navs
              .map(
                (n) => `
              <button class="side-nav-item ${state.aiNav === n.id ? "is-active" : ""}" data-action="ai-nav" data-nav="${n.id}">
                <span class="side-nav-left">${n.label}</span>
                ${n.badge > 0 ? `<span class="side-nav-badge tabular">${n.badge}</span>` : ""}
              </button>`
              )
              .join("")}
          </div>
        </div>
        <div class="sidebar-bottom">
          <button class="sidebar-footer-link" data-action="open-ai-guide">
            ${SVG.help}<span>接入说明</span>
          </button>
        </div>
      </aside>
    `;
  }

  function renderRecordsSidebar() {
    const filters = ["全部任务", "基础烧录", "高级烧录", "AI 调用"];
    return `
      <aside class="app-sidebar" aria-label="任务记录筛选">
        <div class="sidebar-top">
          <div class="sidebar-title-wrap">
            <h2 class="sidebar-title">任务记录</h2>
          </div>
          <div class="side-nav-list">
            ${filters
              .map(
                (f) => `
              <button class="side-nav-item ${state.recordsFilter === f ? "is-active" : ""}" data-action="records-filter" data-filter="${f}">
                <span>${f}</span>
              </button>`
              )
              .join("")}
          </div>
        </div>
        <div class="sidebar-bottom">
          <button class="sidebar-footer-link" data-action="back-to-basic">
            ${SVG.arrowLeft}<span>返回基础烧录</span>
          </button>
        </div>
      </aside>
    `;
  }

  function renderSettingsSidebar() {
    const items = [
      { id: "general", label: "常规" },
      { id: "updates", label: "软件更新" },
      { id: "logs", label: "诊断日志" },
      { id: "about", label: "关于" }
    ];
    return `
      <aside class="app-sidebar" aria-label="设置分类">
        <div class="sidebar-top">
          <div class="sidebar-title-wrap">
            <h2 class="sidebar-title">设置</h2>
          </div>
          <div class="side-nav-list">
            ${items
              .map(
                (it) => `
              <button class="side-nav-item ${state.settingsNav === it.id ? "is-active" : ""}" data-action="settings-nav" data-nav="${it.id}">
                <span>${it.label}</span>
              </button>`
              )
              .join("")}
          </div>
        </div>
        <div class="sidebar-bottom">
          <button class="sidebar-footer-link" data-action="back-from-settings">
            ${SVG.arrowLeft}<span>返回烧录</span>
          </button>
        </div>
      </aside>
    `;
  }

  // =========================================================================
  // 6. MAIN VIEW RENDERERS FOR ALL 17 CANONICAL SCREENS
  // =========================================================================
  function renderMainShell() {
    if (state.workspace === "basic") {
      if (state.basicStep === 1) return renderB01();
      if (state.basicStep === 2) return state.imageSourceTab === "resource" ? renderB02() : renderB03();
      if (state.basicStep === 3) return renderStep3Basic();
      if (state.basicStep === 4) {
        if (state.basicSubView === "result") return renderB08();
        if (state.basicSubView === "failed") return renderB09();
        return renderB07();
      }
    }
    if (state.workspace === "advanced") return renderAdvancedWorkbench();
    if (state.workspace === "ai") {
      if (state.aiNav === "local") return renderC01();
      if (state.aiNav === "remote") return renderC02();
      return renderC03();
    }
    if (state.workspace === "records") return renderS01();
    return renderS02();
  }

  // B01: 选择开发板
  function renderB01() {
    const q = state.boardSearch.trim().toLowerCase();
    const visibleBoards = BOARDS.filter((b) => {
      if (state.boardFilter !== "全部" && b.vendor !== state.boardFilter) return false;
      if (!q) return true;
      const hay = `${b.name} ${b.vendor} ${b.chip} ${b.subtitle}`.toLowerCase();
      if (hay.includes(q)) return true;
      if (b.children) {
        return b.children.some((c) => `${c.displayLabel} ${c.chip} ${c.subtitle}`.toLowerCase().includes(q));
      }
      return false;
    });

    const selectedBoard = findBoard(state.boardId);
    const isFinalBoard = selectedBoard && !selectedBoard.isSeries;

    const listHtml = visibleBoards
      .map((b) => {
        if (b.isSeries) {
          const anyChildSelected = b.children.some((c) => c.id === state.boardId);
          const expanded = state.rkSeriesExpanded || anyChildSelected || q.length > 0;
          return `
            <button class="select-row ${state.boardId === b.id ? "is-selected" : ""}" data-action="toggle-rk-series">
              <div class="select-row-left">
                <span class="board-thumb">${boardThumbSvg(b.thumbKind)}</span>
                <div>
                  <div class="row-title-line">
                    <span class="row-title">${b.name}</span>
                    <span class="status-pill is-neutral">需选具体型号</span>
                  </div>
                  <div class="row-sub">${b.subtitle}</div>
                </div>
              </div>
              <span style="color:var(--ink-muted)">${expanded ? SVG.chevronDown : SVG.chevronRight}</span>
            </button>
            ${
              expanded
                ? `<div class="series-sublist">
                    ${b.children
                      .map(
                        (child) => `
                      <button class="sub-board-row ${state.boardId === child.id ? "is-selected" : ""}" data-action="select-board" data-id="${child.id}">
                        <div>
                          <div style="font-size:14px;font-weight:600;color:var(--ink-primary)">${escapeHtml(child.displayLabel)}</div>
                          <div style="font-size:12px;color:var(--ink-secondary)">${escapeHtml(child.subtitle)}</div>
                        </div>
                        <span class="check-badge">${SVG.check}</span>
                      </button>`
                      )
                      .join("")}
                  </div>`
                : ""
            }
          `;
        }

        const selected = state.boardId === b.id;
        return `
          <button class="select-row ${selected ? "is-selected" : ""}" data-action="select-board" data-id="${b.id}">
            <div class="select-row-left">
              <span class="board-thumb">${boardThumbSvg(b.thumbKind)}</span>
              <div>
                <div class="row-title-line">
                  <span class="row-title">${b.name}</span>
                </div>
                <div class="row-sub">${b.subtitle}</div>
              </div>
            </div>
            <span class="check-badge">${SVG.check}</span>
          </button>
        `;
      })
      .join("");

    const inspectorHtml = isFinalBoard
      ? `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">已选板型</h3>
          <span class="status-pill is-indigo">${SVG.chip}<span>${escapeHtml(selectedBoard.vendor)}</span></span>
        </div>
        <div style="font-size:20px;font-weight:700;color:var(--ink-primary);margin-bottom:12px">${escapeHtml(selectedBoard.displayLabel || selectedBoard.name)}</div>
        <div class="inspector-diagram">${boardDiagramSvg(selectedBoard)}</div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">平台</span><span class="kv-val">${escapeHtml(selectedBoard.vendor)}</span></div>
          <div class="kv-row"><span class="kv-key">连接</span><span class="kv-val">${escapeHtml(selectedBoard.connectMethod)}</span></div>
          <div class="kv-row"><span class="kv-key">目标存储</span><span class="kv-val">${escapeHtml(selectedBoard.storageDefault)}</span></div>
        </div>
        <div style="margin-top:14px">
          <button class="text-link" data-action="open-board-doc">
            <span>查看板型说明</span>${SVG.external}
          </button>
        </div>
      </div>`
      : `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">已选板型</h3>
        </div>
        <div class="inline-note is-amber">
          ${SVG.alert}
          <span>请从左侧展开并选择具体的 RK3588 开发板型号。系列名称不能直接作为烧录目标。</span>
        </div>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">选择开发板</h1>
            <p class="page-subtitle">选择具体板型，准备对应的系统和连接方式。</p>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            <div class="filter-toolbar">
              <div class="search-box">
                ${SVG.search}
                <input id="boardSearchInput" class="search-input" type="text"
                  placeholder="搜索板型、芯片或厂商" value="${escapeHtml(state.boardSearch)}" />
                <span class="kbd-shortcut"><kbd class="kbd">Ctrl</kbd><kbd class="kbd">K</kbd></span>
              </div>
              <div class="tab-pills" role="tablist" aria-label="厂商筛选">
                ${["全部", "全志", "瑞芯微"]
                  .map(
                    (f) => `<button class="tab-pill ${state.boardFilter === f ? "is-active" : ""}" data-action="board-filter" data-filter="${f}">${f}</button>`
                  )
                  .join("")}
              </div>
            </div>
            ${listHtml || `<div class="panel">未找到匹配的板型，请清除筛选条件。</div>`}
            <div style="padding-top:4px">
              <button class="text-link" data-action="open-support-list">
                <span>未找到板型？查看支持列表</span>${SVG.external}
              </button>
            </div>
            <details class="inspector-mobile-wrap">
              <summary><span>已选板型详情：${escapeHtml(isFinalBoard ? (selectedBoard.displayLabel || selectedBoard.name) : "待选择具体型号")}</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${inspectorHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${inspectorHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <div class="bottom-summary">
            <span class="bottom-summary-main">
              ${isFinalBoard ? `${SVG.check}<span>已选择 ${escapeHtml(selectedBoard.displayLabel || selectedBoard.name)}</span>` : `${SVG.alert}<span>请选择具体板型后继续</span>`}
            </span>
            <span class="bottom-summary-sub">此步骤无需连接 USB</span>
          </div>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="b01-next" ${isFinalBoard ? "" : "disabled"}>
            <span>下一步：选择系统</span>${SVG.arrowRight}
          </button>
        </div>
      </div>
    `;
  }

  // B02: 板型系统 (官方板型资源视图)
  function renderB02() {
    const board = findBoard(state.boardId);
    const hasRes = Boolean(board.hasOfficialResource);

    const resourceCard = hasRes
      ? `
      <button class="select-row ${state.selectedResourceId === "nekobox" ? "is-selected" : ""}" data-action="select-resource" data-res="nekobox">
        <div class="select-row-left">
          <span class="board-thumb">${boardThumbSvg("f101")}</span>
          <div>
            <div class="row-title-line">
              <span class="row-title">${escapeHtml(board.resourceName)}</span>
              <span class="status-pill is-indigo">${SVG.dot}<span>板型资源</span></span>
            </div>
            <div class="row-sub">${escapeHtml(board.resourceSub)}</div>
          </div>
        </div>
        <span class="check-badge">${SVG.check}</span>
      </button>
      <div>
        <button class="text-link" data-action="open-release-notes">
          <span>查看发布说明</span>${SVG.external}
        </button>
      </div>`
      : `
      <div class="panel">
        <div class="inline-note is-neutral" style="margin-top:0">
          ${SVG.info}
          <span>当前板型（${escapeHtml(board.displayLabel || board.name)}）暂无在线板型目录资源，请选择本地镜像文件继续。</span>
        </div>
      </div>`;

    const inspectorHtml = `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">系统信息</h3>
        </div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">板型</span><span class="kv-val">${escapeHtml(board.displayLabel || board.name)}</span></div>
          <div class="kv-row"><span class="kv-key">镜像</span><span class="kv-val mono">${hasRes ? escapeHtml(board.resourceFile) : "待选择本地文件"}</span></div>
          <div class="kv-row"><span class="kv-key">存储</span><span class="kv-val">${escapeHtml(board.storageDefault)}</span></div>
          <div class="kv-row"><span class="kv-key">来源</span><span class="kv-val">${hasRes ? escapeHtml(board.resourceSource) : "本地文件"}</span></div>
        </div>
        ${
          board.needsLoader
            ? `<div class="inline-note is-amber">${SVG.alert}<span>此路线需要匹配的 RAM Loader</span></div>`
            : ""
        }
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">选择要安装的系统</h1>
            <p class="page-subtitle">查看当前板型的系统资源，或使用自己的镜像。</p>
          </div>
          <div class="tab-pills" role="tablist">
            <button class="tab-pill is-active" data-action="image-source-tab" data-tab="resource">板型镜像</button>
            <button class="tab-pill" data-action="image-source-tab" data-tab="local">本地文件</button>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            ${resourceCard}
            <button class="select-row" data-action="image-source-tab" data-tab="local" style="min-height:80px">
              <div class="select-row-left">
                <span class="file-icon-box">${SVG.file}</span>
                <div>
                  <div style="font-size:16px;font-weight:600;color:var(--ink-primary)">使用本地镜像</div>
                  <div class="row-sub">选择已有的系统文件</div>
                </div>
              </div>
              <span style="color:var(--ink-muted)">${SVG.chevronRight}</span>
            </button>
            <div style="font-size:12px;color:var(--ink-muted);padding-top:2px">资源目录按具体板型提供</div>
            <details class="inspector-mobile-wrap">
              <summary><span>系统信息</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${inspectorHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${inspectorHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <button class="btn btn-ghost" data-action="basic-back">${SVG.arrowLeft}<span>上一步</span></button>
          <div class="bottom-summary">
            <span class="bottom-summary-main">${hasRes ? `已选择 ${escapeHtml(board.resourceName)}` : "请选择本地镜像"}</span>
          </div>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="b02-next" ${hasRes ? "" : "disabled"}>
            <span>获取并继续</span>${SVG.arrowRight}
          </button>
        </div>
      </div>
    `;
  }

  // B03: 本地镜像
  function renderB03() {
    const board = findBoard(state.boardId);
    const hasFile = Boolean(state.localImageName);
    const readable = state.localImageReadable;

    const inspectorHtml = `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">镜像信息</h3>
        </div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">格式</span><span class="kv-val">${hasFile ? escapeHtml(state.localImageFormat) : "未读取"}</span></div>
          <div class="kv-row">
            <span class="kv-key">文件检查</span>
            <span class="kv-val">
              ${
                !hasFile
                  ? `<span class="status-pill is-neutral">${SVG.hollowDot}<span>未选择</span></span>`
                  : readable
                  ? `<span class="status-pill is-success">${SVG.checkSm}<span>读取成功</span></span>`
                  : `<span class="status-pill is-danger">${SVG.alert}<span>文件不可读</span></span>`
              }
            </span>
          </div>
          <div class="kv-row">
            <span class="kv-key">适用板型</span>
            <span class="kv-val"><span class="status-pill is-amber">${SVG.alert}<span>待核对来源</span></span></span>
          </div>
        </div>
        <div class="inline-note is-neutral">
          ${SVG.info}
          <span>文件可读取，不等于板型适配。</span>
        </div>
        <div style="margin-top:12px">
          <details class="collapse-box">
            <summary><span>所需文件与参数</span>${SVG.chevronRight}</summary>
            <div class="collapse-body">
              <div>当前目标：${escapeHtml(board.displayLabel || board.name)}（${escapeHtml(board.storageDefault)}）</div>
              <div style="margin-top:6px">
                <button class="text-link" data-action="toggle-unreadable-demo">
                  ${readable ? "演示：模拟文件不可读状态" : "恢复为正常读取状态"}
                </button>
              </div>
            </div>
          </details>
        </div>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">选择要安装的系统</h1>
            <p class="page-subtitle">查看当前板型的系统资源，或使用自己的镜像。</p>
          </div>
          <div class="tab-pills" role="tablist">
            <button class="tab-pill" data-action="image-source-tab" data-tab="resource">板型镜像</button>
            <button class="tab-pill is-active" data-action="image-source-tab" data-tab="local">本地文件</button>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            <input id="localFileInput" type="file" accept=".img,.bin,.gz,.xz,.fex" hidden />
            <div id="imageDropzone" class="dropzone ${hasFile ? "is-filled" : ""}">
              <div class="select-row-left">
                <span class="file-icon-box">${SVG.file}</span>
                <div style="min-width:0">
                  <div style="font-size:17px;font-weight:700;color:var(--ink-primary)">${hasFile ? escapeHtml(state.localImageName) : "拖入系统镜像文件"}</div>
                  <div class="row-sub">${hasFile ? `本地文件 · ${escapeHtml(state.localImageSize)}` : "支持 .img / .bin 镜像文件"}</div>
                  ${hasFile ? `<div class="mono" style="font-size:12px;color:var(--ink-muted);margin-top:4px">${escapeHtml(state.localImagePath)}</div>` : ""}
                </div>
              </div>
              <button class="btn btn-secondary btn-sm" data-action="pick-local-file">${hasFile ? "更换" : "选择文件"}</button>
            </div>
            ${
              !readable && hasFile
                ? `<div class="inline-note is-danger" style="margin-top:0">${SVG.alert}<span>无法打开当前文件或文件长度为 0，请重新选择或拖入有效镜像。</span></div>`
                : `<div style="font-size:12px;color:var(--ink-muted)">拖入文件可重新选择镜像</div>`
            }

            <div class="panel-white" style="padding:12px 16px">
              <div style="font-size:12px;font-weight:600;color:var(--ink-muted);margin-bottom:8px">最近使用</div>
              <button class="sidebar-footer-link" style="width:100%;justify-content:space-between;padding:8px 10px;background:var(--bg-panel)" data-action="use-recent-image">
                <span style="display:inline-flex;align-items:center;gap:8px;color:var(--ink-primary);font-weight:500">
                  ${SVG.history}<span>t113-linux.img</span>
                </span>
                <span class="mono" style="font-size:12px;color:var(--ink-muted)">1.20 GB</span>
              </button>
            </div>
            <details class="inspector-mobile-wrap">
              <summary><span>镜像信息</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${inspectorHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${inspectorHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <button class="btn btn-ghost" data-action="basic-back">${SVG.arrowLeft}<span>上一步</span></button>
          <div class="bottom-summary">
            <span class="bottom-summary-main">${hasFile ? escapeHtml(state.localImageName) : "未选择镜像"}</span>
          </div>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="b03-next" ${hasFile && readable ? "" : "disabled"}>
            <span>下一步：配置接口</span>${SVG.arrowRight}
          </button>
        </div>
      </div>
    `;
  }

  // B04 & B05: Step 3 配置烧录接口 / RK 缺少 Loader
  function renderStep3Basic() {
    const board = findBoard(state.boardId);
    // B05 is a missing-loader state that can be reached by both Allwinner and
    // Rockchip boards. The board vendor, rather than the preset code, decides
    // which interface layout is rendered.
    const isRk = board.vendor === "瑞芯微";
    const missingLoader = Boolean(board.needsLoader && !state.loaderFileName);
    const usb = getSelectedUsb();
    const noDevices = state.usbList.length === 0;
    const multiUnselected = state.usbList.length > 1 && !usb;
    const canProceed = Boolean(usb && !missingLoader);

    const usbBadge = noDevices
      ? `<span class="status-pill is-amber">${SVG.alert}<span>未发现设备</span></span>`
      : `<span class="status-pill is-success">${SVG.dot}<span>已发现 ${state.usbList.length} 台设备</span></span>`;

    const deviceRowsHtml = noDevices
      ? `<div class="inline-note is-amber" style="margin-top:0">${SVG.alert}<span>未检测到处于烧录模式的 USB 设备。请按板型说明进入 ${escapeHtml(board.modeHint)} 后点击刷新，工具不会自动重启设备。</span></div>`
      : state.usbList
          .map((u) => {
            const sel = u.id === state.selectedUsbId;
            return `
              <button class="select-row ${sel ? "is-selected" : ""}" style="min-height:72px;padding:12px 16px" data-action="select-usb" data-id="${u.id}">
                <div class="select-row-left">
                  <span class="file-icon-box">${SVG.usb}</span>
                  <div>
                    <div style="font-size:15px;font-weight:700;color:var(--ink-primary)">${escapeHtml(u.label)}</div>
                    <div class="row-sub">${escapeHtml(u.port)} · ${sel ? "已选择" : "点击绑定此设备"}</div>
                  </div>
                </div>
                <span class="check-badge">${SVG.check}</span>
              </button>`;
          })
          .join("");

    const rkLoaderHtml = board.needsLoader
      ? `
      <div class="required-card ${state.loaderFileName ? "is-fulfilled" : ""}">
        <div style="display:flex;align-items:center;justify-content:space-between;gap:10px">
          <div style="display:flex;align-items:center;gap:8px">
            <span style="font-size:14px;font-weight:700;color:var(--ink-primary)">${escapeHtml(board.loaderLabel || "RAM Loader")}</span>
            ${
              state.loaderFileName
                ? `<span class="status-pill is-success">${SVG.checkSm}<span>已选择</span></span>`
                : `<span class="status-pill is-amber">${SVG.alert}<span>必填</span></span>`
            }
          </div>
          <button class="text-link" data-action="open-board-doc">
            <span>查看板型连接说明</span>${SVG.external}
          </button>
        </div>
        <div class="file-input-bar">
          <input id="loaderFileInput" type="file" accept=".bin,.fex" hidden />
          <div class="file-input-display ${state.loaderFileName ? "mono" : "is-placeholder"}">
            ${state.loaderFileName ? escapeHtml(state.loaderFileName) : "选择匹配当前板型的 Loader 文件"}
          </div>
          <button class="btn btn-secondary btn-sm" data-action="pick-loader-file">选择文件</button>
        </div>
        <div style="font-size:12px;color:var(--ink-secondary)">
          ${usb ? `当前设备处于 ${escapeHtml(usb.mode)} 模式。` : "请先连接并选择 USB 设备。"}
          ${!state.loaderFileName ? ` <button class="text-link" data-action="fill-sample-loader">填入示例 Loader (${escapeHtml(board.defaultLoader)})</button>` : ""}
        </div>
      </div>`
      : "";

    const inspectorHtml = isRk
      ? `
      <div class="panel">
        <div class="panel-header"><h3 class="panel-title">本次任务</h3></div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">系统</span><span class="kv-val mono">${escapeHtml(state.localImageName || board.defaultImage)}</span></div>
          <div class="kv-row"><span class="kv-key">存储</span><span class="kv-val">${escapeHtml(board.storageDefault)}</span></div>
          <div class="kv-row"><span class="kv-key">路线</span><span class="kv-val">${escapeHtml(board.defaultRoute)}</span></div>
          <div class="kv-row"><span class="kv-key">检测模式</span><span class="kv-val">${usb ? escapeHtml(usb.mode) : "未检测"}</span></div>
        </div>
        <div class="inline-note is-neutral">${SVG.info}<span>使用当前板型对应的 Loader</span></div>
      </div>`
      : `
      <div class="panel">
        <div class="panel-header"><h3 class="panel-title">本次任务</h3></div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">系统</span><span class="kv-val mono">${escapeHtml(state.localImageName || board.defaultImage)}</span></div>
          <div class="kv-row"><span class="kv-key">路线</span><span class="kv-val">厂家镜像</span></div>
          <div class="kv-row"><span class="kv-key">写入校验</span><span class="kv-val">${state.verifyEnabled ? "开启" : "关闭"}</span></div>
          <div class="kv-row">
            <span class="kv-key">结束动作</span>
            <span class="kv-val">
              <select class="field-select" data-action="change-post-action" aria-label="结束动作">
                <option value="重启" ${state.postAction === "重启" ? "selected" : ""}>重启</option>
                <option value="保持连接" ${state.postAction === "保持连接" ? "selected" : ""}>保持连接</option>
              </select>
            </span>
          </div>
        </div>
        <div class="inline-note is-neutral">${SVG.info}<span>校验通过后执行结束动作</span></div>
      </div>`;

    const bottomStatus = noDevices
      ? `<span style="color:var(--amber);display:inline-flex;align-items:center;gap:6px">${SVG.alert}<span>未发现设备</span></span>`
      : multiUnselected
      ? `<span style="color:var(--amber);display:inline-flex;align-items:center;gap:6px">${SVG.alert}<span>请明确选择一台目标设备</span></span>`
      : missingLoader
      ? `<span style="color:var(--amber);display:inline-flex;align-items:center;gap:6px">${SVG.alert}<span>缺少 ${escapeHtml(board.loaderLabel || "RAM Loader")}</span></span>`
      : `<span style="color:var(--success);display:inline-flex;align-items:center;gap:6px">${SVG.dot}<span>接口已就绪</span></span>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">${isRk ? "准备 RK 烧录接口" : "配置烧录接口"}</h1>
            <p class="page-subtitle">${isRk ? "设备已识别，补齐当前板型需要的文件。" : "选择实际连接的 USB 设备，并核对目标板型。"}</p>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            <div class="panel-white" style="display:flex;flex-direction:column;gap:14px">
              <div style="display:flex;align-items:center;justify-content:space-between;gap:10px;flex-wrap:wrap">
                <div style="display:flex;align-items:center;gap:10px">
                  <span style="font-size:15px;font-weight:700;color:var(--ink-primary)">USB 烧录</span>
                  ${usbBadge}
                </div>
                <div style="display:flex;align-items:center;gap:6px">
                  <div class="tab-pills" title="设备数量状态模拟">
                    <button class="tab-pill ${state.usbScenario === "single" ? "is-active" : ""}" data-action="usb-scenario" data-sc="single" style="padding:3px 9px;font-size:12px">1 台</button>
                    <button class="tab-pill ${state.usbScenario === "multi" ? "is-active" : ""}" data-action="usb-scenario" data-sc="multi" style="padding:3px 9px;font-size:12px">多台</button>
                    <button class="tab-pill ${state.usbScenario === "none" ? "is-active" : ""}" data-action="usb-scenario" data-sc="none" style="padding:3px 9px;font-size:12px">未发现</button>
                  </div>
                  <button class="btn btn-secondary btn-sm" data-action="refresh-usb" title="重新扫描 USB">${SVG.refresh}</button>
                </div>
              </div>

              ${
                !isRk
                  ? `<div class="topo-strip">
                      <span class="topo-node">${SVG.pc}<span>这台电脑</span></span>
                      <span class="topo-line"><span class="topo-dash"></span><span class="topo-badge">FEL</span><span class="topo-dash"></span></span>
                      <span class="topo-node">${SVG.chip}<span>开发板</span></span>
                    </div>`
                  : ""
              }

              ${deviceRowsHtml}

              ${
                !isRk
                  ? `<div style="padding-top:4px;border-top:1px solid var(--border-subtle)">
                      <div class="kv-row"><span class="kv-key">绑定板型（只读）</span><span class="kv-val">${escapeHtml(board.displayLabel || board.name)}</span></div>
                      <div class="kv-row"><span class="kv-key">检测模式（只读）</span><span class="kv-val">${usb ? escapeHtml(usb.mode) : "未检测"}</span></div>
                    </div>`
                  : ""
              }
            </div>

            ${rkLoaderHtml}

            <details class="collapse-box">
              <summary><span>设备没有出现？查看连接方法</span>${SVG.chevronRight}</summary>
              <div class="collapse-body">
                确认使用支持数据传输的 USB 线连接 OTG/烧录口；按开发板说明按住 BOOT / FEL / Maskrom 按键后再接通电源，随后点击右上角刷新按钮重新扫描。
              </div>
            </details>

            <details class="inspector-mobile-wrap">
              <summary><span>本次任务参数</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${inspectorHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${inspectorHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar ${state.confirmDrawerOpen ? "is-dimmed" : ""}">
        <div class="bottom-left">
          <button class="btn btn-ghost" data-action="basic-back">${SVG.arrowLeft}<span>上一步</span></button>
          <div class="bottom-summary">
            <span class="bottom-summary-main">${bottomStatus}</span>
          </div>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="open-confirm-drawer" ${canProceed ? "" : "disabled"}>
            <span>检查并烧录</span>${SVG.arrowRight}
          </button>
        </div>
      </div>
    `;
  }

  // B07: 写入进度
  function renderB07() {
    const board = findBoard(state.boardId);
    const usb = getSelectedUsb() || { port: "USB 1-3" };
    const stages = ["准备", "擦除", "写入", "校验", "结束动作"];
    const activeIdx = stages.indexOf(state.progressStage);

    const inspectorHtml = `
      <div class="panel">
        <div class="panel-header"><h3 class="panel-title">任务信息</h3></div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">接口</span><span class="kv-val mono">${escapeHtml(usb.port)}</span></div>
          <div class="kv-row"><span class="kv-key">板型</span><span class="kv-val">${escapeHtml(board.displayLabel || board.name)}</span></div>
          <div class="kv-row"><span class="kv-key">来源</span><span class="kv-val">基础烧录</span></div>
          <div class="kv-row"><span class="kv-key">任务编号</span><span class="kv-val mono">#F1024</span></div>
        </div>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">正在写入系统</h1>
            <p class="page-subtitle">保持设备供电与 USB 连接，写入后将继续校验。</p>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            <div class="panel-white">
              <div style="display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:14px">
                <div style="display:inline-flex;align-items:center;gap:8px;font-size:14px;font-weight:600;color:var(--ink-primary)">
                  <span class="mono">${escapeHtml(state.localImageName || "t113-linux.img")}</span>
                  <span style="color:var(--ink-muted)">${SVG.arrowRight}</span>
                  <span>${escapeHtml(board.displayLabel || board.name)}</span>
                </div>
                <span class="status-pill is-indigo">${SVG.dot}<span>镜像${escapeHtml(state.progressStage)}</span></span>
              </div>

              <div style="display:flex;align-items:baseline;justify-content:space-between;gap:16px">
                <div class="progress-big tabular">${state.progressPct}%</div>
                <div class="tabular" style="font-size:13px;color:var(--ink-secondary)">
                  ${escapeHtml(state.progressBytes)} · ${escapeHtml(state.progressSpeed)}
                </div>
              </div>

              <div class="progress-track">
                <div class="progress-fill" style="width:${state.progressPct}%"></div>
              </div>

              <div class="stage-bar">
                ${stages
                  .map((st, idx) => {
                    const cls = idx < activeIdx ? "is-done" : idx === activeIdx ? "is-active" : "";
                    const icon = idx < activeIdx ? SVG.checkSm : idx === activeIdx ? SVG.dot : SVG.hollowDot;
                    return `<div class="stage-chip ${cls}">${icon}<span>${st}</span></div>`;
                  })
                  .join("")}
              </div>
            </div>

            <details class="collapse-box">
              <summary><span>查看运行详情</span>${SVG.chevronRight}</summary>
              <div class="collapse-body mono" style="font-size:12px;line-height:1.7">
                [10:24:01] 预检完成：目标 ${escapeHtml(board.displayLabel || board.name)} (${escapeHtml(usb.port)})<br/>
                [10:24:03] 擦除阶段完成<br/>
                [10:24:06] 正在写入分区镜像 (${state.progressPct}%)...
                <div style="margin-top:10px;display:flex;gap:10px;font-family:var(--font-sans)">
                  <button class="btn btn-secondary btn-sm" data-action="sim-complete">模拟推进至完成 (B08)</button>
                  <button class="btn btn-danger-outline btn-sm" data-action="sim-fail">模拟写入中断 (B09)</button>
                </div>
              </div>
            </details>

            <details class="inspector-mobile-wrap">
              <summary><span>任务信息</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${inspectorHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${inspectorHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <div class="bottom-summary">
            <span class="bottom-summary-main" style="color:var(--ink-secondary)">${SVG.lock}<span>任务执行中，设备操作暂时锁定</span></span>
          </div>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" disabled>等待完成</button>
        </div>
      </div>
    `;
  }

  // B08: 完成结果 (Left ~67% result panel + Right ~33% compact top-aligned 接下来 card)
  function renderB08() {
    const board = findBoard(state.boardId);
    const helpCard = `
      <div class="panel" style="align-self:start">
        <div class="panel-header" style="margin-bottom:8px">
          <h3 class="panel-title">接下来</h3>
        </div>
        <p style="margin:0;font-size:13px;line-height:1.65;color:var(--ink-secondary)">
          查看开发板屏幕或串口输出，确认新系统启动。
        </p>
        <div class="inline-note is-neutral" style="margin-top:14px">
          ${SVG.info}
          <span>写入与重启完成不代表系统已成功启动。</span>
        </div>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">系统写入完成</h1>
            <p class="page-subtitle">写入和校验已通过，启动状态单独确认。</p>
          </div>
        </div>
        <div class="two-col is-balanced">
          <div class="col-primary">
            <div class="panel-white" style="padding:22px 24px">
              <div style="display:flex;align-items:center;gap:14px;padding-bottom:16px;border-bottom:1px solid var(--border-subtle)">
                <span style="width:40px;height:40px;border-radius:50%;background:var(--success-bg);color:var(--success);border:1px solid var(--success-border);display:inline-flex;align-items:center;justify-content:center">
                  ${SVG.check}
                </span>
                <div>
                  <div style="font-size:19px;font-weight:700;color:var(--ink-primary)">${escapeHtml(board.displayLabel || board.name)}</div>
                  <div class="row-sub mono">${escapeHtml(state.localImageName || "t113-linux.img")} · 用时 02:18</div>
                </div>
              </div>

              <div class="kv-list" style="margin-top:6px">
                <div class="kv-row" style="padding:12px 0">
                  <span style="font-weight:600;color:var(--ink-primary)">镜像写入</span>
                  <span class="status-pill is-success">${SVG.checkSm}<span>完成</span></span>
                </div>
                <div class="kv-row" style="padding:12px 0">
                  <span style="font-weight:600;color:var(--ink-primary)">回读校验</span>
                  <span class="status-pill is-success">${SVG.checkSm}<span>通过</span></span>
                </div>
                <div class="kv-row" style="padding:12px 0">
                  <span style="font-weight:600;color:var(--ink-primary)">重启指令</span>
                  <span class="status-pill is-success">${SVG.checkSm}<span>已执行</span></span>
                </div>
                <div class="kv-row" style="padding:12px 0">
                  <span style="font-weight:600;color:var(--ink-primary)">系统启动</span>
                  <span class="status-pill is-amber">${SVG.hollowDot}<span>待确认</span></span>
                </div>
              </div>

              <div style="margin-top:16px;display:flex;align-items:center;gap:18px">
                <button class="text-link" data-action="open-task-log">查看日志</button>
                <button class="text-link" data-action="export-report">导出报告</button>
              </div>
            </div>
            <details class="inspector-mobile-wrap">
              <summary><span>接下来</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${helpCard}</div>
            </details>
          </div>
          <div class="col-inspector" style="align-self:start">${helpCard}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <button class="btn btn-ghost" data-action="go-home">返回首页</button>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="flash-another">
            <span>烧录另一块板</span>${SVG.arrowRight}
          </button>
        </div>
      </div>
    `;
  }

  // B09: 失败恢复
  function renderB09() {
    const board = findBoard(state.boardId);
    const usb = getSelectedUsb() || { port: "USB 1-3" };

    const inspectorHtml = `
      <div class="panel">
        <div class="panel-header"><h3 class="panel-title">本次任务</h3></div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">板型</span><span class="kv-val">${escapeHtml(board.displayLabel || board.name)}</span></div>
          <div class="kv-row"><span class="kv-key">镜像</span><span class="kv-val mono">${escapeHtml(state.localImageName || "t113-linux.img")}</span></div>
          <div class="kv-row"><span class="kv-key">接口</span><span class="kv-val mono">${escapeHtml(usb.port)}</span></div>
        </div>
        <div class="inline-note is-neutral">${SVG.info}<span>镜像选择已保留，无需重新选取文件。</span></div>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">写入中断</h1>
            <p class="page-subtitle">连接已断开，本次任务已停止。</p>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            <div class="failure-panel">
              <div style="display:flex;align-items:center;gap:8px;font-size:16px;font-weight:700;color:var(--amber)">
                ${SVG.alert}<span>失败阶段：镜像写入</span>
              </div>
              <p style="margin:8px 0 0;font-size:14px;color:var(--ink-secondary)">
                部分数据可能已被覆盖，工具未执行烧后重启，且不会自动重复擦写。
              </p>
              <div class="recovery-list">
                <div class="recovery-item"><span class="recovery-num">1</span><span>检查 USB 数据线接触与开发板供电是否稳定</span></div>
                <div class="recovery-item"><span class="recovery-num">2</span><span>按板型说明重新进入 FEL / 烧录模式</span></div>
                <div class="recovery-item"><span class="recovery-num">3</span><span>点击“重新检测设备”并在确认抽屉重新核对目标</span></div>
              </div>
            </div>

            <details class="collapse-box">
              <summary><span>错误详情与日志</span>${SVG.chevronRight}</summary>
              <div class="collapse-body mono" style="font-size:12px;line-height:1.7">
                [ERROR] stage=write offset=0x1e400000 reason=USB_DEVICE_DISCONNECTED (LIBUSB_ERROR_NO_DEVICE)<br/>
                [SAFEGUARD] post_action=reboot skipped; previous destructive confirmation invalidated.
              </div>
            </details>

            <details class="inspector-mobile-wrap">
              <summary><span>本次任务信息</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${inspectorHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${inspectorHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <button class="btn btn-secondary" data-action="failed-back">返回修改</button>
          <button class="btn btn-ghost" data-action="export-diag">导出诊断记录</button>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="redetect-after-fail">重新检测设备</button>
        </div>
      </div>
    `;
  }

  // A01, A02, A03: 高级烧录工作台
  function renderAdvancedWorkbench() {
    const isRk = state.advBoardId === "rk3588-evb1";
    const missingLoader = !isRk && !state.advLoaderName;
    const preflightIssues = state.advPreflightStatus === "issues" || (missingLoader && state.activePreset === "A03");
    const preflightPassed = state.advPreflightStatus === "passed" && !missingLoader;

    const routes = isRk
      ? ["完整镜像 · eMMC", "厂家包 · eMMC"]
      : ["主线系统 · SPI NOR", "全志厂家镜像 · SPI NAND"];

    const seqSteps = isRk
      ? ["预检", "擦除", "写入", "校验", "结束动作"]
      : ["设备预检", "物理备份", "写入", "物理回读", "结束动作"];

    const rightPanelHtml = preflightIssues
      ? `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">预检结果</h3>
          <span class="status-pill is-amber">${SVG.alert}<span>1 项待处理</span></span>
        </div>
        <div class="plan-seq">
          <div class="plan-step-row" style="border-color:var(--success-border);background:var(--success-bg);color:var(--success)">
            ${SVG.checkSm}<span>已选择目标设备 (F101 · USB 1-3)</span>
          </div>
          <div class="plan-step-row" style="border-color:var(--success-border);background:var(--success-bg);color:var(--success)">
            ${SVG.checkSm}<span>镜像文件可读取 (${escapeHtml(state.advImageName)})</span>
          </div>
          <div class="plan-step-row" style="border-color:var(--amber-border);background:var(--amber-bg);color:var(--amber);justify-content:space-between">
            <span style="display:inline-flex;align-items:center;gap:8px;font-weight:600">! <span>缺少 RAM FES Loader</span></span>
          </div>
          <div class="plan-step-row" style="color:var(--ink-muted)">
            ${SVG.hollowDot}<span>物理布局检查 (等待)</span>
          </div>
          <div class="plan-step-row" style="color:var(--ink-muted)">
            ${SVG.hollowDot}<span>写入准备 (未开始)</span>
          </div>
        </div>
        <button class="text-link" data-action="adv-locate-missing">定位到缺少的文件</button>
      </div>`
      : `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">执行计划</h3>
          ${
            preflightPassed
              ? `<span class="status-pill is-success">${SVG.checkSm}<span>预检通过</span></span>`
              : `<span class="status-pill is-amber">${SVG.alert}<span>尚未执行预检</span></span>`
          }
        </div>
        <div class="kv-list">
          <div class="kv-row"><span class="kv-key">目标</span><span class="kv-val">${isRk ? "RK3588 · USB 2-1" : "F101 · USB 1-3"}</span></div>
          <div class="kv-row"><span class="kv-key">文件</span><span class="kv-val mono">${escapeHtml(state.advImageName)}</span></div>
        </div>
        <div class="plan-seq">
          ${seqSteps
            .map(
              (s, i) => `
            <div class="plan-step-row">
              <span class="recovery-num">${i + 1}</span>
              <span style="color:var(--ink-primary);font-weight:500">${s}</span>
            </div>`
            )
            .join("")}
        </div>
        ${
          isRk
            ? `<div class="inline-note is-neutral" style="margin-top:0;margin-bottom:12px">${SVG.info}<span>完整擦除会覆盖目标存储数据</span></div>`
            : ""
        }
        <button class="btn btn-secondary btn-sm" style="width:100%" data-action="open-plan-modal">查看计划</button>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">高级烧录</h1>
            <p class="page-subtitle">集中配置设备、镜像与执行参数。</p>
          </div>
        </div>

        <div class="two-col is-advanced">
          <div class="col-primary">
            <div class="panel-white">
              <div class="panel-header"><h3 class="panel-title">镜像与文件</h3></div>
              <div class="adv-form-row">
                <span class="adv-form-label">平台</span>
                <div class="adv-form-ctrl">
                  <span style="font-weight:600">${escapeHtml(state.advPlatform)}</span>
                </div>
              </div>
              <div class="adv-form-row">
                <span class="adv-form-label">烧录路线</span>
                <div class="adv-form-ctrl">
                  <select class="field-select field-select-wide" data-action="adv-change-route" aria-label="烧录路线">
                    ${routes.map((r) => `<option value="${r}" ${state.advRoute === r ? "selected" : ""}>${r}</option>`).join("")}
                  </select>
                </div>
              </div>
              <div class="adv-form-row">
                <span class="adv-form-label">${isRk ? "系统镜像" : "完整镜像"}</span>
                <div class="adv-form-ctrl">
                  <span class="mono" style="font-size:13px">${escapeHtml(state.advImageName)}</span>
                  <button class="btn btn-secondary btn-sm" data-action="adv-pick-image">选择</button>
                </div>
              </div>
              ${
                isRk
                  ? `<div class="adv-form-row">
                      <span class="adv-form-label">当前 Loader</span>
                      <div class="adv-form-ctrl">
                        <div>
                          <div style="font-weight:600;color:var(--success)">已运行</div>
                          <div style="font-size:12px;color:var(--ink-muted)">使用当前设备的 Loader 会话</div>
                        </div>
                      </div>
                    </div>`
                  : `<div class="adv-form-row" id="advLoaderRow">
                      <span class="adv-form-label">
                        RAM FES Loader
                        ${missingLoader ? `<span class="status-pill is-amber" style="padding:1px 6px">必填</span>` : ""}
                      </span>
                      <div class="adv-form-ctrl">
                        <div class="file-input-display ${missingLoader ? "is-placeholder is-amber-outline" : "mono"} ${state.advLoaderFocusPulse ? "pulse-highlight" : ""}" style="height:34px">
                          ${state.advLoaderName ? escapeHtml(state.advLoaderName) : "选择匹配板型的 Loader"}
                        </div>
                        <button class="btn btn-secondary btn-sm" data-action="adv-pick-loader">选择</button>
                      </div>
                    </div>`
              }
            </div>

            <div class="panel-white">
              <div class="panel-header"><h3 class="panel-title">执行参数</h3></div>
              <div class="adv-form-row">
                <span class="adv-form-label">目标存储</span>
                <div class="adv-form-ctrl"><span style="font-weight:600">${escapeHtml(state.advStorage)}</span><span style="font-size:12px;color:var(--ink-muted)">由路线决定（只读）</span></div>
              </div>
              ${
                isRk
                  ? `<div class="adv-form-row">
                      <span class="adv-form-label">擦除方式</span>
                      <div class="adv-form-ctrl"><span style="font-weight:500">${escapeHtml(state.advEraseMode)}</span></div>
                    </div>`
                  : ""
              }
              <div class="adv-form-row">
                <span class="adv-form-label">写入校验</span>
                <div class="adv-form-ctrl">
                  <span style="font-weight:500">${state.advVerify ? "开启" : "关闭"}</span>
                  ${!isRk ? `<span style="font-size:12px;color:var(--ink-muted)">独立物理回读保护</span>` : ""}
                </div>
              </div>
              <div class="adv-form-row">
                <span class="adv-form-label">结束动作</span>
                <div class="adv-form-ctrl">
                  <select class="field-select" data-action="adv-change-post" aria-label="高级模式结束动作">
                    <option value="重启" ${state.advPostAction === "重启" ? "selected" : ""}>重启</option>
                    <option value="保持连接" ${state.advPostAction === "保持连接" ? "selected" : ""}>保持连接</option>
                  </select>
                </div>
              </div>
              ${
                !isRk
                  ? `<div style="margin-top:10px">
                      <details class="collapse-box">
                        <summary><span>其他受支持参数</span>${SVG.chevronRight}</summary>
                        <div class="collapse-body">物理布局边界保护：由后端板型能力锁定；不支持自由输入任意物理地址或跳过回读保护。</div>
                      </details>
                    </div>`
                  : ""
              }
            </div>

            ${
              preflightIssues
                ? `<div class="diagnostic-strip">${SVG.alert}<span>预检未通过，尚未开始擦写</span></div>`
                : `<details class="collapse-box">
                    <summary><span>日志与诊断</span>${SVG.chevronRight}</summary>
                    <div class="collapse-body mono" style="font-size:12px">[INFO] Ready for preflight check on ${escapeHtml(state.advStorage)} route.</div>
                  </details>`
            }

            <details class="inspector-mobile-wrap">
              <summary><span>${preflightIssues ? "预检结果" : "执行计划"}</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${rightPanelHtml}</div>
            </details>
          </div>

          <div class="col-inspector">${rightPanelHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <button class="btn btn-secondary btn-sm" data-action="adv-save-cfg">保存配置</button>
          ${preflightIssues ? "" : `<button class="btn btn-ghost btn-sm" data-action="adv-import-cfg">导入配置</button>`}
        </div>
        <div class="bottom-right">
          ${
            preflightIssues
              ? `<button class="btn btn-secondary" data-action="adv-run-preflight">重新预检</button>
                 <button class="btn btn-primary" disabled>开始烧录</button>`
              : preflightPassed
              ? `<button class="btn btn-secondary" data-action="adv-run-preflight">重新预检</button>
                 <button class="btn btn-primary" data-action="adv-start-confirm">开始烧录</button>`
              : `<button class="btn btn-primary" data-action="adv-run-preflight">运行预检</button>`
          }
        </div>
      </div>
    `;
  }

  // C01: 本机 MCP 通道
  function renderC01() {
    const snippets = {
      ZCode: `{
  "mcpServers": {
    "lynx": {
      "type": "http",
      "url": "${state.aiMcpUrl}"
    }
  }
}`,
      Codex: `[mcp_servers.lynx]
url = "${state.aiMcpUrl}"`,
      "Claude Code": `claude mcp add --transport http lynx ${state.aiMcpUrl}`,
      "通用": `{
  "transport": "streamable-http",
  "endpoint": "${state.aiMcpUrl}"
}`
    };

    const currentSnippet = snippets[state.aiClientTab] || snippets.ZCode;
    const statusHtml = `
      <div class="panel">
        <div class="panel-header"><h3 class="panel-title">连接状态</h3></div>
        <div style="display:flex;flex-direction:column;gap:10px">
          <div><span class="status-pill is-success">${SVG.dot}<span>本机服务可用</span></span></div>
          <div>
            ${
              state.aiClientConnected
                ? `<span class="status-pill is-success">${SVG.dot}<span>客户端已建立会话</span></span>`
                : `<span class="status-pill is-neutral">${SVG.hollowDot}<span>客户端未连接</span></span>`
            }
          </div>
        </div>
        <p style="margin:12px 0 0;font-size:13px;color:var(--ink-secondary)">添加配置后，在客户端发起一次工具调用。</p>
        <div class="inline-note is-neutral">${SVG.info}<span>扫描和检查可直接调用，擦写与重启需确认本次任务。</span></div>
      </div>`;

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">连接 AI 工具</h1>
            <p class="page-subtitle">复制当前服务配置到客户端，开始调用 LYNX。</p>
          </div>
        </div>
        <div class="two-col">
          <div class="col-primary">
            <div class="tab-pills" role="tablist" style="align-self:flex-start">
              ${["Codex", "ZCode", "Claude Code", "通用"]
                .map(
                  (c) => `<button class="tab-pill ${state.aiClientTab === c ? "is-active" : ""}" data-action="ai-client-tab" data-client="${c}">${c}</button>`
                )
                .join("")}
            </div>

            <div class="panel-white" style="display:flex;flex-direction:column;gap:10px">
              <label style="font-size:13px;font-weight:600;color:var(--ink-primary)">本机 MCP 地址</label>
              <div class="file-input-bar">
                <div class="file-input-display mono" style="height:40px">${escapeHtml(state.aiMcpUrl)}</div>
                <button class="btn btn-secondary" data-action="copy-text" data-copy="${escapeHtml(state.aiMcpUrl)}" data-label="已复制 MCP 地址">
                  ${SVG.copy}<span>复制地址</span>
                </button>
              </div>
              <div style="font-size:12px;color:var(--ink-muted)">端口取自当前运行状态</div>
            </div>

            <div class="code-panel">
              <div class="code-panel-head">
                <span>${escapeHtml(state.aiClientTab)} 配置</span>
                <button class="btn btn-secondary btn-sm" data-action="copy-text" data-copy="${escapeHtml(currentSnippet)}" data-label="已复制客户端配置">
                  ${SVG.copy}<span>复制配置</span>
                </button>
              </div>
              <pre class="code-pre">${escapeHtml(currentSnippet)}</pre>
            </div>

            <details class="inspector-mobile-wrap">
              <summary><span>连接状态</span>${SVG.chevronDown}</summary>
              <div style="margin-top:10px">${statusHtml}</div>
            </details>
          </div>
          <div class="col-inspector">${statusHtml}</div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <span style="font-size:13px;color:var(--ink-muted)">本机地址仅供这台电脑使用</span>
        </div>
        <div class="bottom-right">
          <button class="btn btn-secondary" data-action="ai-check-service">检查服务</button>
        </div>
      </div>
    `;
  }

  // C02: 远端通道
  function renderC02() {
    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">远端通道</h1>
            <p class="page-subtitle">分别核对本机服务与远端 SDK 主机的连接状态。</p>
          </div>
        </div>

        <div class="topo-strip" style="margin-bottom:18px">
          <span class="topo-node">${SVG.pc}<span>Windows 本机</span></span>
          <span class="topo-line"><span class="topo-dash"></span><span class="topo-badge">SSH 隧道</span><span class="topo-dash"></span></span>
          <span class="topo-node">${SVG.server}<span>远端 SDK (Ubuntu)</span></span>
        </div>

        <div class="two-col is-equal">
          <div class="panel-white" style="display:flex;flex-direction:column;gap:12px">
            <div style="display:flex;align-items:center;justify-content:space-between">
              <h3 class="panel-title">Windows 本机 MCP</h3>
              <span class="status-pill is-success">${SVG.checkSm}<span>本机 HTTP 访问通过</span></span>
            </div>
            <div class="file-input-bar">
              <div class="file-input-display mono">${escapeHtml(state.aiMcpUrl)}</div>
              <button class="btn btn-secondary btn-sm" data-action="copy-text" data-copy="${escapeHtml(state.aiMcpUrl)}" data-label="已复制本机地址">
                ${SVG.copy}<span>复制本机地址</span>
              </button>
            </div>
            <div style="font-size:12px;color:var(--ink-muted)">在运行 LYNX 的 Windows 电脑使用</div>
          </div>

          <div class="panel-white" style="display:flex;flex-direction:column;gap:12px">
            <div style="display:flex;align-items:center;justify-content:space-between">
              <h3 class="panel-title">远端 SDK 通道</h3>
              ${
                state.aiRemoteConnected
                  ? `<span class="status-pill is-success">${SVG.checkSm}<span>远端隧道已验证</span></span>`
                  : `<span class="status-pill is-amber">${SVG.alert}<span>远端连接失败</span></span>`
              }
            </div>
            <div class="file-input-bar">
              <div class="file-input-display ${state.aiRemoteConnected ? "mono" : "is-placeholder"}">
                ${state.aiRemoteConnected ? escapeHtml(state.aiRemoteUrl) : "尚无已验证地址"}
              </div>
              <button class="btn btn-secondary btn-sm" ${state.aiRemoteConnected ? `data-action="copy-text" data-copy="${escapeHtml(state.aiRemoteUrl)}" data-label="已复制远端地址"` : "disabled"}>
                ${SVG.copy}<span>复制远端地址</span>
              </button>
            </div>
            <div style="display:flex;align-items:center;justify-content:space-between;gap:10px">
              <span style="font-size:12px;color:${state.aiRemoteConnected ? "var(--success)" : "var(--danger)"}">
                ${state.aiRemoteConnected ? "SSH 隧道握手正常" : escapeHtml(state.aiRemoteError)}
              </span>
              <button class="btn btn-secondary btn-sm" data-action="ai-reconnect-remote">重新连接</button>
            </div>
          </div>
        </div>

        <div style="margin-top:16px">
          <details class="collapse-box">
            <summary><span>配置远端主机</span>${SVG.chevronRight}</summary>
            <div class="collapse-body">
              目标主机：ubuntu-sdk.local · 端口转发策略：按需建立本机回环隧道，不直接暴露公网端口。
            </div>
          </details>
          <div class="inline-note is-neutral">${SVG.info}<span>两端地址与验证结果独立，不能互相替代。</span></div>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <span style="font-size:13px;color:var(--ink-muted)">远端未验证时不提供可复制的旧地址</span>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="ai-recheck-all">重新检查</button>
        </div>
      </div>
    `;
  }

  // C03: AI 调用记录 & 请求确认背景表
  function renderC03() {
    const rowsHtml = state.aiCalls
      .map((c, idx) => {
        const pill =
          c.status === "待确认"
            ? `<span class="status-pill is-amber">${SVG.alert}<span>待确认</span></span>`
            : c.status === "已拒绝"
            ? `<span class="status-pill is-neutral">${SVG.hollowDot}<span>已拒绝</span></span>`
            : `<span class="status-pill is-success">${SVG.checkSm}<span>${escapeHtml(c.status)}</span></span>`;

        return `
          <tr class="${idx === 0 ? "is-row-selected" : ""}">
            <td class="mono tabular">${escapeHtml(c.time)}</td>
            <td><strong>${escapeHtml(c.client)}</strong></td>
            <td>${escapeHtml(c.op)}</td>
            <td>${pill}</td>
            <td style="text-align:right">
              ${
                c.status === "待确认"
                  ? `<button class="btn btn-secondary btn-sm" data-action="open-ai-drawer">处理确认</button>`
                  : `<span style="color:var(--ink-muted);font-size:12px">已记录</span>`
              }
            </td>
          </tr>`;
      })
      .join("");

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">AI 调用记录</h1>
            <p class="page-subtitle">查看外部工具的请求与执行结果。</p>
          </div>
        </div>
        <div class="data-table-wrap">
          <table class="data-table">
            <thead>
              <tr>
                <th style="width:120px">时间</th>
                <th style="width:160px">客户端</th>
                <th>操作</th>
                <th style="width:150px">状态</th>
                <th style="width:120px;text-align:right">操作</th>
              </tr>
            </thead>
            <tbody>${rowsHtml}</tbody>
          </table>
        </div>
      </div>
      <div class="bottom-action-bar ${state.aiDrawerOpen ? "is-dimmed" : ""}">
        <div class="bottom-left">
          <span style="font-size:13px;color:var(--ink-muted)">AI 请求与手动烧录使用同一确认保护模型</span>
        </div>
        <div class="bottom-right">
          <button class="btn btn-secondary" data-action="ai-nav" data-nav="local">返回本机通道</button>
        </div>
      </div>
    `;
  }

  // S01: 共享 - 任务记录
  function renderS01() {
    const q = state.recordsSearch.trim().toLowerCase();
    const list = state.records.filter((r) => {
      if (state.recordsFilter !== "全部任务" && r.source !== state.recordsFilter) return false;
      if (state.recordsStatusFilter !== "全部状态" && r.statusText !== state.recordsStatusFilter) return false;
      if (q && !`${r.board} ${r.image} ${r.id}`.toLowerCase().includes(q)) return false;
      return true;
    });

    const rowsHtml = list
      .map((r) => {
        let pillHtml = "";
        if (r.statusType === "pass-pending-boot") {
          pillHtml = `
            <div style="display:flex;align-items:center;gap:6px;flex-wrap:wrap">
              <span class="status-pill is-success">${SVG.checkSm}<span>${escapeHtml(r.statusText)}</span></span>
              <span class="status-pill is-amber">${SVG.hollowDot}<span>${escapeHtml(r.secondaryStatus)}</span></span>
            </div>`;
        } else if (r.statusType === "interrupted") {
          pillHtml = `<span class="status-pill is-amber">${SVG.alert}<span>${escapeHtml(r.statusText)}</span></span>`;
        } else {
          pillHtml = `<span class="status-pill is-neutral">${SVG.hollowDot}<span>${escapeHtml(r.statusText)}</span></span>`;
        }

        return `
          <tr>
            <td>
              <div style="font-weight:700;color:var(--ink-primary)">${escapeHtml(r.board)}</div>
              <div class="mono" style="font-size:12px;color:var(--ink-muted)">${escapeHtml(r.image)}</div>
            </td>
            <td>${escapeHtml(r.source)}</td>
            <td style="color:var(--ink-secondary)">${escapeHtml(r.time)}</td>
            <td>${pillHtml}</td>
            <td style="text-align:right">
              <button class="text-link" data-action="open-record-detail" data-id="${escapeHtml(r.id)}">${escapeHtml(r.actionText)}</button>
            </td>
          </tr>`;
      })
      .join("");

    return `
      <div class="app-main-scroll">
        <div class="page-header">
          <div>
            <h1 class="page-title">任务记录</h1>
            <p class="page-subtitle">每次执行的目标、来源和阶段结果。</p>
          </div>
        </div>
        <div class="filter-toolbar">
          <div class="search-box" style="max-width:460px">
            ${SVG.search}
            <input id="recordsSearchInput" class="search-input" type="text" placeholder="搜索板型或镜像" value="${escapeHtml(state.recordsSearch)}" />
          </div>
          <select class="field-select" data-action="records-status-filter" aria-label="按状态筛选">
            ${["全部状态", "写入校验通过", "写入中断", "预检未通过"]
              .map((s) => `<option value="${s}" ${state.recordsStatusFilter === s ? "selected" : ""}>${s}</option>`)
              .join("")}
          </select>
        </div>

        <div class="data-table-wrap">
          <table class="data-table">
            <thead>
              <tr>
                <th>设备与镜像</th>
                <th>来源</th>
                <th>时间</th>
                <th>结果</th>
                <th style="text-align:right">操作</th>
              </tr>
            </thead>
            <tbody>${rowsHtml || `<tr><td colspan="5" style="text-align:center;color:var(--ink-muted);padding:28px">暂无匹配记录</td></tr>`}</tbody>
          </table>
        </div>
        <div class="inline-note is-neutral" style="margin-top:14px">
          ${SVG.info}
          <span>恢复配置不会复用旧的设备绑定与烧录授权。</span>
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <span style="font-size:13px;color:var(--ink-muted)">共 ${list.length} 条记录</span>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="export-all-records">导出记录</button>
        </div>
      </div>
    `;
  }

  // S02: 共享 - 设置 (NO checkbox before "设置自动保存", bottom-right "完成")
  function renderS02() {
    const showAll = state.settingsNav === "general";
    const aboutBlock =
      state.settingsNav === "about"
        ? `
          <div class="panel-white">
            <div class="panel-header"><h3 class="panel-title">关于 LYNX Imager</h3></div>
            <div class="kv-list">
              <div class="kv-row"><span class="kv-key">版本定位</span><span class="kv-val">开源版 · 设计预览 (V5)</span></div>
              <div class="kv-row"><span class="kv-key">适用对象</span><span class="kv-val">全志 / 瑞芯微开发板系统镜像烧录与 MCP 调用管理</span></div>
              <div class="kv-row"><span class="kv-key">安全边界</span><span class="kv-val">不绕过预检、设备互斥与单次人工确认</span></div>
            </div>
          </div>`
        : "";

    return `
      <div class="app-main-scroll">
        <div class="settings-wrap">
          <div class="page-header" style="margin-bottom:4px">
            <div>
              <h1 class="page-title">设置</h1>
              <p class="page-subtitle">配置外观、更新与诊断选项。</p>
            </div>
          </div>

          ${
            showAll
              ? `<div class="panel-white">
                  <div class="panel-header"><h3 class="panel-title">常规</h3></div>
                  <div class="settings-row">
                    <div>
                      <div style="font-weight:600">语言</div>
                      <div style="font-size:12px;color:var(--ink-muted)">界面显示文字语言</div>
                    </div>
                    <select class="field-select" data-action="change-language" aria-label="界面语言">
                      <option value="zh-CN" ${state.language === "zh-CN" ? "selected" : ""}>简体中文</option>
                      <option value="en-US" ${state.language === "en-US" ? "selected" : ""}>English (Preview)</option>
                    </select>
                  </div>
                  <div class="settings-row">
                    <div>
                      <div style="font-weight:600">外观</div>
                      <div style="font-size:12px;color:var(--ink-muted)">切换浅色或深色界面</div>
                    </div>
                    <div class="tab-pills" role="radiogroup" aria-label="外观主题">
                      ${[
                        { id: "light", label: "浅色" },
                        { id: "dark", label: "深色" },
                        { id: "system", label: "跟随系统" }
                      ]
                        .map(
                          (t) => `<button class="tab-pill ${state.theme === t.id ? "is-active" : ""}" data-action="set-theme" data-theme="${t.id}">${t.label}</button>`
                        )
                        .join("")}
                    </div>
                  </div>
                  <div class="settings-row">
                    <div>
                      <div style="font-weight:600">动效</div>
                      <div style="font-size:12px;color:var(--ink-muted)">关闭后减少页面过渡与抽屉动画</div>
                    </div>
                    <button class="switch" data-action="toggle-motion" role="switch" aria-checked="${state.motionEnabled}">
                      <span class="switch-track ${state.motionEnabled ? "is-on" : ""}"><span class="switch-thumb"></span></span>
                      <span style="font-size:13px;font-weight:500">${state.motionEnabled ? "开启" : "已减少动效"}</span>
                    </button>
                  </div>
                </div>`
              : ""
          }

          ${
            showAll || state.settingsNav === "updates"
              ? `<div class="panel-white">
                  <div class="panel-header"><h3 class="panel-title">软件更新</h3></div>
                  <div class="settings-row">
                    <div>
                      <div style="font-weight:600">当前版本</div>
                      <div style="font-size:12px;color:var(--ink-muted)">设计预览</div>
                    </div>
                    <button class="btn btn-secondary btn-sm" data-action="check-update">检查更新</button>
                  </div>
                </div>`
              : ""
          }

          ${
            showAll || state.settingsNav === "logs"
              ? `<div class="panel-white">
                  <div class="panel-header"><h3 class="panel-title">诊断日志</h3></div>
                  <div class="settings-row">
                    <div>
                      <div style="font-weight:600">详细日志</div>
                      <div style="font-size:12px;color:var(--ink-muted)">排查问题时可临时开启</div>
                    </div>
                    <button class="switch" data-action="toggle-logs" role="switch" aria-checked="${state.detailedLogs}">
                      <span class="switch-track ${state.detailedLogs ? "is-on" : ""}"><span class="switch-thumb"></span></span>
                      <span style="font-size:13px;font-weight:500">${state.detailedLogs ? "开启" : "关闭"}</span>
                    </button>
                  </div>
                </div>`
              : ""
          }

          ${aboutBlock}
        </div>
      </div>
      <div class="bottom-action-bar">
        <div class="bottom-left">
          <span style="font-size:13px;color:var(--ink-muted)">设置自动保存</span>
        </div>
        <div class="bottom-right">
          <button class="btn btn-primary" data-action="back-from-settings">完成</button>
        </div>
      </div>
    `;
  }

  // =========================================================================
  // 7. RIGHT CONFIRMATION DRAWERS (B06 & C03) & MODALS
  // =========================================================================
  function renderOverlayDrawer() {
    if (state.confirmDrawerOpen && state.workspace === "basic" && state.basicStep === 3) {
      const board = findBoard(state.boardId);
      const usb = getSelectedUsb() || { port: "USB 1-3" };
      const img = state.localImageName || board.defaultImage;
      const size = state.localImageSize || board.defaultSize;
      return `
        <div class="drawer-backdrop" data-action="close-drawer-backdrop">
          <div class="drawer-panel" role="dialog" aria-modal="true" aria-labelledby="confirmDrawerTitle">
            <div class="drawer-head">
              <div>
                <h2 id="confirmDrawerTitle" class="drawer-title">确认本次烧录</h2>
                <p class="drawer-sub">请核对目标设备和将写入的系统。</p>
              </div>
              <button class="header-tool-btn" data-action="close-confirm-drawer" aria-label="关闭确认抽屉">${SVG.close}</button>
            </div>
            <div class="drawer-body">
              <div class="panel">
                <div class="kv-list">
                  <div class="kv-row"><span class="kv-key">设备</span><span class="kv-val">${escapeHtml(board.displayLabel || board.name)} · ${escapeHtml(usb.port)}</span></div>
                  <div class="kv-row"><span class="kv-key">镜像</span><span class="kv-val mono">${escapeHtml(img)} · ${escapeHtml(size)}</span></div>
                  <div class="kv-row"><span class="kv-key">镜像指纹</span><span class="kv-val mono">${escapeHtml(state.localImageFingerprint)}</span></div>
                  <div class="kv-row"><span class="kv-key">方式</span><span class="kv-val">${escapeHtml(state.localImageFormat)}</span></div>
                  <div class="kv-row"><span class="kv-key">校验</span><span class="kv-val">${state.verifyEnabled ? "开启" : "关闭"}</span></div>
                  <div class="kv-row"><span class="kv-key">结束动作</span><span class="kv-val">校验通过后${escapeHtml(state.postAction)}</span></div>
                </div>
              </div>

              <div class="inline-note is-amber" style="margin-top:0">
                ${SVG.alert}
                <span>目标设备中的原有系统与数据将被覆盖</span>
              </div>

              <label class="consent-label">
                <input id="basicConfirmCheckbox" type="checkbox" ${state.confirmChecked ? "checked" : ""} data-action="toggle-confirm-check" />
                <span>我已核对设备，并备份重要数据</span>
              </label>
            </div>
            <div class="drawer-foot">
              <button class="btn btn-secondary" data-action="close-confirm-drawer">返回修改</button>
              <button id="basicConfirmSubmitBtn" class="btn btn-primary" data-action="submit-confirm-flash" ${state.confirmChecked ? "" : "disabled"}>
                确认烧录
              </button>
            </div>
          </div>
        </div>`;
    }

    if (state.aiDrawerOpen && state.workspace === "ai" && state.aiNav === "history") {
      const req = state.aiCalls[0];
      return `
        <div class="drawer-backdrop" data-action="close-ai-backdrop">
          <div class="drawer-panel is-ai" role="dialog" aria-modal="true" aria-labelledby="aiDrawerTitle">
            <div class="drawer-head">
              <div>
                <h2 id="aiDrawerTitle" class="drawer-title">确认 AI 烧录请求</h2>
                <p class="drawer-sub">请求来源：${escapeHtml(req.client)} · 本机会话</p>
              </div>
              <button class="header-tool-btn" data-action="close-ai-drawer" aria-label="关闭">${SVG.close}</button>
            </div>
            <div class="drawer-body">
              <div class="panel">
                <div class="kv-list">
                  <div class="kv-row"><span class="kv-key">目标</span><span class="kv-val">${escapeHtml(req.target)}</span></div>
                  <div class="kv-row"><span class="kv-key">镜像</span><span class="kv-val mono">${escapeHtml(req.image)}</span></div>
                  <div class="kv-row"><span class="kv-key">文件指纹</span><span class="kv-val mono">${escapeHtml(req.fingerprint)}</span></div>
                  <div class="kv-row"><span class="kv-key">路线</span><span class="kv-val">${escapeHtml(req.route)}</span></div>
                  <div class="kv-row"><span class="kv-key">动作</span><span class="kv-val">${escapeHtml(req.action)}</span></div>
                </div>
              </div>

              <div class="inline-note is-amber" style="margin-top:0">
                ${SVG.alert}
                <span>将覆盖目标存储数据</span>
              </div>

              <label class="consent-label">
                <input id="aiConfirmCheckbox" type="checkbox" ${state.aiConfirmChecked ? "checked" : ""} data-action="toggle-ai-check" />
                <span>我已核对本次目标和镜像</span>
              </label>

              <div style="font-size:12px;color:var(--ink-muted)">本次授权不适用于后续任务</div>
            </div>
            <div class="drawer-foot">
              <button class="btn btn-secondary" data-action="reject-ai-req">拒绝请求</button>
              <button id="aiConfirmSubmitBtn" class="btn btn-primary" data-action="approve-ai-req" ${state.aiConfirmChecked ? "" : "disabled"}>
                确认本次烧录
              </button>
            </div>
          </div>
        </div>`;
    }
    return "";
  }

  function renderModal() {
    if (!state.modal) return "";
    const m = state.modal;
    return `
      <div class="modal-backdrop" data-action="close-modal">
        <div class="modal-card" role="dialog" aria-modal="true">
          <div class="modal-head">
            <strong style="font-size:16px">${escapeHtml(m.title)}</strong>
            <button class="header-tool-btn" data-action="close-modal" aria-label="关闭弹窗">${SVG.close}</button>
          </div>
          <div class="modal-body">${m.bodyHtml}</div>
          <div class="modal-foot">
            ${
              m.kind === "adv-to-basic-diff"
                ? `<button class="btn btn-secondary btn-sm" data-action="close-modal">留在高级烧录</button>
                   <button class="btn btn-primary btn-sm" data-action="confirm-reset-to-basic">恢复基础默认配置</button>`
                : `<button class="btn btn-primary btn-sm" data-action="close-modal">关闭</button>`
            }
          </div>
        </div>
      </div>`;
  }

  // =========================================================================
  // 8. GALLERY OVERVIEW MODE (ALL 17 CANONICAL SCREENS)
  // =========================================================================
  function renderGallery() {
    const groups = ["基础烧录", "高级烧录", "连接 AI", "共享页面"];
    return `
      <div class="gallery-wrap">
        ${groups
          .map((g) => {
            const items = PRESETS.filter((p) => p.group === g);
            return `
              <section>
                <h2 class="gallery-section-title">
                  <span>${g}</span>
                  <span class="status-pill is-neutral tabular">${items.length} 页</span>
                </h2>
                <div class="gallery-grid">
                  ${items
                    .map((item) => {
                      const hasDrawer = item.code === "B06" || item.code === "C03";
                      return `
                        <button class="gallery-card ${state.activePreset === item.code ? "is-current" : ""}" data-action="jump-preset" data-code="${item.code}">
                          <div class="gallery-card-top">
                            <span class="gallery-code">${item.code}</span>
                            <span style="font-size:12px;color:var(--indigo);font-weight:500">点击进入交互</span>
                          </div>
                          <div class="gallery-wireframe">
                            <div class="wf-side">
                              <div class="wf-bar is-accent" style="width:78%"></div>
                              <div class="wf-bar" style="width:90%"></div>
                              <div class="wf-bar" style="width:70%"></div>
                            </div>
                            <div class="wf-main">
                              <div class="wf-col">
                                <div class="wf-bar" style="width:55%;height:7px"></div>
                                <div class="wf-bar" style="width:92%;height:14px"></div>
                                <div class="wf-bar" style="width:82%;height:14px"></div>
                              </div>
                              <div style="width:30%;display:flex;flex-direction:column;gap:4px">
                                <div class="wf-bar" style="height:44px"></div>
                              </div>
                              ${
                                hasDrawer
                                  ? `<div class="wf-drawer"><div class="wf-bar is-accent" style="width:75%"></div><div class="wf-bar" style="width:90%"></div><div class="wf-bar" style="width:60%;margin-top:auto"></div></div>`
                                  : ""
                              }
                            </div>
                          </div>
                          <div>
                            <div style="font-size:14px;font-weight:700;color:var(--ink-primary)">${escapeHtml(item.title)}</div>
                            <div style="font-size:12px;color:var(--ink-secondary);margin-top:2px">${escapeHtml(item.desc)}</div>
                          </div>
                        </button>`;
                    })
                    .join("")}
                </div>
              </section>`;
          })
          .join("")}
      </div>
    `;
  }

  // =========================================================================
  // 9. PROTOTYPE TOP BAR + ROOT RENDER + RESPONSIVE SCALING
  // =========================================================================
  function renderProtoBar() {
    const idx = PRESETS.findIndex((p) => p.code === state.activePreset);
    const cur = PRESETS[idx] || PRESETS[0];
    const groups = [
      { label: "基础", items: PRESETS.filter((p) => p.group === "基础烧录") },
      { label: "高级", items: PRESETS.filter((p) => p.group === "高级烧录") },
      { label: "AI", items: PRESETS.filter((p) => p.group === "连接 AI") },
      { label: "共享", items: PRESETS.filter((p) => p.group === "共享页面") }
    ];

    return `
      <div class="proto-bar" role="region" aria-label="原型切换工具栏">
        <div class="proto-left">
          <span class="proto-brand"><span class="proto-brand-dot"></span><span>LYNX Imager V5</span></span>
          <div class="proto-groups">
            ${groups
              .map(
                (g) => `
              <div class="proto-group">
                <span class="proto-group-label">${g.label}</span>
                ${g.items
                  .map(
                    (it) => `<button class="proto-pill ${state.activePreset === it.code && state.viewMode !== "gallery" ? "is-active" : ""}"
                      data-action="jump-preset" data-code="${it.code}" title="${it.code} · ${escapeHtml(it.title)}">${it.code}</button>`
                  )
                  .join("")}
              </div>`
              )
              .join("")}
          </div>
        </div>
        <div class="proto-right">
          <div class="proto-nav-step">
            <button class="proto-icon-btn" data-action="prev-preset" ${idx <= 0 ? "disabled" : ""} title="上一页">${SVG.arrowLeft}</button>
            <span class="proto-current-label">${cur.code} · ${escapeHtml(cur.title)}</span>
            <button class="proto-icon-btn" data-action="next-preset" ${idx >= PRESETS.length - 1 ? "disabled" : ""} title="下一页">${SVG.arrowRight}</button>
          </div>
          <div class="proto-seg">
            <button class="${state.viewMode === "canvas" ? "is-active" : ""}" data-action="set-view-mode" data-mode="canvas">1536×1024 画布</button>
            <button class="${state.viewMode === "responsive" ? "is-active" : ""}" data-action="set-view-mode" data-mode="responsive">自适应铺满</button>
            <button class="${state.viewMode === "gallery" ? "is-active" : ""}" data-action="set-view-mode" data-mode="gallery">17 页画廊</button>
          </div>
          <button class="proto-pure-btn" data-action="toggle-pure" title="快捷键 H">纯享模式</button>
        </div>
      </div>
    `;
  }

  function applyThemeAndMotion() {
    const root = document.documentElement;
    let resolvedTheme = state.theme;
    if (resolvedTheme === "system" && typeof window !== "undefined" && window.matchMedia) {
      resolvedTheme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    }
    root.setAttribute("data-theme", resolvedTheme === "dark" ? "dark" : "light");
    root.setAttribute("data-motion", state.motionEnabled ? "normal" : "reduced");
  }

  function updateCanvasScale() {
    if (state.viewMode !== "canvas") return;
    const scaler = document.getElementById("canvasScaler");
    const win = document.getElementById("appWindow");
    if (!scaler || !win || typeof window === "undefined") return;

    const padW = state.pureMode ? 32 : 36;
    const padH = state.pureMode ? 32 : 96;
    const availW = Math.max(360, window.innerWidth - padW);
    const availH = Math.max(320, window.innerHeight - padH);
    const scale = Math.min(availW / 1536, availH / 1024, 1);

    scaler.style.width = `${Math.round(1536 * scale)}px`;
    scaler.style.height = `${Math.round(1024 * scale)}px`;
    win.style.transform = scale < 1 ? `scale(${scale.toFixed(4)})` : "none";
  }

  function render() {
    applyThemeAndMotion();
    const root = document.getElementById("app");
    if (!root) return;

    const bodyHtml =
      state.viewMode === "gallery"
        ? renderGallery()
        : `
      <div class="canvas-scaler" id="canvasScaler">
        <div class="app-window ${state.viewMode === "canvas" ? "mode-canvas" : "mode-responsive"}" id="appWindow">
          <div class="win-titlebar">
            <div class="win-titlebar-left">
              <span style="color:var(--indigo);display:inline-flex">${SVG.brand}</span>
              <span>LYNX Imager</span>
            </div>
            <div class="win-caption-btns" aria-hidden="true">
              <span class="win-caption-btn">${SVG.winMin}</span>
              <span class="win-caption-btn">${SVG.winMax}</span>
              <span class="win-caption-btn is-close">${SVG.winClose}</span>
            </div>
          </div>
          ${renderHeader()}
          <div class="app-body">
            ${renderSidebar()}
            <main class="app-main-shell">
              ${renderMainShell()}
            </main>
            ${renderOverlayDrawer()}
            ${renderModal()}
          </div>
          <footer class="global-footer">
            <span>开源版 · 无需登录</span>
            <span>${getFooterRightText()}</span>
          </footer>
        </div>
      </div>`;

    root.innerHTML = `
      <div class="studio-shell ${state.pureMode ? "is-pure" : ""}">
        <button class="proto-restore-btn" data-action="toggle-pure">退出纯享 [H]</button>
        ${renderProtoBar()}
        <div class="stage-wrap">${bodyHtml}</div>
      </div>
      <div id="toastHost">${state.toast ? `<div class="toast-banner">${SVG.info}<span>${escapeHtml(state.toast)}</span></div>` : ""}</div>
    `;

    updateCanvasScale();
    wireDropzoneAndInputs();
  }

  // =========================================================================
  // 10. EVENT DELEGATION, DRAG-DROP & KEYBOARD SAFETY
  // =========================================================================
  function wireDropzoneAndInputs() {
    const searchEl = document.getElementById("boardSearchInput");
    if (searchEl) {
      searchEl.oninput = (e) => {
        state.boardSearch = e.target.value;
        render();
        const nextEl = document.getElementById("boardSearchInput");
        if (nextEl) {
          nextEl.focus();
          nextEl.setSelectionRange(state.boardSearch.length, state.boardSearch.length);
        }
      };
    }

    const recSearchEl = document.getElementById("recordsSearchInput");
    if (recSearchEl) {
      recSearchEl.oninput = (e) => {
        state.recordsSearch = e.target.value;
        render();
        const nextEl = document.getElementById("recordsSearchInput");
        if (nextEl) {
          nextEl.focus();
          nextEl.setSelectionRange(state.recordsSearch.length, state.recordsSearch.length);
        }
      };
    }

    const localInput = document.getElementById("localFileInput");
    if (localInput) {
      localInput.onchange = (e) => {
        const f = e.target.files && e.target.files[0];
        if (!f) return;
        applyPickedImageFile(f.name, f.size);
      };
    }

    const loaderInput = document.getElementById("loaderFileInput");
    if (loaderInput) {
      loaderInput.onchange = (e) => {
        const f = e.target.files && e.target.files[0];
        if (!f) return;
        state.loaderFileName = f.name;
        state.confirmDrawerOpen = false;
        state.confirmChecked = false;
        showToast(`已载入 Loader：${f.name}`);
        render();
      };
    }

    const dropzone = document.getElementById("imageDropzone");
    if (dropzone) {
      dropzone.ondragover = (e) => {
        e.preventDefault();
        dropzone.classList.add("is-dragover");
      };
      dropzone.ondragleave = () => dropzone.classList.remove("is-dragover");
      dropzone.ondrop = (e) => {
        e.preventDefault();
        dropzone.classList.remove("is-dragover");
        const f = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
        if (f) applyPickedImageFile(f.name, f.size);
      };
    }
  }

  function applyPickedImageFile(name, sizeBytes) {
    const mb = sizeBytes > 0 ? `${(sizeBytes / (1024 * 1024)).toFixed(2)} MB` : "0 B";
    state.localImageName = name;
    state.localImageSize = sizeBytes >= 1024 * 1024 * 1024 ? `${(sizeBytes / (1024 ** 3)).toFixed(2)} GB` : mb;
    state.localImagePath = `本地文件\\${name}`;
    state.localImageReadable = sizeBytes > 0;
    state.localImageFormat = name.includes("rk") ? "Rockchip 完整镜像" : "全志厂家镜像";
    state.localImageFingerprint = "SHA-256 待预检计算";
    state.confirmDrawerOpen = false;
    state.confirmChecked = false;
    render();
  }

  function handleAction(action, target) {
    switch (action) {
      case "jump-preset":
        applyPreset(target.dataset.code);
        break;
      case "prev-preset": {
        const idx = PRESETS.findIndex((p) => p.code === state.activePreset);
        if (idx > 0) applyPreset(PRESETS[idx - 1].code);
        break;
      }
      case "next-preset": {
        const idx = PRESETS.findIndex((p) => p.code === state.activePreset);
        if (idx < PRESETS.length - 1) applyPreset(PRESETS[idx + 1].code);
        break;
      }
      case "set-view-mode":
        state.viewMode = target.dataset.mode;
        render();
        break;
      case "toggle-pure":
        state.pureMode = !state.pureMode;
        render();
        break;
      case "go-home":
        if (!isTaskLocked()) applyPreset("B01");
        break;
      case "switch-ws": {
        if (isTaskLocked()) return;
        const ws = target.dataset.ws;
        if (ws === "basic") {
          if (state.workspace === "advanced" && state.advModifiedFromBasic) {
            state.modal = {
              kind: "adv-to-basic-diff",
              title: "存在高级自定义参数",
              bodyHtml: "当前高级工作台包含基础流程无法表达的自定义参数（例如自定义烧录路线或擦除策略）。是否恢复为基础默认配置？"
            };
            render();
            return;
          }
          state.workspace = "basic";
          render();
        } else if (ws === "advanced") {
          state.workspace = "advanced";
          if (state.boardId === "rk3588-evb1") {
            applyPreset("A02");
          } else {
            applyPreset("A01");
          }
        } else if (ws === "ai") {
          state.workspace = "ai";
          render();
        }
        break;
      }
      case "confirm-reset-to-basic":
        state.modal = null;
        state.advModifiedFromBasic = false;
        applyPreset("B01");
        break;
      case "open-records":
        if (isTaskLocked()) return;
        state.prevWorkspace = state.workspace === "records" || state.workspace === "settings" ? state.prevWorkspace : state.workspace;
        state.workspace = "records";
        state.activePreset = "S01";
        render();
        break;
      case "open-settings":
        if (isTaskLocked()) return;
        state.prevWorkspace = state.workspace === "records" || state.workspace === "settings" ? state.prevWorkspace : state.workspace;
        state.workspace = "settings";
        state.activePreset = "S02";
        render();
        break;
      case "back-to-basic":
        state.workspace = "basic";
        render();
        break;
      case "back-from-settings":
        state.workspace = state.prevWorkspace || "basic";
        render();
        break;
      case "open-help":
        state.modal = {
          kind: "help",
          title: "LYNX Imager 使用帮助",
          bodyHtml: `
            <p style="margin-top:0"><strong>基础烧录（四步）</strong>：1. 选择具体板型 → 2. 选择板型资源或本地镜像 → 3. 连接 USB 并确认接口与 Loader → 4. 确认抽屉核对并执行。</p>
            <p><strong>安全边界</strong>：更换板型、镜像、设备或 Loader 会自动使旧预检与确认失效；写入、校验、重启与系统启动状态始终分开报告。</p>
            <p style="margin-bottom:0"><strong>快捷键</strong>：<kbd class="kbd">Ctrl</kbd>+<kbd class="kbd">K</kbd> 聚焦搜索 · <kbd class="kbd">Esc</kbd> 关闭未执行的抽屉 · <kbd class="kbd">H</kbd> 切换纯享模式。</p>`
        };
        render();
        break;
      case "open-support-list":
        state.modal = {
          kind: "support",
          title: "支持板型与后端路线清单",
          bodyHtml: `
            <div class="kv-list">
              <div class="kv-row"><span class="kv-key">T113-Pro (全志)</span><span class="kv-val">FEL · 厂家镜像 / 主线 NAND</span></div>
              <div class="kv-row"><span class="kv-key">F101 (全志)</span><span class="kv-val">FEL · 主线 SPI NOR + RAM FES Loader</span></div>
              <div class="kv-row"><span class="kv-key">RK3588-EVB1 / LubanCat-4 / Rock 5B</span><span class="kv-val">Maskrom / Loader · eMMC 完整镜像 / 厂家包</span></div>
            </div>`
        };
        render();
        break;
      case "open-board-doc": {
        const b = findBoard(state.boardId);
        state.modal = {
          kind: "board-doc",
          title: `${b.displayLabel || b.name} 板型说明`,
          bodyHtml: `<p style="margin-top:0">${escapeHtml(b.specSummary || "")}</p><p style="margin-bottom:0">默认模式：<strong>${escapeHtml(b.modeHint)}</strong> · 默认目标存储：<strong>${escapeHtml(b.storageDefault)}</strong>。</p>`
        };
        render();
        break;
      }
      case "open-release-notes":
        state.modal = {
          kind: "release",
          title: "NekoBox 发布说明 (示例资源)",
          bodyHtml: "<p style='margin-top:0'>目标板型：F101（SPI NOR 完整镜像）。</p><p style='margin-bottom:0'>烧录前须提供匹配该板型的 RAM FES Loader (fes1.fex)，并通过独立物理回读校验。</p>"
        };
        render();
        break;
      case "open-ai-guide":
        state.modal = {
          kind: "ai-guide",
          title: "AI MCP 接入说明",
          bodyHtml: "<p style='margin-top:0'>LYNX Imager 通过 MCP 暴露只读设备扫描与镜像检查能力；任何涉及擦写与重启的调用均会挂起为待确认请求，需在本机界面手动勾选确认后方可执行。</p><p style='margin-bottom:0'>不提供跳过确认的永久授权或 API Key 托管。</p>"
        };
        render();
        break;
      case "close-modal":
        state.modal = null;
        render();
        break;

      // Basic step 1
      case "board-filter":
        state.boardFilter = target.dataset.filter;
        render();
        break;
      case "toggle-rk-series":
        state.rkSeriesExpanded = !state.rkSeriesExpanded;
        if (!["rk3588-evb1", "lubancat-4", "rock-5b"].includes(state.boardId)) {
          state.boardId = "rk3588-series";
        }
        render();
        break;
      case "select-board": {
        const id = target.dataset.id;
        if (state.boardId !== id) {
          state.boardId = id;
          state.confirmDrawerOpen = false;
          state.confirmChecked = false;
          state.loaderFileName = "";
          const b = findBoard(id);
          state.imageSourceTab = b.hasOfficialResource ? "resource" : "local";
          state.localImageName = b.defaultImage || "";
          state.localImageSize = b.defaultSize || "";
          state.localImagePath = b.defaultPath || "";
          state.localImageFormat = b.vendor === "瑞芯微" ? "Rockchip 完整镜像" : b.id === "f101" ? "SPI NOR 完整镜像" : "全志厂家镜像";
        }
        render();
        break;
      }
      case "jump-basic-step":
        state.basicStep = Number(target.dataset.step);
        state.basicSubView = "normal";
        state.confirmDrawerOpen = false;
        render();
        break;
      case "b01-next": {
        const b = findBoard(state.boardId);
        if (b && !b.isSeries) {
          state.basicStep = 2;
          state.activePreset = b.hasOfficialResource ? "B02" : "B03";
          render();
        }
        break;
      }
      case "basic-back":
        if (state.basicStep > 1) {
          state.basicStep -= 1;
          state.confirmDrawerOpen = false;
          render();
        }
        break;
      case "image-source-tab":
        state.imageSourceTab = target.dataset.tab;
        state.activePreset = state.imageSourceTab === "resource" ? "B02" : "B03";
        render();
        break;
      case "select-resource":
        state.selectedResourceId = target.dataset.res || "";
        state.localImageName = findBoard(state.boardId).defaultImage || state.localImageName;
        showToast(`已选择板型资源：${findBoard(state.boardId).resourceName || "官方资源"}`);
        render();
        break;
      case "pick-local-file": {
        const inp = document.getElementById("localFileInput");
        if (inp) inp.click();
        break;
      }
      case "use-recent-image":
        state.localImageName = "t113-linux.img";
        state.localImageSize = "1.20 GB";
        state.localImagePath = "D:\\Images\\t113-linux.img";
        state.localImageReadable = true;
        state.localImageFormat = "全志厂家镜像";
        render();
        break;
      case "toggle-unreadable-demo":
        state.localImageReadable = !state.localImageReadable;
        render();
        break;
      case "b02-next":
      case "b03-next": {
        const b = findBoard(state.boardId);
        state.basicStep = 3;
        if (b.vendor === "瑞芯微") {
          state.usbList = [{ id: "usb-2-1", label: "Rockchip Maskrom", port: "USB 2-1", mode: "Maskrom" }];
          state.selectedUsbId = "usb-2-1";
          state.activePreset = state.loaderFileName ? "B04" : "B05";
        } else {
          state.usbList = [{ id: "usb-1-3", label: "Allwinner FEL", port: "USB 1-3", mode: "FEL" }];
          state.selectedUsbId = "usb-1-3";
          state.activePreset = b.needsLoader && !state.loaderFileName ? "B05" : "B04";
        }
        render();
        break;
      }
      case "usb-scenario": {
        const sc = target.dataset.sc;
        const b = findBoard(state.boardId);
        const mode = b.vendor === "瑞芯微" ? "Maskrom" : "FEL";
        const label = b.vendor === "瑞芯微" ? "Rockchip Maskrom" : "Allwinner FEL";
        state.usbScenario = sc;
        state.confirmDrawerOpen = false;
        state.confirmChecked = false;
        if (sc === "none") {
          state.usbList = [];
          state.selectedUsbId = "";
        } else if (sc === "multi") {
          state.usbList = [
            { id: "usb-1-3", label, port: "USB 1-3", mode },
            { id: "usb-2-4", label, port: "USB 2-4", mode }
          ];
          state.selectedUsbId = ""; // Enforce explicit selection!
        } else {
          state.usbList = [{ id: "usb-1-3", label, port: "USB 1-3", mode }];
          state.selectedUsbId = "usb-1-3";
        }
        render();
        break;
      }
      case "refresh-usb":
        showToast("已完成一次 USB 设备只读扫描");
        render();
        break;
      case "select-usb":
        state.selectedUsbId = target.dataset.id;
        state.confirmDrawerOpen = false;
        state.confirmChecked = false;
        render();
        break;
      case "pick-loader-file": {
        const inp = document.getElementById("loaderFileInput");
        if (inp) inp.click();
        break;
      }
      case "fill-sample-loader": {
        const b = findBoard(state.boardId);
        state.loaderFileName = b.defaultLoader || "rk3588_spl_loader_v1.15.bin";
        showToast(`已选择示例 Loader：${state.loaderFileName}`);
        render();
        break;
      }
      case "open-confirm-drawer":
        state.confirmDrawerOpen = true;
        state.confirmChecked = false;
        state.activePreset = "B06";
        render();
        break;
      case "close-confirm-drawer":
      case "close-drawer-backdrop":
        if (action === "close-drawer-backdrop" && !target.classList.contains("drawer-backdrop")) return;
        state.confirmDrawerOpen = false;
        state.confirmChecked = false;
        state.activePreset = "B04";
        render();
        break;
      case "toggle-confirm-check":
        state.confirmChecked = Boolean(target.checked);
        render();
        break;
      case "submit-confirm-flash":
        if (!state.confirmChecked) return;
        state.confirmDrawerOpen = false;
        state.basicStep = 4;
        state.basicSubView = "progress";
        state.activePreset = "B07";
        state.progressPct = 18;
        state.progressBytes = "216 MB / 1.20 GB";
        state.progressStage = "擦除";
        render();
        stopProgressTimer();
        state.progressTimer = setInterval(() => {
          if (state.progressPct < 62) {
            state.progressPct = 62;
            state.progressBytes = "744 MB / 1.20 GB";
            state.progressStage = "写入";
          } else if (state.progressPct < 88) {
            state.progressPct = 88;
            state.progressBytes = "1.06 GB / 1.20 GB";
            state.progressStage = "校验";
          } else {
            stopProgressTimer();
            state.basicSubView = "result";
            state.activePreset = "B08";
          }
          render();
        }, 900);
        break;
      case "sim-complete":
        stopProgressTimer();
        state.basicSubView = "result";
        state.activePreset = "B08";
        render();
        break;
      case "sim-fail":
        stopProgressTimer();
        state.basicSubView = "failed";
        state.activePreset = "B09";
        render();
        break;
      case "open-task-log":
        state.modal = {
          kind: "log",
          title: "任务 #F1024 执行日志",
          bodyHtml: `<pre class="code-pre" style="border:1px solid var(--border-subtle);border-radius:6px">[10:24:01] preflight: OK (T113-Pro @ USB 1-3)\n[10:24:04] erase: OK\n[10:25:41] write: 1.20 GB written (18 MB/s)\n[10:26:12] verify: readback checksum matched\n[10:26:18] post-action: reboot issued; boot pending user check</pre>`
        };
        render();
        break;
      case "export-report":
        showToast("已导出示例烧录报告 JSON");
        break;
      case "flash-another":
        applyPreset("B01");
        break;
      case "failed-back":
        state.basicStep = 3;
        state.basicSubView = "normal";
        state.activePreset = "B04";
        render();
        break;
      case "export-diag":
        showToast("已导出示例诊断记录与上下文快照");
        break;
      case "redetect-after-fail":
        state.basicStep = 3;
        state.basicSubView = "normal";
        state.confirmDrawerOpen = false;
        state.confirmChecked = false;
        state.activePreset = "B04";
        showToast("已重新发现设备，请核对并重新确认");
        render();
        break;

      // Advanced
      case "adv-redetect":
        showToast("已重新检测当前目标设备");
        break;
      case "adv-preset": {
        const p = target.dataset.preset;
        state.advPreset = p;
        if (p === "rk-emmc") applyPreset("A02");
        else if (p === "f101-default") applyPreset("A01");
        state.advPreset = p;
        render();
        break;
      }
      case "adv-pick-image":
        showToast("已重新读取镜像文件（需重新预检）");
        state.advPreflightStatus = "idle";
        render();
        break;
      case "adv-pick-loader":
        state.advLoaderName = state.advLoaderName ? "" : "fes1.fex";
        state.advPreflightStatus = state.advLoaderName ? "idle" : "issues";
        state.advLoaderFocusPulse = false;
        render();
        break;
      case "adv-run-preflight":
        if (state.advBoardId === "f101" && !state.advLoaderName) {
          state.advPreflightStatus = "issues";
          state.activePreset = "A03";
        } else {
          state.advPreflightStatus = "passed";
          showToast("预检通过，当前参数满足执行条件");
        }
        render();
        break;
      case "adv-locate-missing":
        state.advLoaderFocusPulse = true;
        render();
        break;
      case "adv-start-confirm":
        state.modal = {
          kind: "adv-confirm",
          title: "统一确认入口说明",
          bodyHtml: "高级烧录同样复用统一的右侧确认抽屉模型：需核对目标设备、指纹与存储覆盖警告后再开始写入。"
        };
        render();
        break;
      case "open-plan-modal":
        state.modal = {
          kind: "plan",
          title: "当前执行计划详情",
          bodyHtml: `<p style="margin-top:0">当前存储路线：<strong>${escapeHtml(state.advStorage)}</strong>（${escapeHtml(state.advRoute)}）</p><p style="margin-bottom:0">包含只读预检、${state.advBoardId === "f101" ? "物理布局校验与独立回读" : "eMMC 完整擦除与校验"}及校验后结束动作。</p>`
        };
        render();
        break;
      case "adv-save-cfg":
        showToast("已保存当前高级配置（仅保存文件与路线，不含旧设备授权）");
        break;
      case "adv-import-cfg":
        showToast("已载入配置示例（将触发重新预检）");
        state.advPreflightStatus = "idle";
        render();
        break;

      // Connect AI
      case "ai-nav":
        state.aiNav = target.dataset.nav;
        state.activePreset = state.aiNav === "local" ? "C01" : state.aiNav === "remote" ? "C02" : "C03";
        state.aiDrawerOpen = state.aiNav === "history" && state.aiCalls[0].status === "待确认";
        render();
        break;
      case "ai-client-tab":
        state.aiClientTab = target.dataset.client;
        render();
        break;
      case "copy-text":
        {
          const text = target.dataset.copy || "";
          const successLabel = target.dataset.label || "已复制到剪贴板";
          const failureLabel = "当前环境无法访问剪贴板，请手动复制";
          const report = (ok) => showToast(ok ? successLabel : failureLabel);
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(text).then(() => report(true), () => report(false));
          } else {
            const helper = document.createElement("textarea");
            helper.value = text;
            helper.setAttribute("readonly", "");
            helper.style.position = "fixed";
            helper.style.opacity = "0";
            document.body.appendChild(helper);
            helper.select();
            let copied = false;
            try {
              copied = document.execCommand("copy");
            } catch (_) {
              copied = false;
            }
            helper.remove();
            report(copied);
          }
        }
        break;
      case "ai-check-service":
        state.aiLocalHealthy = true;
        showToast("本机 MCP 服务运行正常 (http://127.0.0.1:8769/mcp)");
        render();
        break;
      case "ai-reconnect-remote":
      case "ai-recheck-all":
        showToast("远端 SSH 隧道仍被拒绝，未生成可复制的远端地址");
        render();
        break;
      case "open-ai-drawer":
        state.aiDrawerOpen = true;
        state.aiConfirmChecked = false;
        render();
        break;
      case "close-ai-drawer":
      case "close-ai-backdrop":
        if (action === "close-ai-backdrop" && !target.classList.contains("drawer-backdrop")) return;
        state.aiDrawerOpen = false;
        state.aiConfirmChecked = false;
        render();
        break;
      case "toggle-ai-check":
        state.aiConfirmChecked = Boolean(target.checked);
        render();
        break;
      case "reject-ai-req":
        state.aiCalls[0].status = "已拒绝";
        state.aiDrawerOpen = false;
        state.aiConfirmChecked = false;
        showToast("已拒绝该 AI 烧录请求，本次授权已失效");
        render();
        break;
      case "approve-ai-req":
        if (!state.aiConfirmChecked) return;
        state.aiCalls[0].status = "已完成";
        state.aiDrawerOpen = false;
        state.aiConfirmChecked = false;
        showToast("已单次确认并通过统一后端任务入口完成执行");
        render();
        break;

      // Records & Settings
      case "records-filter":
        state.recordsFilter = target.dataset.filter;
        render();
        break;
      case "open-record-detail": {
        const rec = state.records.find((r) => r.id === target.dataset.id);
        state.modal = {
          kind: "rec",
          title: `任务详情 ${rec ? rec.id : ""}`,
          bodyHtml: rec
            ? `<p style="margin-top:0">设备：<strong>${escapeHtml(rec.board)}</strong> · 镜像：<span class="mono">${escapeHtml(rec.image)}</span></p><p style="margin-bottom:0">来源：${escapeHtml(rec.source)} · 状态：${escapeHtml(rec.statusText)}</p>`
            : ""
        };
        render();
        break;
      }
      case "export-all-records":
        showToast("已导出全部示例任务记录");
        break;
      case "settings-nav":
        state.settingsNav = target.dataset.nav;
        render();
        break;
      case "set-theme":
        state.theme = target.dataset.theme;
        render();
        break;
      case "toggle-motion":
        state.motionEnabled = !state.motionEnabled;
        render();
        break;
      case "toggle-logs":
        state.detailedLogs = !state.detailedLogs;
        render();
        break;
      case "check-update":
        showToast("当前为设计预览版本，未连接线上更新源");
        break;
    }
  }

  function handleChange(action, target) {
    if (action === "change-post-action") {
      state.postAction = target.value;
      state.confirmChecked = false;
      render();
    } else if (action === "adv-change-route") {
      state.advRoute = target.value;
      state.advStorage = target.value.includes("SPI NOR") ? "SPI NOR" : target.value.includes("SPI NAND") ? "SPI NAND" : "eMMC";
      state.advModifiedFromBasic = true;
      state.advPreflightStatus = "idle";
      render();
    } else if (action === "adv-change-post") {
      state.advPostAction = target.value;
      state.advModifiedFromBasic = true;
      render();
    } else if (action === "records-status-filter") {
      state.recordsStatusFilter = target.value;
      render();
    } else if (action === "change-language") {
      state.language = target.value;
      showToast("语言设置已保存");
    }
  }

  function init() {
    document.addEventListener("click", (e) => {
      const el = e.target.closest("[data-action]");
      if (!el) return;
      handleAction(el.dataset.action, el);
    });

    document.addEventListener("change", (e) => {
      const el = e.target.closest("[data-action]");
      if (!el) return;
      handleChange(el.dataset.action, el);
    });

    document.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        const search = document.getElementById("boardSearchInput") || document.getElementById("recordsSearchInput");
        if (search) {
          e.preventDefault();
          search.focus();
        }
      } else if (e.key === "Escape") {
        if (state.modal) {
          state.modal = null;
          render();
        } else if (state.confirmDrawerOpen) {
          state.confirmDrawerOpen = false;
          state.confirmChecked = false;
          render();
        } else if (state.aiDrawerOpen) {
          state.aiDrawerOpen = false;
          state.aiConfirmChecked = false;
          render();
        }
      } else if (e.key.toLowerCase() === "h" && !["INPUT", "SELECT", "TEXTAREA"].includes(document.activeElement.tagName)) {
        state.pureMode = !state.pureMode;
        render();
      }
    });

    window.addEventListener("resize", updateCanvasScale);

    // Expose read-only test hooks for verification
    window.__LYNX_UI__ = {
      presets: PRESETS,
      getState: () => state,
      applyPreset
    };

    applyPreset("B01");
  }

  if (typeof document !== "undefined") {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", init);
    } else {
      init();
    }
  }
})();
