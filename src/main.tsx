import React, { useState, useEffect, useCallback, useRef } from 'react';
import ReactDOM from 'react-dom/client';
import { LynxIcon } from './CoreUI/LynxIcon/LynxIcon';
import { Popup } from './CoreUI';
import { WorkbenchPage } from './Components/Workbench';
import { ImagerAIWorkspace } from './Components/Imager/ImagerAIWorkspace';
import { TaskRecords } from './Components/Imager/TaskRecords';
import { ImagerPage } from './Components/Imager/ImagerPage';
import { useAppVersion } from './Hooks/useAppVersion';
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
import './Components/Imager/FlashLight.css';

type Workspace = 'basic' | 'advanced' | 'ai';
type WorkspaceStatus = { target: string; task: string };
const workspaceNames: Record<Workspace, string> = {
  basic: '烧录系统',
  advanced: '高级功能',
  ai: '连接 AI',
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
  const { setThemeMode, isDark, currentThemeId, currentColors } = useTheme();
  const version = useAppVersion();
  const [activeTool, setActiveTool] = useState<Workspace>('basic');
  const [visited, setVisited] = useState<Workspace[]>(['basic']);
  const [flashTarget, setFlashTarget] = useState<{ deviceId: number; sequence: number }>();
  const [recordsOpen, setRecordsOpen] = useState(false);
  const [statuses, setStatuses] = useState<Partial<Record<Workspace, WorkspaceStatus>>>({});
  const rememberStatus = useCallback((workspace: Workspace, next: WorkspaceStatus) => {
    setStatuses((old) =>
      old[workspace]?.target === next.target && old[workspace]?.task === next.task
        ? old
        : { ...old, [workspace]: next }
    );
  }, []);
  const basicStatus = useCallback(
    (s: WorkspaceStatus) => rememberStatus('basic', s),
    [rememberStatus]
  );
  const advancedStatus = useCallback(
    (s: WorkspaceStatus) => rememberStatus('advanced', s),
    [rememberStatus]
  );
  const aiStatus = useCallback((s: WorkspaceStatus) => rememberStatus('ai', s), [rememberStatus]);
  const activate = useCallback((key: Workspace) => {
    setVisited((keys) => (keys.includes(key) ? keys : [...keys, key]));
    setActiveTool(key);
  }, []);
  const openFlash = useCallback(
    (deviceId: number) => {
      setFlashTarget((old) => ({ deviceId, sequence: (old?.sequence ?? 0) + 1 }));
      activate('advanced');
    },
    [activate]
  );
  const openRecords = useCallback(() => setRecordsOpen(true), []);
  const settingsTrigger = useRef<HTMLElement | null>(null);
  const displayedStatus = statuses[activeTool];
  const alternateStyle = currentThemeId.startsWith('lynx-')
    ? undefined
    : ({
        '--color-background': currentColors.base,
        '--color-surface': currentColors.mantle,
        '--color-sidebar': currentColors.mantle,
        '--color-subtle': currentColors.crust,
        '--color-text': currentColors.text,
        '--color-muted': currentColors.subtext0,
        '--color-brand': currentColors.accent,
        '--color-primary': currentColors.accent,
        '--color-on-primary': currentColors.accentText,
        '--color-selected': currentColors.interactiveActive,
        '--color-selection-line': currentColors.accent,
        '--color-focus': currentColors.accent,
        '--color-border': currentColors.border,
        '--color-divider': currentColors.surface1,
        '--color-control-border': currentColors.overlay1,
        '--color-connected': currentColors.success,
        '--color-error': currentColors.error,
        '--color-warning': currentColors.warning,
        '--color-chart-cpu': currentColors.accent,
        '--color-chart-memory': currentColors.subtext0,
      } as React.CSSProperties);
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
    settingsTrigger.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setSettingsVisible(true);
  }, []);
  const closeSettings = useCallback(() => {
    setSettingsVisible(false);
    requestAnimationFrame(() => settingsTrigger.current?.focus());
  }, []);
  const shared = { onOpenSettings: handleSettingsClick, onOpenRecords: openRecords };
  return (
    <div className="imager-themed" data-theme={isDark ? 'dark' : 'light'} style={alternateStyle}>
      <div
        className={activeTool === 'basic' ? 'imager-app-shell v2-basic-shell' : 'imager-app-shell'}
        inert={settingsVisible || undefined}
      >
        <header className="imager-app-header" aria-label="应用导航">
          <div className="imager-brand">
            <svg aria-hidden="true" viewBox="0 0 25 25">
              <path fill="currentColor" d="M5 3h7L8.8 16.5H22l-1.3 5.5H.5z" />
              <path fill="currentColor" opacity=".45" d="M14.4 3h5l-1.5 6.5h-5z" />
            </svg>
            <strong>LYNX Imager</strong>
            <small>v{version}</small>
          </div>
          <nav className="imager-app-tabs" aria-label="工作区">
            {(Object.keys(workspaceNames) as Workspace[]).map((key) => (
              <button
                key={key}
                type="button"
                aria-current={activeTool === key ? 'page' : undefined}
                className={activeTool === key ? 'is-active' : ''}
                onClick={() => activate(key)}
              >
                {workspaceNames[key]}
              </button>
            ))}
          </nav>
          <div className="imager-app-actions">
            <button type="button" aria-label="任务记录" onClick={openRecords}>
              <LynxIcon name="history" size={16} />
              任务记录
            </button>
            <button
              type="button"
              onClick={() =>
                void openUrl('https://github.com/dshanpi/lynx-imager').catch((e) =>
                  setHardwareBlockMessage(String(e))
                )
              }
            >
              <LynxIcon name="info" size={16} />
              使用帮助
            </button>
            <button type="button" aria-label="设置" onClick={handleSettingsClick}>
              <LynxIcon name="settings" size={16} />
              设置
            </button>
          </div>
        </header>
        <main className="imager-app-main">
          <div className="imager-screen" hidden={activeTool !== 'basic'}>
            <ImagerPage
              active={activeTool === 'basic'}
              {...shared}
              onStatusChange={basicStatus}
              onOpenAdvancedWorkbench={() => activate('advanced')}
            />
          </div>
          {visited.includes('advanced') && (
            <div className="imager-screen" hidden={activeTool !== 'advanced'}>
              <WorkbenchPage
                {...shared}
                flashTarget={flashTarget}
                onStatusChange={advancedStatus}
                isActive={activeTool === 'advanced'}
              />
            </div>
          )}
          {visited.includes('ai') && (
            <div className="imager-screen" hidden={activeTool !== 'ai'}>
              <ImagerAIWorkspace
                {...shared}
                onOpenFlash={openFlash}
                onStatusChange={aiStatus}
                isActive={activeTool === 'ai'}
              />
            </div>
          )}
        </main>
        <footer className="imager-app-status">
          <LynxIcon name="board" size={16} />
          <span>{displayedStatus?.target || workspaceNames[activeTool]}</span>
          {displayedStatus?.task && <span>{displayedStatus.task}</span>}
        </footer>
      </div>
      {recordsOpen && <TaskRecords onClose={() => setRecordsOpen(false)} />}
      {settingsVisible && (
        <Settings
          visible
          authenticated={Boolean(authUser)}
          onAuthExpired={clearExpiredAuthentication}
          onClose={closeSettings}
          onSettingsChange={() => undefined}
        />
      )}
      <Popup
        visible={Boolean(hardwareBlockMessage)}
        type="warning"
        title="工作台提示"
        message={hardwareBlockMessage}
        onClose={() => setHardwareBlockMessage('')}
      />
    </div>
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
