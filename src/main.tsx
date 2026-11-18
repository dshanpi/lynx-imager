import React, { useState, useEffect, useCallback } from 'react';
import ReactDOM from 'react-dom/client';
import { motion } from 'framer-motion';
import { Popup } from './CoreUI';
import { WorkbenchPage } from './Components/Workbench';
import { AISupervisorPage, type AISupervisorSection } from './Components/AISupervisor';
import { ImagerPage } from './Components/Imager/ImagerPage';
import { Settings, loadSettings } from './Settings';
import { authService, AuthUserInfo } from './Services';
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
import { invokeCommand, subscribeEvent } from './Platform/IPC';
import './Components/Imager/ImagerShell.css';

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
  const { setThemeMode } = useTheme();
  const [activeTool, setActiveTool] = useState('lynx-imager');
  const [settingsVisible, setSettingsVisible] = useState(false);
  const [hardwareBlockMessage, setHardwareBlockMessage] = useState('');
  // 当前登录用户（null 表示未登录）。登录结果经 auth-login-result 事件回传。
  const [authUser, setAuthUser] = useState<AuthUserInfo | null>(null);
  const clearExpiredAuthentication = useCallback(() => {
    setAuthUser(null);
  }, []);

  // Load saved settings on mount
  useEffect(() => {
    loadSettings().then(async (loadedSettings) => {
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
    void subscribeEvent('hardware-close-blocked', (activities) => {
      setHardwareBlockMessage(
        `检测到运行中的硬件任务，LYNX 已保持运行且未触碰设备：${activities.join('；')}`
      );
    }).then((value) => {
      unlisten = value;
    });
    return () => unlisten?.();
  }, []);

  // 订阅登录结果事件，并查询已有登录态（重启后从后端会话恢复）。
  useEffect(() => {
    let unreg: (() => void) | undefined;
    authService
      .onLoginResult((success, user, error) => {
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
        <ImagerPage onOpenAdvancedWorkbench={() => setActiveTool('workbench')} />
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
      <div className="imager-app-shell">
        <header className="imager-app-header">
          <div className="imager-brand" aria-label="LYNX Imager">
            <span className="imager-brand-mark" aria-hidden="true">L</span>
            <strong>LYNX Imager</strong>
          </div>
          <nav className="imager-app-tabs" aria-label="工作区">
            <button
              className={activeTool === 'lynx-imager' ? 'is-active' : ''}
              onClick={() => setActiveTool('lynx-imager')}
            >
              基础烧录
            </button>
            <button
              className={activeTool === 'workbench' ? 'is-active' : ''}
              onClick={() => setActiveTool('workbench')}
            >
              高级烧录
            </button>
            <button
              className={SUPERVISOR_SECTIONS[activeTool] ? 'is-active' : ''}
              onClick={() => setActiveTool('ai-supervisor-config')}
            >
              连接 AI
            </button>
          </nav>
          <div className="imager-app-actions">
            <button
              type="button"
              title="帮助"
              aria-label="帮助"
              onClick={() => void openUrl('https://github.com/dshanpi/lynx-imager')}
            >
              ?
            </button>
            <button type="button" title="设置" aria-label="设置" onClick={handleSettingsClick}>
              ⚙
            </button>
          </div>
        </header>
        <main className="imager-app-main">{renderAllTools()}</main>
        <footer className="imager-app-status">
          <span>开源版 · 专注系统烧录与 AI 接管</span>
          <span>LYNX Imager</span>
        </footer>
      </div>
      <Settings
        visible={settingsVisible}
        authenticated={Boolean(authUser)}
        onAuthExpired={clearExpiredAuthentication}
        onClose={() => setSettingsVisible(false)}
        onSettingsChange={() => undefined}
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
