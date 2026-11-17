import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import ReactDOM from 'react-dom/client';
import { useTranslation } from 'react-i18next';
import { motion } from 'framer-motion';
import { Layout, Popup, ToolItem, UserProfileDialog } from './CoreUI';
import { WorkbenchPage } from './Components/Workbench';
import { AISupervisorPage, type AISupervisorSection } from './Components/AISupervisor';
import { MCUWorkbenchPage } from './Components/MCUWorkbench';
import { RecordCenterPage } from './Components/RecordCenter';
import { LearningCenterPage, type LearningSection } from './Components/LearningCenter';
import { HardwareMonitorPage } from './Components/HardwareMonitor';
import { ImagerPage } from './Components/Imager/ImagerPage';
import { Settings, AppSettings, loadSettings } from './Settings';
import { authService, AuthUserInfo } from './Services';
import { flashManager } from './FlashManager';
import { efexService } from './Services';
import { ThemeProvider, useTheme, ThemeMode } from './Themes';
import './i18n';
import './Themes/tokens.css';
import './Themes/themes.css';
import './Themes/AIWorkspace.css';
import './Themes/EmbeddedWorkbenchTypography.css';
import './Themes/GlobalUI.css';
import './Themes/FlatUI.css';
import './Themes/LynxLightPages.css';
import i18n from './i18n';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openUrl } from '@tauri-apps/plugin-opener';
import { scheduleAutomaticUpdateCheck } from './Services/AutoUpdater';
import { installDiagnosticReporter } from './Services/DiagnosticReporter';
import { invokeCommand, subscribeEvent, type LearningDeepLinkTarget } from './Platform/IPC';

const SUPERVISOR_SECTIONS: Record<string, AISupervisorSection> = {
  'ai-supervisor-overview': 'overview',
  'ai-supervisor-config': 'config',
  'ai-supervisor-automatic': 'automatic',
  'ai-supervisor-runtime': 'runtime',
  // Compatibility aliases for persisted navigation IDs from v0.3 previews.
  'ai-supervisor-providers': 'config',
  'ai-supervisor-policies': 'config',
  'ai-supervisor-permissions': 'config',
  'ai-supervisor-human': 'runtime',
  'ai-supervisor-audit': 'runtime',
};

const LEARNING_SECTIONS: Record<string, LearningSection> = {
  'learning-home': 'home',
  'learning-hardware': 'hardware',
  'learning-courses': 'courses',
  'learning-labs': 'labs',
  'learning-history': 'history',
};

/**
 * Shows the application window.
 *
 * Called after initialization to display the main window,
 * avoiding the white flash during React hydration.
 */
async function showAppWindow() {
  const appWindow = getCurrentWindow();
  appWindow.show();
}

/**
 * Main application content component.
 *
 * Renders the complete application UI including:
 * - Sidebar navigation with tool selection
 * - Multiple tool page panels (firmware flash, sector flash, etc.)
 * - Settings modal overlay
 *
 * Manages state for:
 * - Active tool selection
 * - Sidebar collapse state
 * - Working/flashing state (locks sidebar)
 * - Settings visibility
 */
