import { BaseDirectory, mkdir, readTextFile, writeTextFile, exists } from '@tauri-apps/plugin-fs';
import type { FlashMode, PostFlashAction } from '../Domain/flash';
import type { UsbBackend } from '../Services';
import { SupportedLanguage, getSystemLanguage } from '../i18n';
import { ThemeMode } from '../Themes';

/**
 * Application settings structure.
 *
 * Contains all user-configurable settings for the application,
 * including flash options, UI preferences, and theme settings.
 */
export interface AppSettings {
  /** Whether sidebar is collapsed */
  sidebarCollapsed: boolean;
  /** Default flash operation mode */
  defaultFlashMode: FlashMode;
  /** Whether to automatically scan for devices */
  autoScanDevices: boolean;
  /** Whether to auto-flash when device connects */
  autoFlashOnConnect: boolean;
  /** Whether to verify downloads after flashing */
  verifyDownload: boolean;
  /** Action to perform after flash completes */
  postFlashAction: PostFlashAction;
  /** Whether to remember last used image path */
  rememberLastImage: boolean;
  /** Path to last used firmware image */
  lastImagePath: string | null;
  /** USB backend driver preference */
  usbBackend: UsbBackend;
  /** Application language preference */
  language: SupportedLanguage;
  /** Theme mode preference */
  themeMode: ThemeMode;
  /** Dark theme identifier */
  themeIdDark: string;
  /** Light theme identifier */
  themeIdLight: string;
  /** Version of one-time theme preference migrations */
  themeStyleMigrationVersion: number;
  /** Local loopback port for the MCP hardware gateway */
  aiGatewayPort: number;
  /** Remote development host reached through SSH */
  aiRemoteHost: string;
  /** SSH port of the remote development host */
  aiRemoteSshPort: number;
  /** Port exposed on the remote host by reverse forwarding */
  aiRemoteForwardPort: number;
  /** SSH user used by the generated tunnel command */
  aiSshUser: string;
  /** Agent for which connection instructions are generated */
  aiAgent: 'codex' | 'claude';
}

/** Check if running on Windows */
const isWindows = navigator.userAgent?.toLowerCase().includes('windows');

/** Default USB backend based on platform */
const DEFAULT_USB_BACKEND: UsbBackend = isWindows ? 'winusb' : 'libusb';

/**
 * Gets default settings values.
 *
 * Returns settings with sensible defaults including:
 * - Platform-appropriate USB backend
 * - System language detection
 * - Keep_data flash mode (preserves user data)
 *
 * @returns Default AppSettings structure
 */
function getDefaultSettings(): AppSettings {
  return {
    sidebarCollapsed: false,
    defaultFlashMode: 'keep_data',
    autoScanDevices: true,
    autoFlashOnConnect: false,
    verifyDownload: true,
    postFlashAction: 'reboot',
    rememberLastImage: false,
    lastImagePath: null,
    usbBackend: DEFAULT_USB_BACKEND,
    language: getSystemLanguage(),
    themeMode: 'light',
    themeIdDark: 'lynx-dark',
    themeIdLight: 'lynx-light',
    themeStyleMigrationVersion: 1,
    aiGatewayPort: 8765,
    aiRemoteHost: '',
    aiRemoteSshPort: 22,
    aiRemoteForwardPort: 8765,
    aiSshUser: '',
    aiAgent: 'codex',
  };
}

/** Settings directory name in home folder */
const SETTINGS_DIR = '.lynx';

/** Settings file name */
const SETTINGS_FILE = 'settings.json';

/** Queue for serializing save operations */
let saveQueue: Promise<void> = Promise.resolve();

/** Settings pending delayed save */
let pendingSettings: AppSettings | null = null;

/** Timer for delayed save debounce */
let saveTimer: ReturnType<typeof setTimeout> | null = null;

/** Delay before saving settings (ms) */
const SAVE_DELAY_MS = 1000;

/**
 * Ensures settings directory exists.
 *
 * Creates .lynx directory in user home if it doesn't exist.
 */
async function ensureSettingsDir(): Promise<void> {
  try {
    const dirExists = await exists(SETTINGS_DIR, { baseDir: BaseDirectory.Home });
    if (!dirExists) {
      await mkdir(SETTINGS_DIR, {
        baseDir: BaseDirectory.Home,
        recursive: true,
      });
    }
  } catch (error) {
    console.error('Failed to create settings directory:', error);
  }
}

/**
 * Migrates settings from older versions.
 *
 * Handles legacy theme naming and converts old settings
 * format to current structure. Ensures compatibility when
 * updating from older application versions.
 *
 * @param parsed - Raw settings object from file
 * @returns Migrated AppSettings structure
 */
