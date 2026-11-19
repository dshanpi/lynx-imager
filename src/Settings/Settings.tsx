import React, { useState, useEffect, useMemo, useRef } from 'react';
import { handleDialogKey } from '../CoreUI/dialogKeys';
import { useTranslation } from 'react-i18next';
import { motion, AnimatePresence } from 'framer-motion';
import { AppSettings, loadSettings, saveSettings } from './settingsStore';
// 烧录设置区块已隐藏（legacy Allwinner 烧录相关），以下导入暂未使用；恢复时取消注释
// import { POST_FLASH_ACTION_OPTIONS, type PostFlashAction, type FlashMode } from '../Domain/flash';
// import { FLASH_MODE_LABELS } from '../FlashManager/Types';
import { efexService, type UsbBackend } from '../Services';
import { supportedLanguages } from '../i18n';
import { relaunch } from '@tauri-apps/plugin-process';
import { openUrl } from '@tauri-apps/plugin-opener';
import { formatErrorMessage, formatSize } from '../Utils';
import { useAppVersion } from '../Hooks/useAppVersion';
import { useTheme, ThemeMode } from '../Themes';
import { ThemeViewer } from '../Themes/Components';
import { invokeCommand } from '../Platform/IPC';
import type {
  CloudSyncStatus,
  DiagnosticIncident,
  DiagnosticReportingStatus,
} from '../Platform/IPC/Commands';
import { LynxIcon } from '../CoreUI';
import {
  checkForUpdate,
  installIdleUpdate,
  getUpdateAssetSize,
  getUpdateStrategy,
  isUpdateVersionSkipped,
  skipUpdateVersion,
  type UpdateStrategy,
} from '../Services/AutoUpdater';
import './Settings.css';

const isWindows = navigator.userAgent?.toLowerCase().includes('windows');

const THEME_MODE_OPTIONS: { value: ThemeMode; labelKey: string }[] = [
  { value: 'light', labelKey: 'themes.light' },
  { value: 'dark', labelKey: 'themes.dark' },
  { value: 'system', labelKey: 'themes.system' },
];

interface SettingsProps {
  visible: boolean;
  authenticated: boolean;
  onAuthExpired: () => void;
  onClose: () => void;
  onSettingsChange: (settings: AppSettings) => void;
}

interface OriginalUIState {
  language: string;
  themeMode: ThemeMode;
  themeIdDark: string;
  themeIdLight: string;
  usbBackend: UsbBackend;
}