const AppContent: React.FC = () => {
  const { t } = useTranslation();
  const { setThemeMode } = useTheme();
  const [activeTool, setActiveTool] = useState('lynx-imager');
  const [boardPublicationView, setBoardPublicationView] = useState(false);
  useEffect(() => {
    const update = (event: Event) =>
      setBoardPublicationView(
        (event as CustomEvent<{ publication?: boolean }>).detail?.publication === true
      );
    window.addEventListener('lynx:firmware-navigation', update);
    return () => window.removeEventListener('lynx:firmware-navigation', update);
  }, []);

  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [settingsVisible, setSettingsVisible] = useState(false);
  const [isWorking, setIsWorking] = useState(false);
  const [hardwareBlockMessage, setHardwareBlockMessage] = useState('');
  // 当前登录用户（null 表示未登录）。登录结果经 auth-login-result 事件回传。
  const [authUser, setAuthUser] = useState<AuthUserInfo | null>(null);
  const [loginPending, setLoginPending] = useState(false);
  const loginAttempt = useRef(0);
  const loginLocked = useRef(false);
  // 用户信息弹窗是否显示（点击用户名按钮打开）。
  const [profileVisible, setProfileVisible] = useState(false);
  const [learningDeepLink, setLearningDeepLink] = useState<LearningDeepLinkTarget | null>(null);

  const clearExpiredAuthentication = useCallback(() => {
    setAuthUser(null);
    setProfileVisible(false);
  }, []);

  /**
   * Tool items configuration for sidebar.
   *
   * Defines all available tools with their IDs, names,
   * icons, and descriptions for sidebar navigation.
   */
  const tools: ToolItem[] = useMemo(
    () => [
      {
        id: 'lynx-imager',
        name: '系统烧录',
        icon: 'download',
        description: '基础烧录向导、专业路线和 AI 连接入口',
      },
      {
        id: 'development-workbenches',
        name: '嵌入式开发工作台',
        icon: 'workbench',
        description: 'Linux 与 MCU 开发板工作台',
        children: [
          {
            id: 'workbench',
            name: 'Linux 工作台',
            icon: 'server',
            description: t('workbench.description'),
          },
          {
            id: 'mcu-workbench',
            name: 'MCU 工作台',
            icon: 'board',
            description: '使用 ST-Link、DAPLink 接管 MCU 烧写、复位与启动测试',
          },
        ],
      },
      {
        id: 'learning-center',
        name: '学习中心',
        icon: 'learning',
        description: '课程、实验与学习记录',
        children: [
          {
            id: 'learning-home',
            name: '学习指南',
            icon: 'learning',
            description: '继续学习与待处理事项',
          },
          {
            id: 'learning-hardware',
            name: '我的硬件',
            icon: 'box',
            description: '订单兑换、已绑定设备与资料引导',
          },
          {
            id: 'learning-courses',
            name: '我的课程',
            icon: 'courses',
            description: '查看已拥有的官网课程',
          },
          {
            id: 'learning-labs',
            name: '实验中心',
            icon: 'lab',
            description: '启动、恢复与检查课程实验',
          },
          {
            id: 'learning-history',
            name: '学习记录',
            icon: 'history',
            description: '实验历史、报告与同步状态',
          },
        ],
      },
      {
        id: 'ai-channel',
        name: t('tools.aiGateway.name', 'AI 通道'),
        icon: 'agent',
        description: t('tools.aiGateway.description', '连接 Codex、Claude Code 与硬件工具'),
        children: [
          {
            id: 'ai-supervisor-runtime',
            name: '运行中心',
            icon: 'runtime',
            description: '统一处理运行、人工介入与审计证据',
          },
          {
            id: 'ai-supervisor-config',
            name: '配置中心',
            icon: 'settings',
            description: '统一配置 AI 服务、策略、预算与动作权限',
          },
          {
            id: 'ai-supervisor-automatic',
            name: '创建任务',
            icon: 'task',
            description: '配置定时、Webhook 和设备事件任务',
          },
          {
            id: 'ai-supervisor-overview',
            name: '监督总览',
            icon: 'overview',
            description: 'AI 服务、恢复预算与监督状态',
          },
        ],
      },
      {
        id: 'hardware-monitor',
        name: t('tools.hardwareMonitor.name', '硬件监视'),
        icon: 'lab',
        description: t(
          'tools.hardwareMonitor.description',
          'CH347 四路电源、SPI 显示与 AI 通道实时监视'
        ),
      },
      {
        id: 'record-center',
        name: '记录中心',
        icon: 'records',
        description: '统一追踪事件、会话、反馈与本地备份',
      },
    ],
    [t]
  );

  // Load saved settings on mount
  useEffect(() => {
    loadSettings().then(async (loadedSettings) => {
      setSidebarCollapsed(loadedSettings.sidebarCollapsed);
      if (loadedSettings.language) {
        i18n.changeLanguage(loadedSettings.language);
      }
      if (loadedSettings.themeMode) {
        setThemeMode(loadedSettings.themeMode);
      }
      try {
        await efexService.setUsbBackend(loadedSettings.usbBackend);
      } catch (e) {
        console.error('Failed to set USB backend:', e);
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Hide loading screen after React hydrates
  useEffect(() => {
    const loadingScreen = document.getElementById('loading-screen');
    if (loadingScreen) {
      loadingScreen.classList.add('hidden');
      setTimeout(() => {
        loadingScreen.remove();
      }, 500);
    }
  }, []);

  // Show window after initialization
  useEffect(() => {
    showAppWindow();
  }, []);

  // Check the signed public GitHub release feed shortly after startup.
  useEffect(() => scheduleAutomaticUpdateCheck(), []);

  // Capture only global software failures. Rust enforces the reporting opt-in,
  // fixed schema, redaction, queue bounds and authenticated upload boundary.
  useEffect(() => installDiagnosticReporter(), []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const openLearningTarget = (target: LearningDeepLinkTarget) => {
      setLearningDeepLink({ ...target });
      setActiveTool(target.labKey ? 'learning-labs' : 'learning-courses');
    };
    void subscribeEvent('learning-deep-link', openLearningTarget).then((value) => {
      unlisten = value;
      void invokeCommand('learning_take_pending_deep_link').then((pending) => {
        if (pending) openLearningTarget(pending);
      });
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void subscribeEvent('hardware-close-blocked', (activities) => {
      setHardwareBlockMessage(
        `检测到运行中的硬件任务，LYNX 已保持运行且未触碰设备：${activities.join('；')}`
      );
    }).then((value) => {
      unlisten = value;
    });
    return () => unlisten?.();
  }, []);

  // Track flash manager working state
  useEffect(() => {
    const unsubWorkingChange = flashManager.onWorkingChange((working) => {
      setIsWorking(working);
    });

    setIsWorking(flashManager.getIsFlashing());

    return () => {
      unsubWorkingChange();
    };
  }, []);

  // 订阅登录结果事件，并查询已有登录态（重启后从后端会话恢复）。
  useEffect(() => {
    let unreg: (() => void) | undefined;
    authService
      .onLoginResult((success, user, error) => {
        loginAttempt.current++;
        loginLocked.current = false;
        setLoginPending(false);
        if (success && user) {
          setAuthUser(user);
          void invokeCommand('diagnostics_set_enabled', { enabled: true })
            .then(() => invokeCommand('diagnostics_sync_now'))
            .catch((diagnosticError) => {
              // 登录本身已经成功；诊断失败保持本地队列并由状态页呈现，不影响登录结果。
              console.warn('登录后自动开启诊断上报失败:', diagnosticError);
            });
        } else if (error) {
          clearExpiredAuthentication();
          console.error('登录失败:', error);
          alert(error);
        }
      })
      .then((fn) => {
        unreg = fn;
      });
    // 恢复已有会话
    authService
      .getUser()
      .then((user) => {
        if (user) setAuthUser(user);
        else clearExpiredAuthentication();
      })
      .catch(() => {
        clearExpiredAuthentication();
        alert('登录已过期，请重新登录');
      });
    return () => {
      unreg?.();
    };
  }, [clearExpiredAuthentication]);

  // 已登录期间持续复核后端 Bearer 会话；一旦刷新或 userinfo 校验失败，立即撤销 UI 登录投影。
  useEffect(() => {
    if (!authUser) return;
    let active = true;
    const interval = window.setInterval(() => {
      void authService
        .getUser()
        .then((user) => {
          if (active && !user) {
            clearExpiredAuthentication();
          }
        })
        .catch(() => {
          if (!active) return;
          active = false;
          window.clearInterval(interval);
          clearExpiredAuthentication();
          alert('登录已过期，请重新登录');
        });
    }, 30_000);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, [authUser, clearExpiredAuthentication]);

  /**
   * Handles settings changes from Settings modal.
   *
   * @param newSettings - Updated AppSettings
   */
  const handleSettingsChange = (newSettings: AppSettings) => {
    setSidebarCollapsed(newSettings.sidebarCollapsed);
  };

  /**
   * Handles tool selection from sidebar.
   *
   * @param toolId - Selected tool ID
   */
  const handleToolSelect = useCallback((toolId: string) => {
    setActiveTool(toolId);
  }, []);

  /**
   * Handles sidebar collapse toggle.
   */
  const handleToggleSidebar = useCallback(() => {
    setSidebarCollapsed((prev) => !prev);
  }, []);

  /**
   * Handles login button click.
   *
   * 未登录 → 启动 100ask.net OAuth2 登录（后端起本机回调服务并打开浏览器授权），
   *   登录结果经 auth-login-result 事件回传并更新 authUser；
   * 已登录 → 打开用户信息弹窗（在弹窗底部退出登录）。
   */
  const handleLoginClick = useCallback(() => {
    if (authUser) {
      authService
        .getUser()
        .then((user) => {
          setAuthUser(user);
          setProfileVisible(Boolean(user));
        })
        .catch(() => {
          clearExpiredAuthentication();
          alert('登录已过期，请重新登录');
        });
      return;
    }
    if (loginLocked.current) return;
    loginLocked.current = true;
    const attempt = ++loginAttempt.current;
    setLoginPending(true);
    authService
      .login()
      .then(async ({ authorizeUrl }) => {
        if (attempt !== loginAttempt.current) return;
        try {
          await openUrl(authorizeUrl);
          // The learning prompt keeps its explicit waiting/cancel state. Other
          // existing login entries may still restart an abandoned browser flow.
          if (attempt === loginAttempt.current) loginLocked.current = false;
        } catch (error) {
          if (attempt !== loginAttempt.current) return;
          // 浏览器没有成功打开时立即释放回调端口和 pending，允许用户直接重试。
          await authService.cancelLogin().catch(() => undefined);
          throw error;
        }
      })
      .catch((e) => {
        if (attempt !== loginAttempt.current) return;
        loginLocked.current = false;
        setLoginPending(false);
        console.error('启动登录失败:', e);
        alert(typeof e === 'string' ? e : ((e as { message?: string })?.message ?? '启动登录失败'));
      });
  }, [authUser, clearExpiredAuthentication]);

  const handleCancelLogin = useCallback(() => {
    const attempt = ++loginAttempt.current;
    loginLocked.current = true;
    void authService
      .cancelLogin()
      .then(() => {
        if (attempt !== loginAttempt.current) return;
        loginLocked.current = false;
        setLoginPending(false);
      })
      .catch(() => {
        if (attempt === loginAttempt.current) alert('取消登录失败，请稍后重试。');
      });
  }, []);

  const handleSettingsClick = useCallback(() => {
    authService
      .getUser()
      .then((user) => setAuthUser(user))
      .catch(() => {
        clearExpiredAuthentication();
        alert('登录已过期，请重新登录');
      })
      .finally(() => setSettingsVisible(true));
  }, [clearExpiredAuthentication]);

  /**
   * 退出登录：调用后端清空会话，更新本地状态并关闭用户信息弹窗。
   */
  const handleLogout = useCallback(() => {
    setProfileVisible(false);
    authService.logout().then(() => setAuthUser(null));
  }, []);

  /** Page fade animation variants */
  const pageVariants = {
    visible: { opacity: 1 },
    hidden: { opacity: 0 },
  };

  /** Page transition duration */
  const pageTransition = {
    duration: 0.15,
  };

  /**
   * Renders all tool panels.
   *
   * Uses visibility toggling with opacity animations for smooth
   * transitions between tools. All panels exist in DOM for state
   * preservation, but only active one is visible and interactive.
   */
  const renderAllTools = () => (
    <div className="tool-panels-container">
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        animate={activeTool === 'lynx-imager' ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: activeTool === 'lynx-imager' ? 'auto' : 'none',
          visibility: activeTool === 'lynx-imager' ? 'visible' : 'hidden',
          zIndex: activeTool === 'lynx-imager' ? 1 : 0,
        }}
      >
        <ImagerPage
          onOpenAdvanced={() => setActiveTool('workbench')}
          onOpenAi={() => setActiveTool('ai-supervisor-config')}
          onOpenLegacyWorkbench={() => setActiveTool('workbench')}
        />
      </motion.div>
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        animate={activeTool === 'hardware-monitor' ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: activeTool === 'hardware-monitor' ? 'auto' : 'none',
          visibility: activeTool === 'hardware-monitor' ? 'visible' : 'hidden',
          zIndex: activeTool === 'hardware-monitor' ? 1 : 0,
        }}
      >
        <HardwareMonitorPage isActive={activeTool === 'hardware-monitor'} />
      </motion.div>
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        animate={LEARNING_SECTIONS[activeTool] ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: LEARNING_SECTIONS[activeTool] ? 'auto' : 'none',
          visibility: LEARNING_SECTIONS[activeTool] ? 'visible' : 'hidden',
          zIndex: LEARNING_SECTIONS[activeTool] ? 1 : 0,
        }}
      >
        <LearningCenterPage
          key={authUser?.id ?? 'anonymous'}
          isActive={Boolean(LEARNING_SECTIONS[activeTool])}
          section={LEARNING_SECTIONS[activeTool] ?? 'home'}
          authUser={authUser}
          onRequestLogin={handleLoginClick}
          loginPending={loginPending}
          onCancelLogin={handleCancelLogin}
          onSectionChange={(nextSection) => setActiveTool(`learning-${nextSection}`)}
          deepLinkTarget={learningDeepLink}
          onOpenLink={setLearningDeepLink}
          onDeepLinkHandled={() => setLearningDeepLink(null)}
        />
      </motion.div>
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        animate={activeTool === 'record-center' ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: activeTool === 'record-center' ? 'auto' : 'none',
          visibility: activeTool === 'record-center' ? 'visible' : 'hidden',
          zIndex: activeTool === 'record-center' ? 1 : 0,
        }}
      >
        <RecordCenterPage
          isActive={activeTool === 'record-center'}
          actorId={authUser ? `user:${authUser.id}` : 'local-user'}
        />
      </motion.div>
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        animate={activeTool === 'mcu-workbench' ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: activeTool === 'mcu-workbench' ? 'auto' : 'none',
          visibility: activeTool === 'mcu-workbench' ? 'visible' : 'hidden',
          zIndex: activeTool === 'mcu-workbench' ? 1 : 0,
        }}
      >
        <MCUWorkbenchPage isActive={activeTool === 'mcu-workbench'} />
      </motion.div>
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        animate={SUPERVISOR_SECTIONS[activeTool] ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: SUPERVISOR_SECTIONS[activeTool] ? 'auto' : 'none',
          visibility: SUPERVISOR_SECTIONS[activeTool] ? 'visible' : 'hidden',
          zIndex: SUPERVISOR_SECTIONS[activeTool] ? 1 : 0,
        }}
      >
        <AISupervisorPage
          section={SUPERVISOR_SECTIONS[activeTool] ?? 'overview'}
          isActive={Boolean(SUPERVISOR_SECTIONS[activeTool])}
        />
      </motion.div>
      <motion.div
        className="tool-panel"
        variants={pageVariants}
        data-active-workbench={activeTool === 'workbench'}
        animate={activeTool === 'workbench' ? 'visible' : 'hidden'}
        transition={pageTransition}
        style={{
          pointerEvents: activeTool === 'workbench' ? 'auto' : 'none',
          visibility: activeTool === 'workbench' ? 'visible' : 'hidden',
          zIndex: activeTool === 'workbench' ? 1 : 0,
        }}
      >
        <WorkbenchPage isActive={activeTool === 'workbench'} />
      </motion.div>
      {/* {activeTool === 'dram-tunning' && (
        <motion.div
          className="tool-panel active"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={pageTransition}
        >
          <DRAMTunningPage isActive={true} />
        </motion.div>
      )} */}
    </div>
  );

  return (
    <>
      <Layout
        className={LEARNING_SECTIONS[activeTool] ? 'learn-design-surface' : ''}
        tools={tools}
        activeTool={
          activeTool === 'workbench' && boardPublicationView ? 'ai-supervisor-runtime' : activeTool
        }
        onToolSelect={handleToolSelect}
        sidebarCollapsed={sidebarCollapsed}
        onToggleSidebar={handleToggleSidebar}
        onSettingsClick={handleSettingsClick}
        onLoginClick={handleLoginClick}
        authUserName={authUser?.name ?? null}
        sidebarLocked={isWorking}
      >
        {renderAllTools()}
      </Layout>
      <Settings
        visible={settingsVisible}
        authenticated={Boolean(authUser)}
        onAuthExpired={clearExpiredAuthentication}
        onClose={() => setSettingsVisible(false)}
        onSettingsChange={handleSettingsChange}
      />
      <UserProfileDialog
        visible={profileVisible}
        user={authUser}
        onClose={() => setProfileVisible(false)}
        onLogout={handleLogout}
      />
      <Popup
        visible={Boolean(hardwareBlockMessage)}
        type="warning"
        title="硬件任务保护"
        message={hardwareBlockMessage}
        onClose={() => setHardwareBlockMessage('')}
      />
    </>
  );
};

/**
 * Initial settings for ThemeProvider.
 *
 * Used to delay rendering until settings are loaded,
 * ensuring correct initial theme state.
 */
interface InitialSettings {
  /** Theme mode preference */
  themeMode: ThemeMode;
  /** Dark theme identifier */
  themeIdDark: string;
  /** Light theme identifier */
  themeIdLight: string;
}

const FALLBACK_INITIAL_SETTINGS: InitialSettings = {
  themeMode: 'light',
  themeIdDark: 'lynx-dark',
  themeIdLight: 'lynx-light',
};

/**
 * Root App component with settings loading.
 *
 * Loads settings before rendering the full workspace. A visible startup shell
 * and a bounded fallback prevent a filesystem/plugin stall from becoming an
 * indefinite blank window.
 */
const App: React.FC = () => {
  const [initialSettings, setInitialSettings] = useState<InitialSettings | null>(null);

  useEffect(() => {
    let active = true;
    const fallbackTimer = window.setTimeout(() => {
      if (active) setInitialSettings((current) => current ?? FALLBACK_INITIAL_SETTINGS);
    }, 1500);

    void showAppWindow().catch((error) =>
      console.error('Failed to show application window:', error)
    );
    void loadSettings()
      .then((settings) => {
        if (!active) return;
        setInitialSettings({
          themeMode: settings.themeMode || FALLBACK_INITIAL_SETTINGS.themeMode,
          themeIdDark: settings.themeIdDark || FALLBACK_INITIAL_SETTINGS.themeIdDark,
          themeIdLight: settings.themeIdLight || FALLBACK_INITIAL_SETTINGS.themeIdLight,
        });
      })
      .catch((error) => {
        console.error('Failed to initialize application settings:', error);
        if (active) setInitialSettings(FALLBACK_INITIAL_SETTINGS);
      })
      .finally(() => window.clearTimeout(fallbackTimer));

    return () => {
      active = false;
      window.clearTimeout(fallbackTimer);
    };
  }, []);

  if (initialSettings === null) {
    return (
      <div className="app-startup-shell" role="status" aria-live="polite">
        <div className="app-startup-mark">LYNX</div>
        <div className="app-startup-spinner" aria-hidden="true" />
        <p>正在加载工作台…</p>
      </div>
    );
  }

  return (
    <ThemeProvider
      key={`${initialSettings.themeMode}-${initialSettings.themeIdDark}-${initialSettings.themeIdLight}`}
      initialMode={initialSettings.themeMode}
      initialThemeIdDark={initialSettings.themeIdDark}
      initialThemeIdLight={initialSettings.themeIdLight}
    >
      <AppContent />
    </ThemeProvider>
  );
};

/**
 * Application entry point.
 *
 * Renders the App component into the root element with
 * React.StrictMode for development checks.
 */
ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