function migrateSettings(parsed: Record<string, unknown>): AppSettings {
  // v0.3 moves the legacy Python production-test AI connection into the
  // supervisor's OS keyring-backed provider store. Discard the old plaintext
  // object instead of carrying an inaccessible secret forward.
  delete parsed.ai;
  if (parsed.themeId && !parsed.themeIdDark && !parsed.themeIdLight) {
    const themeId = parsed.themeId as string;
    if (themeId === 'catppuccin-dark') {
      parsed.themeIdDark = 'catppuccin-mocha';
    } else if (themeId === 'catppuccin-light') {
      parsed.themeIdLight = 'catppuccin-latte';
    } else if (themeId.endsWith('-dark')) {
      parsed.themeIdDark = themeId;
    } else if (themeId.endsWith('-light')) {
      parsed.themeIdLight = themeId;
    }
    delete parsed.themeId;
  }
  if (parsed.themeIdDark === 'catppuccin-dark') {
    parsed.themeIdDark = 'catppuccin-mocha';
  }
  const themeStyleMigrationVersion =
    typeof parsed.themeStyleMigrationVersion === 'number' ? parsed.themeStyleMigrationVersion : 0;
  if (themeStyleMigrationVersion < 1 && parsed.themeIdDark === 'openix-dark') {
    parsed.themeIdDark = 'lynx-dark';
  }
  if (parsed.themeIdLight === 'catppuccin-light') {
    parsed.themeIdLight = 'catppuccin-latte';
  }
  if (parsed.themeIdLight === 'openix-light') {
    parsed.themeIdLight = 'lynx-light';
  }
  parsed.themeStyleMigrationVersion = 1;
  const defaults = getDefaultSettings();
  const merged: AppSettings = { ...defaults, ...parsed } as AppSettings;
  return merged;
}

/**
 * Loads settings from file or creates defaults.
 *
 * Reads settings from .lynx/settings.json in home directory,
 * applying migrations if needed. Creates default settings file
 * if none exists.
 *
 * Also syncs theme settings to localStorage for immediate access.
 *
 * @returns Promise resolving to AppSettings
 */
export async function loadSettings(): Promise<AppSettings> {
  try {
    const fileExists = await exists(`${SETTINGS_DIR}/${SETTINGS_FILE}`, {
      baseDir: BaseDirectory.Home,
    });

    if (!fileExists) {
      const defaultSettings = getDefaultSettings();
      await ensureSettingsDir();
      await writeTextFile(
        `${SETTINGS_DIR}/${SETTINGS_FILE}`,
        JSON.stringify(defaultSettings, null, 2),
        { baseDir: BaseDirectory.Home }
      );
      localStorage.setItem('app-theme', defaultSettings.themeMode);
      localStorage.setItem('app-theme-id-dark', defaultSettings.themeIdDark);
      localStorage.setItem('app-theme-id-light', defaultSettings.themeIdLight);
      localStorage.setItem('i18nextLng', defaultSettings.language);
      return { ...defaultSettings };
    }

    const content = await readTextFile(`${SETTINGS_DIR}/${SETTINGS_FILE}`, {
      baseDir: BaseDirectory.Home,
    });

    const parsed = JSON.parse(content) as Record<string, unknown>;
    const hadLegacyAiSecret = Object.prototype.hasOwnProperty.call(parsed, 'ai');
    const hadLegacyThemeSchema = parsed.themeStyleMigrationVersion !== 1;
    const hadLegacyOpenixTheme =
      parsed.themeId === 'openix-dark' ||
      parsed.themeIdDark === 'openix-dark' ||
      parsed.themeId === 'openix-light' ||
      parsed.themeIdLight === 'openix-light';
    const settings = migrateSettings(parsed);

    // v0.3 credentials live exclusively in the OS keyring. Rewrite the file
    // immediately so a legacy plaintext API key does not remain on disk until
    // the user happens to save another setting.
    if (hadLegacyAiSecret || hadLegacyOpenixTheme || hadLegacyThemeSchema) {
      await writeTextFile(`${SETTINGS_DIR}/${SETTINGS_FILE}`, JSON.stringify(settings, null, 2), {
        baseDir: BaseDirectory.Home,
      });
    }

    localStorage.setItem('app-theme', settings.themeMode);
    localStorage.setItem('app-theme-id-dark', settings.themeIdDark);
    localStorage.setItem('app-theme-id-light', settings.themeIdLight);
    return settings;
  } catch (error) {
    console.error(`Failed to load settings file ${SETTINGS_FILE}, using default settings:`, error);
    return { ...getDefaultSettings() };
  }
}

/**
 * Saves settings to file with debounce.
 *
 * Writes settings to .lynx/settings.json after a delay
 * to prevent excessive writes during rapid changes. Uses a
 * queue to serialize save operations.
 *
 * Also updates localStorage theme settings immediately.
 *
 * @param settings - AppSettings to save
 */
export async function saveSettings(settings: AppSettings): Promise<void> {
  localStorage.setItem('app-theme', settings.themeMode);
  localStorage.setItem('app-theme-id-dark', settings.themeIdDark);
  localStorage.setItem('app-theme-id-light', settings.themeIdLight);

  pendingSettings = settings;

  if (saveTimer) {
    clearTimeout(saveTimer);
  }

  saveTimer = setTimeout(() => {
    const settingsToSave = pendingSettings;
    pendingSettings = null;
    saveTimer = null;

    if (settingsToSave) {
      const settingsJson = JSON.stringify(settingsToSave, null, 2);
      saveQueue = saveQueue
        .then(async () => {
          try {
            await ensureSettingsDir();
            await writeTextFile(`${SETTINGS_DIR}/${SETTINGS_FILE}`, settingsJson, {
              baseDir: BaseDirectory.Home,
            });
          } catch (error) {
            console.error('Failed to save settings:', error);
          }
        })
        .catch((error) => {
          console.error('Save queue error:', error);
        });
    }
  }, SAVE_DELAY_MS);
}