export const Settings: React.FC<SettingsProps> = ({
  visible,
  authenticated,
  onAuthExpired,
  onClose,
  onSettingsChange,
}) => {
  const { t, i18n } = useTranslation();
  const { setThemeMode, setThemeId, availableThemes, currentThemeId, effectiveVariant } =
    useTheme();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [originalUIState, setOriginalUIState] = useState<OriginalUIState | null>(null);
  const [saving, setSaving] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<{
    available: boolean;
    version?: string;
    notes?: string;
    strategy?: UpdateStrategy;
    skipped?: boolean;
    downloading?: boolean;
    progress?: number;
    downloaded?: number;
    contentLength?: number;
    error?: string;
  } | null>(null);
  const [themeViewerVisible, setThemeViewerVisible] = useState(false);
  const paletteButton = useRef<HTMLButtonElement>(null);
  const [cloudSync, setCloudSync] = useState<CloudSyncStatus | null>(null);
  const [savedCloudSyncEnabled, setSavedCloudSyncEnabled] = useState(false);
  const [syncingCloud, setSyncingCloud] = useState(false);
  const [diagnostics, setDiagnostics] = useState<DiagnosticReportingStatus | null>(null);
  const [savedDiagnosticsEnabled, setSavedDiagnosticsEnabled] = useState(false);
  const [diagnosticIncidents, setDiagnosticIncidents] = useState<DiagnosticIncident[]>([]);
  const [syncingDiagnostics, setSyncingDiagnostics] = useState(false);
  const [clearingDiagnostics, setClearingDiagnostics] = useState(false);
  const [refreshingResolutions, setRefreshingResolutions] = useState(false);
  const [diagnosticActionMessage, setDiagnosticActionMessage] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveSuccess, setSaveSuccess] = useState<string | null>(null);
  const appVersion = useAppVersion();
  // 烧录设置区块已隐藏，此辅助函数暂未使用；恢复时取消注释
  // const getFlashModeLabel = (mode: FlashMode): string => {
  //   return t(FLASH_MODE_LABELS[mode]);
  // };

  const filteredThemes = useMemo(
    () => availableThemes.filter((theme) => theme.variant === effectiveVariant),
    [availableThemes, effectiveVariant]
  );

  const validThemeId = useMemo(() => {
    const isValid = filteredThemes.some((t) => t.id === currentThemeId);
    return isValid ? currentThemeId : filteredThemes[0]?.id || currentThemeId;
  }, [filteredThemes, currentThemeId]);

  useEffect(() => {
    if (visible) {
      loadSettings().then((loadedSettings) => {
        setSettings(loadedSettings);
        setOriginalUIState({
          language: loadedSettings.language,
          themeMode: loadedSettings.themeMode,
          themeIdDark: loadedSettings.themeIdDark,
          themeIdLight: loadedSettings.themeIdLight,
          usbBackend: loadedSettings.usbBackend,
        });
      });
      invokeCommand('cloud_sync_get_status')
        .then((status) => {
          setCloudSync(status);
          setSavedCloudSyncEnabled(status.enabled);
        })
        .catch(() => setCloudSync(null));
      invokeCommand('diagnostics_get_status')
        .then((status) => {
          setDiagnostics(status);
          setSavedDiagnosticsEnabled(status.enabled);
        })
        .catch(() => setDiagnostics(null));
      invokeCommand('diagnostics_list_incidents', { includeDismissed: false })
        .then(setDiagnosticIncidents)
        .catch(() => setDiagnosticIncidents([]));
      setUpdateInfo(null);
      setSaveError(null);
      setSaveSuccess(null);
    }
  }, [visible]);

  const handleClose = () => {
    if (saving) return;
    if (originalUIState) {
      i18n.changeLanguage(originalUIState.language);
      setThemeMode(originalUIState.themeMode);
      setThemeId(originalUIState.themeIdDark, 'dark');
      setThemeId(originalUIState.themeIdLight, 'light');
    }
    onClose();
  };

  const handleChange = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => {
    if (settings) {
      setSettings({ ...settings, [key]: value });
    }
  };

  const handleLanguageChange = (lang: string) => {
    i18n.changeLanguage(lang);
    handleChange('language', lang as AppSettings['language']);
  };

  const handleThemeModeChange = (mode: ThemeMode) => {
    setThemeMode(mode);
    handleChange('themeMode', mode);
  };

  const handleThemeIdChange = (themeId: string) => {
    setThemeId(themeId, effectiveVariant);
    handleChange(effectiveVariant === 'dark' ? 'themeIdDark' : 'themeIdLight', themeId);
  };

  const handleSave = async () => {
    if (!settings || !originalUIState || saving) return;
    setSaving(true);
    setSaveError(null);
    setSaveSuccess(null);
    try {
      if (cloudSync && cloudSync.enabled !== savedCloudSyncEnabled) {
        const status = await invokeCommand('cloud_sync_set_enabled', {
          enabled: cloudSync.enabled,
        });
        setCloudSync(status);
        setSavedCloudSyncEnabled(status.enabled);
      }
      let reporting = diagnostics;
      if (diagnostics && diagnostics.enabled !== savedDiagnosticsEnabled) {
        const status = await invokeCommand('diagnostics_set_enabled', {
          enabled: diagnostics.enabled,
        });
        setDiagnostics(status);
        setSavedDiagnosticsEnabled(status.enabled);
        reporting = status;
      }
      const backendChanged = settings.usbBackend !== originalUIState.usbBackend;
      if (backendChanged) await efexService.setUsbBackend(settings.usbBackend);
      try {
        await saveSettings(settings);
      } catch (error) {
        if (backendChanged) await efexService.setUsbBackend(originalUIState.usbBackend);
        throw error;
      }
      onSettingsChange(settings);
      setOriginalUIState({
        language: settings.language,
        themeMode: settings.themeMode,
        themeIdDark: settings.themeIdDark,
        themeIdLight: settings.themeIdLight,
        usbBackend: settings.usbBackend,
      });
      setSaveSuccess(
        reporting
          ? t('settings.saveSucceeded', {
              state: reporting.enabled
                ? t('settings.diagnostics.enabledState')
                : t('settings.diagnostics.disabledState'),
              count: reporting.pendingCount,
            })
          : t('settings.savedPreferences')
      );
    } catch (error) {
      console.error('Failed to save settings:', error);
      const message = formatErrorMessage(error);
      if (/AUTH_TOKEN_EXPIRED|AUTH_REQUIRED|LINK_AUTH_REQUIRED/.test(message)) {
        onAuthExpired();
      }
      try {
        await refreshDiagnostics();
      } catch {
        // Keep the last known state when even the local status command is unavailable.
      }
      setSaveError(t('settings.saveFailed', { error: message }));
    } finally {
      setSaving(false);
    }
  };

  const handleCloudSyncNow = async () => {
    setSyncingCloud(true);
    try {
      const status = await invokeCommand('cloud_sync_sync_now');
      setCloudSync(status);
    } catch {
      try {
        setCloudSync(await invokeCommand('cloud_sync_get_status'));
      } catch {
        // Keep the existing status when the local backend itself is unavailable.
      }
    } finally {
      setSyncingCloud(false);
    }
  };

  const cloudSyncStateText = (status: CloudSyncStatus) => {
    if (status.lastErrorCode) {
      return t(`settings.cloudSync.errors.${status.lastErrorCode}`, status.lastErrorCode);
    }
    return t(`settings.cloudSync.states.${status.state}`);
  };

  const refreshDiagnostics = async () => {
    const [status, incidents] = await Promise.all([
      invokeCommand('diagnostics_get_status'),
      invokeCommand('diagnostics_list_incidents', { includeDismissed: false }),
    ]);
    setDiagnostics(status);
    setDiagnosticIncidents(incidents);
    setSavedDiagnosticsEnabled(status.enabled);
    return status;
  };

  const handleDiagnosticsSyncNow = async () => {
    setSyncingDiagnostics(true);
    try {
      await invokeCommand('diagnostics_sync_now');
    } catch {
      // The status below exposes auth, network and server errors without
      // reflecting potentially sensitive response bodies.
    } finally {
      try {
        await refreshDiagnostics();
      } catch {
        // Preserve the last locally known state if the backend is unavailable.
      }
      setSyncingDiagnostics(false);
    }
  };

  const handleDiagnosticsClear = async () => {
    if (!window.confirm(t('settings.diagnostics.confirmClear'))) return;
    setClearingDiagnostics(true);
    try {
      const status = await invokeCommand('diagnostics_clear_pending');
      setDiagnostics(status);
      await refreshDiagnostics();
    } finally {
      setClearingDiagnostics(false);
    }
  };

  const handleRefreshResolutions = async () => {
    setRefreshingResolutions(true);
    setDiagnosticActionMessage(null);
    try {
      setDiagnosticIncidents(await invokeCommand('diagnostics_refresh_resolutions'));
      setDiagnosticActionMessage('已检查官网公开解决方案');
    } catch (error) {
      setDiagnosticActionMessage(`解决方案检查失败：${formatErrorMessage(error)}`);
    } finally {
      setRefreshingResolutions(false);
    }
  };

  const handleDismissIncident = async (fingerprint: string) => {
    await invokeCommand('diagnostics_dismiss_incident', { fingerprint, dismissed: true });
    setDiagnosticIncidents((current) =>
      current.filter((incident) => incident.fingerprint !== fingerprint)
    );
  };

  const handleIncidentAiHandoff = async (incident: DiagnosticIncident) => {
    const prompt = await invokeCommand('diagnostics_build_ai_handoff', {
      fingerprint: incident.fingerprint,
    });
    window.dispatchEvent(
      new CustomEvent('lynx-diagnostic-ai-handoff', {
        detail: { prompt, recommendedSkill: incident.recommendedSkill },
      })
    );
    await navigator.clipboard.writeText(prompt);
    setDiagnosticActionMessage('故障包已交给当前工作台，并已复制到剪贴板');
  };

  const diagnosticStateText = (status: DiagnosticReportingStatus) => {
    if (status.lastErrorCode) {
      return t(`settings.diagnostics.errors.${status.lastErrorCode}`, status.lastErrorCode);
    }
    return t(`settings.diagnostics.states.${status.state}`);
  };

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateInfo(null);
    try {
      const update = await checkForUpdate();
      if (update) {
        setUpdateInfo({
          available: true,
          version: update.version,
          skipped: isUpdateVersionSkipped(update.version),
          notes: update.body || undefined,
          strategy: getUpdateStrategy(update),
          contentLength: getUpdateAssetSize(update),
        });
      } else {
        setUpdateInfo({
          available: false,
        });
      }
    } catch (error) {
      console.error('Failed to check for updates:', error);
      setUpdateInfo({
        available: false,
        error: formatErrorMessage(error),
      });
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleSkipUpdate = () => {
    if (!updateInfo?.version) return;
    skipUpdateVersion(updateInfo.version);
    setUpdateInfo((current) => (current ? { ...current, skipped: true } : current));
  };

  const handleDownloadUpdate = async () => {
    if (!updateInfo?.available) return;

    const download = async (forceFull: boolean): Promise<UpdateStrategy> => {
      setUpdateInfo((prev) =>
        prev
          ? {
              ...prev,
              downloading: true,
              progress: 0,
              downloaded: 0,
              contentLength: 0,
              error: undefined,
            }
          : null
      );
      const update = await checkForUpdate(forceFull);
      if (!update) throw new Error('更新包已不可用，请重新检查更新。');
      const strategy = getUpdateStrategy(update);
      if (forceFull && strategy !== 'full') {
        throw new Error('更新服务未返回强制完整包，已停止安装。');
      }
      setUpdateInfo((prev) => (prev ? { ...prev, strategy } : null));
      let downloaded = 0;
      let contentLength = 0;
      await installIdleUpdate(update, (event) => {
        switch (event.event) {
          case 'Started': {
            contentLength = event.data.contentLength || 0;
            setUpdateInfo((prev) =>
              prev ? { ...prev, contentLength, downloaded: 0, progress: 0 } : null
            );
            break;
          }
          case 'Progress': {
            downloaded += event.data.chunkLength;
            const progress = contentLength > 0 ? Math.round((downloaded / contentLength) * 100) : 0;
            setUpdateInfo((prev) => (prev ? { ...prev, downloaded, progress } : null));
            break;
          }
          case 'Finished':
            setUpdateInfo((prev) =>
              prev
                ? { ...prev, progress: 100, downloaded: contentLength, downloading: false }
                : null
            );
            break;
        }
      });
      await relaunch();
      return strategy;
    };

    let attemptedStrategy = updateInfo.strategy;
    try {
      attemptedStrategy = await download(false);
    } catch (error) {
      console.error('Failed to download update:', error);
      if (
        attemptedStrategy === 'delta' &&
        window.confirm('小版本更新下载或签名校验失败。是否立即改用签名完整安装包？')
      ) {
        try {
          await download(true);
          return;
        } catch (fallbackError) {
          console.error('Failed to download the full fallback update:', fallbackError);
          setUpdateInfo((prev) =>
            prev
              ? {
                  ...prev,
                  downloading: false,
                  error: `完整包回退失败：${formatErrorMessage(fallbackError)}`,
                }
              : null
          );
          return;
        }
      }
      setUpdateInfo((prev) =>
        prev ? { ...prev, downloading: false, error: formatErrorMessage(error) } : null
      );
    }
  };

  if (!settings) return null;

  const overlayVariants = {
    initial: { opacity: 0 },
    animate: { opacity: 1 },
    exit: { opacity: 0 },
  };

  const panelVariants = {
    initial: { opacity: 0, scale: 0.95, y: 10 },
    animate: { opacity: 1, scale: 1, y: 0 },
    exit: { opacity: 0, scale: 0.95, y: 10 },
  };

  const transition = { duration: 0.15 };

  return (
    <>
      <AnimatePresence>
        {visible && (
          <motion.div
            data-testid="settings-overlay"
            className="settings-overlay"
            onClick={handleClose}
            variants={overlayVariants}
            initial="initial"
            animate="animate"
            exit="exit"
            transition={transition}
          >
            <motion.div
              data-testid="settings-panel"
              className="settings-panel"
              role="dialog"
              aria-modal="true"
              aria-labelledby="imager-settings-title"
              inert={themeViewerVisible || undefined}
              onKeyDown={(event) => handleDialogKey(event, handleClose)}
              onClick={(e) => e.stopPropagation()}
              variants={panelVariants}
              transition={transition}
            >
              <div className="settings-header">
                <h2 id="imager-settings-title">{t('settings.title')}</h2>
                <motion.button
                  data-testid="settings-close"
                  disabled={saving}
                  autoFocus
                  className="settings-close"
                  onClick={handleClose}
                  aria-label={t('settings.close')}
                  whileHover={{ scale: 1.05 }}
                  whileTap={{ scale: 0.95 }}
                  transition={{ duration: 0.1 }}
                >
                  <LynxIcon name="dismiss" size={16} />
                </motion.button>
              </div>

              <div className="settings-content" inert={saving || undefined} aria-busy={saving}>
                <div className="settings-section">
                  <h3>{t('settings.uiSettings')}</h3>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.language')}</span>
                    <select
                      data-testid="settings-language"
                      value={settings.language}
                      onChange={(e) => handleLanguageChange(e.target.value)}
                    >
                      {supportedLanguages.map((lang) => (
                        <option key={lang.code} value={lang.code}>
                          {lang.nativeName}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.themeMode')}</span>
                    <select
                      data-testid="settings-theme-mode"
                      value={settings.themeMode}
                      onChange={(e) => handleThemeModeChange(e.target.value as ThemeMode)}
                    >
                      {THEME_MODE_OPTIONS.map((option) => (
                        <option key={option.value} value={option.value}>
                          {t(option.labelKey)}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.themeStyle')}</span>
                    <select
                      data-testid="settings-theme-style"
                      value={validThemeId}
                      onChange={(e) => handleThemeIdChange(e.target.value)}
                    >
                      {filteredThemes.map((theme) => (
                        <option key={theme.id} value={theme.id}>
                          {theme.name} - {theme.variantName}
                        </option>
                      ))}
                    </select>
                  </label>
                  <div className="settings-item">
                    <span className="settings-label">
                      {t('settings.themeViewer', '主题调色板查看器')}
                    </span>
                    <button
                      ref={paletteButton}
                      className="settings-btn settings-btn-secondary"
                      onClick={() => setThemeViewerVisible(true)}
                    >
                      {t('settings.themeViewer', '主题调色板查看器')}
                    </button>
                  </div>
                </div>

                {/* 烧录设置（legacy Allwinner 烧录相关）已隐藏，需要恢复时去掉此 JSX 注释即可
                <div className="settings-section">
                  <h3>{t('settings.flashSettings')}</h3>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.defaultFlashMode')}</span>
                    <select
                      value={settings.defaultFlashMode}
                      onChange={(e) =>
                        handleChange('defaultFlashMode', e.target.value as FlashMode)
                      }
                    >
                      {(Object.keys(FLASH_MODE_LABELS) as FlashMode[]).map((mode) => (
                        <option key={mode} value={mode}>
                          {getFlashModeLabel(mode)}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.postFlashAction')}</span>
                    <select
                      value={settings.postFlashAction}
                      onChange={(e) =>
                        handleChange('postFlashAction', e.target.value as PostFlashAction)
                      }
                    >
                      {POST_FLASH_ACTION_OPTIONS.map((option) => (
                        <option key={option.value} value={option.value}>
                          {t(option.label)}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.verifyDownload')}</span>
                    <input
                      type="checkbox"
                      checked={settings.verifyDownload}
                      onChange={(e) => handleChange('verifyDownload', e.target.checked)}
                    />
                  </label>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.rememberLastImage')}</span>
                    <input
                      type="checkbox"
                      checked={settings.rememberLastImage}
                      onChange={(e) => handleChange('rememberLastImage', e.target.checked)}
                    />
                  </label>
                </div>
                */}

                <div className="settings-section">
                  <h3>{t('settings.deviceSettings')}</h3>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.autoScanDevices')}</span>
                    <input
                      type="checkbox"
                      checked={settings.autoScanDevices}
                      onChange={(e) => handleChange('autoScanDevices', e.target.checked)}
                    />
                  </label>
                  <label className="settings-item">
                    <span className="settings-label">{t('settings.usbBackend')}</span>
                    <select
                      aria-label={t('settings.usbBackend')}
                      value={settings.usbBackend}
                      onChange={(e) => handleChange('usbBackend', e.target.value as UsbBackend)}
                    >
                      {isWindows && <option value="winusb">{t('usbBackend.winusb')}</option>}
                      <option value="libusb">{t('usbBackend.libusb')}</option>
                    </select>
                  </label>
                </div>

                <div className="settings-section">
                  <h3>{t('settings.cloudSync.title')}</h3>
                  <label className="settings-item">
                    <span className="settings-label settings-cloud-sync-label">
                      {t('settings.cloudSync.enabled')}
                      <small>{t('settings.cloudSync.hint')}</small>
                    </span>
                    <input
                      data-testid="settings-cloud-sync"
                      type="checkbox"
                      checked={cloudSync?.enabled || false}
                      disabled={!cloudSync}
                      onChange={(event) =>
                        setCloudSync((current) =>
                          current ? { ...current, enabled: event.target.checked } : current
                        )
                      }
                    />
                  </label>
                  {cloudSync && (
                    <div className="settings-item settings-about-item">
                      <span className="settings-label">
                        {t('settings.cloudSync.status')}: {cloudSyncStateText(cloudSync)}
                      </span>
                      <button
                        className="settings-btn settings-btn-secondary"
                        disabled={
                          !cloudSync.enabled ||
                          cloudSync.enabled !== savedCloudSyncEnabled ||
                          syncingCloud
                        }
                        onClick={handleCloudSyncNow}
                      >
                        {syncingCloud
                          ? t('settings.cloudSync.syncing')
                          : t('settings.cloudSync.syncNow')}
                      </button>
                    </div>
                  )}
                </div>

                <div className="settings-section">
                  <h3>{t('settings.diagnostics.title')}</h3>
                  <label className="settings-item">
                    <span className="settings-label settings-cloud-sync-label">
                      {t('settings.diagnostics.enabled')}
                      <small>{t('settings.diagnostics.hint')}</small>
                    </span>
                    <input
                      data-testid="settings-diagnostic-reporting"
                      type="checkbox"
                      checked={Boolean(diagnostics?.enabled && authenticated)}
                      disabled={!diagnostics || !authenticated}
                      onChange={(event) =>
                        setDiagnostics((current) =>
                          current ? { ...current, enabled: event.target.checked } : current
                        )
                      }
                    />
                  </label>
                  {!authenticated && (
                    <div
                      data-testid="settings-diagnostic-auth-hint"
                      className="settings-diagnostic-auth-hint"
                    >
                      {t('settings.diagnostics.loginRequired')}
                    </div>
                  )}
                  {diagnostics && (
                    <>
                      <div className="settings-item settings-about-item settings-diagnostic-status">
                        <span className="settings-label">
                          {t('settings.diagnostics.status')}: {diagnosticStateText(diagnostics)}
                          <small>
                            {t('settings.diagnostics.pending', {
                              count: diagnostics.pendingCount,
                            })}
                          </small>
                        </span>
                        <div className="settings-inline-actions">
                          <button
                            className="settings-btn settings-btn-secondary"
                            disabled={!authenticated || refreshingResolutions}
                            onClick={handleRefreshResolutions}
                          >
                            {refreshingResolutions ? '检查中…' : '检查解决方案'}
                          </button>
                          <button
                            className="settings-btn settings-btn-secondary"
                            disabled={
                              !diagnostics.enabled ||
                              !authenticated ||
                              diagnostics.enabled !== savedDiagnosticsEnabled ||
                              diagnostics.pendingCount === 0 ||
                              syncingDiagnostics
                            }
                            onClick={handleDiagnosticsSyncNow}
                          >
                            {syncingDiagnostics
                              ? t('settings.diagnostics.syncing')
                              : t('settings.diagnostics.syncNow')}
                          </button>
                          <button
                            className="settings-btn settings-btn-secondary"
                            disabled={diagnostics.pendingCount === 0 || clearingDiagnostics}
                            onClick={handleDiagnosticsClear}
                          >
                            {t('settings.diagnostics.clear')}
                          </button>
                        </div>
                      </div>
                      {diagnosticActionMessage && (
                        <div className="settings-diagnostic-action-message">
                          {diagnosticActionMessage}
                        </div>
                      )}
                      {diagnosticIncidents.length > 0 && (
                        <details className="settings-diagnostic-preview" open>
                          <summary>问题中心（{diagnosticIncidents.length}）</summary>
                          {diagnosticIncidents.slice(0, 20).map((incident) => (
                            <div className="settings-diagnostic-report" key={incident.fingerprint}>
                              <div className="settings-diagnostic-report-title">
                                <span>{incident.errorCode}</span>
                                <span className={`diagnostic-upload-state ${incident.uploadState}`}>
                                  {incident.uploadState === 'uploaded' ? '已上报' : '待上报'}
                                </span>
                              </div>
                              <small>
                                {incident.component} · {incident.failureClass} · ×
                                {incident.occurrenceCount}
                              </small>
                              <small>
                                建议：{incident.resolutionKey}
                                {incident.retryable ? ' · 可由用户重试' : ' · 不自动重试'}
                              </small>
                              {incident.recommendedSkill && (
                                <small>Skill：{incident.recommendedSkill}</small>
                              )}
                              {incident.publicWorkaround && (
                                <small className="diagnostic-public-workaround">
                                  官网方案：{incident.publicWorkaround}
                                </small>
                              )}
                              {(incident.targetVersion || incident.releasedVersion) && (
                                <small>
                                  目标版本：{incident.targetVersion || '—'} · 已发布：
                                  {incident.releasedVersion || '—'}
                                </small>
                              )}
                              <div className="settings-inline-actions diagnostic-item-actions">
                                <button
                                  className="settings-btn settings-btn-secondary"
                                  onClick={() => handleIncidentAiHandoff(incident)}
                                >
                                  交给 AI 处理
                                </button>
                                <button
                                  className="settings-btn settings-btn-secondary"
                                  onClick={() => handleDismissIncident(incident.fingerprint)}
                                >
                                  忽略
                                </button>
                              </div>
                            </div>
                          ))}
                        </details>
                      )}
                    </>
                  )}
                </div>

                <div className="settings-section">
                  <h3>{t('settings.update.title')}</h3>
                  <div className="settings-item settings-update-item">
                    <span className="settings-label">
                      {t('settings.currentVersion')}: v{appVersion}
                    </span>
                    <button
                      className="settings-btn settings-btn-secondary"
                      onClick={handleCheckUpdate}
                      disabled={checkingUpdate}
                    >
                      {checkingUpdate ? t('settings.update.checking') : t('settings.update.check')}
                    </button>
                  </div>
                  {updateInfo && (
                    <div className="settings-update-info">
                      {updateInfo.error ? (
                        <div className="settings-update-error">
                          <strong>更新检查失败</strong>
                          <span>{updateInfo.error}</span>
                        </div>
                      ) : updateInfo.available ? (
                        <>
                          <div className="settings-update-available">
                            <span>
                              {t('settings.update.available', { version: updateInfo.version })}
                            </span>
                            <small>
                              {t(
                                updateInfo.strategy === 'delta'
                                  ? 'settings.update.strategyDelta'
                                  : 'settings.update.strategyFull'
                              )}
                            </small>
                          </div>
                          <div className="settings-update-notes">
                            <strong>{t('settings.update.notes')}</strong>
                            <pre>{updateInfo.notes || t('settings.update.noNotes')}</pre>
                          </div>
                          {updateInfo.skipped && (
                            <div className="settings-update-latest">
                              <span>{t('settings.update.skipped')}</span>
                            </div>
                          )}
                          {updateInfo.downloading && (
                            <div className="settings-update-progress">
                              <div className="settings-progress-bar">
                                <div
                                  className="settings-progress-fill"
                                  style={{ width: `${updateInfo.progress || 0}%` }}
                                />
                              </div>
                              <div className="settings-progress-text">
                                <span>{updateInfo.progress}%</span>
                                {updateInfo.contentLength && updateInfo.contentLength > 0 && (
                                  <span>
                                    {formatSize(updateInfo.downloaded || 0)} /{' '}
                                    {formatSize(updateInfo.contentLength)}
                                  </span>
                                )}
                              </div>
                            </div>
                          )}
                          <div className="settings-update-actions">
                            <button
                              className="settings-btn settings-btn-primary settings-update-btn"
                              onClick={handleDownloadUpdate}
                              disabled={updateInfo.downloading}
                            >
                              {updateInfo.downloading
                                ? t('settings.update.downloading')
                                : t('settings.update.download')}
                            </button>
                            <button
                              className="settings-btn settings-btn-secondary settings-update-btn"
                              onClick={handleSkipUpdate}
                              disabled={updateInfo.downloading || updateInfo.skipped}
                            >
                              {t('settings.update.skip')}
                            </button>
                          </div>
                        </>
                      ) : (
                        <div className="settings-update-latest">
                          <span>{t('settings.update.latest')}</span>
                        </div>
                      )}
                    </div>
                  )}
                </div>

                <div className="settings-section">
                  <h3>{t('settings.developer.title')}</h3>
                  <div className="settings-item">
                    <span className="settings-label">{t('settings.developer.devtools')}</span>
                    <button
                      className="settings-btn settings-btn-secondary"
                      onClick={() => invokeCommand('open_devtools')}
                    >
                      {t('settings.developer.openDevtools')}
                    </button>
                  </div>
                </div>

                <div className="settings-section">
                  <h3>{t('settings.about.title')}</h3>
                  <div className="settings-item settings-about-item">
                    <span className="settings-label">{t('settings.about.company')}</span>
                    <span className="settings-about-value">{t('settings.about.companyName')}</span>
                  </div>
                  <div className="settings-item settings-about-item">
                    <span className="settings-label">{t('settings.about.website')}</span>
                    <a
                      className="settings-about-link"
                      href="https://www.100ask.net/"
                      onClick={(e) => {
                        e.preventDefault();
                        openUrl('https://www.100ask.net/').catch((err) =>
                          console.error('Failed to open URL:', err)
                        );
                      }}
                    >
                      https://www.100ask.net/
                    </a>
                  </div>
                </div>
              </div>

              {saveError && (
                <div
                  data-testid="settings-save-error"
                  className="settings-save-message error"
                  role="alert"
                >
                  {saveError}
                </div>
              )}
              {saveSuccess && (
                <div
                  data-testid="settings-save-success"
                  className="settings-save-message success"
                  role="status"
                >
                  {saveSuccess}
                </div>
              )}
              <div className="settings-footer">
                <motion.button
                  className="settings-btn settings-btn-secondary"
                  onClick={handleClose}
                  disabled={saving}
                  whileHover={{ scale: 1.02 }}
                  whileTap={{ scale: 0.98 }}
                  transition={{ duration: 0.1 }}
                >
                  {t('settings.cancel')}
                </motion.button>
                <motion.button
                  className="settings-btn settings-btn-primary"
                  onClick={handleSave}
                  disabled={saving}
                  whileHover={{ scale: saving ? 1 : 1.02 }}
                  whileTap={{ scale: saving ? 1 : 0.98 }}
                  transition={{ duration: 0.1 }}
                >
                  {saving ? t('settings.saving') : t('settings.save')}
                </motion.button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
      <ThemeViewer
        visible={themeViewerVisible}
        onClose={() => {
          setThemeViewerVisible(false);
          requestAnimationFrame(() => paletteButton.current?.focus());
        }}
      />
    </>
  );
};

export default Settings;
