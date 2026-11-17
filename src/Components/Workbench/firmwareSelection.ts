import { readAllwinnerFlashOptions } from './linuxWorkbenchModel';

export interface FirmwareSelection {
  systemType: 'vendor' | 'mainline';
  paths: { vendor: string; mainline: string };
  loaderPath: string;
  rockchipLoaderPath: string;
  mainlineRoute: '' | 'mainline_nand' | 'mainline_emmc' | 'mainline_spinor';
}

export const firmwareSelectionKey = (deviceId: number) => `lynx.firmwareSelection.v1.${deviceId}`;

export function readFirmwareSelection(
  deviceId: number | null,
  flasherArgs: string
): FirmwareSelection {
  const saved = readAllwinnerFlashOptions(flasherArgs);
  const fallback: FirmwareSelection = {
    systemType: saved.route === 'vendor_image' ? 'vendor' : 'mainline',
    paths: { vendor: '', mainline: '' },
    loaderPath: saved.loaderPath,
    rockchipLoaderPath: '',
    mainlineRoute: saved.route === 'vendor_image' ? '' : saved.route,
  };
  if (deviceId == null) return fallback;
  try {
    const value: unknown = JSON.parse(
      window.localStorage.getItem(firmwareSelectionKey(deviceId)) || 'null'
    );
    if (!value || typeof value !== 'object') return fallback;
    const entry = value as Partial<FirmwareSelection>;
    return {
      systemType:
        entry.systemType === 'vendor' || entry.systemType === 'mainline'
          ? entry.systemType
          : fallback.systemType,
      paths: {
        vendor: typeof entry.paths?.vendor === 'string' ? entry.paths.vendor : '',
        mainline: typeof entry.paths?.mainline === 'string' ? entry.paths.mainline : '',
      },
      rockchipLoaderPath:
        typeof entry.rockchipLoaderPath === 'string' ? entry.rockchipLoaderPath : '',
      loaderPath: typeof entry.loaderPath === 'string' ? entry.loaderPath : fallback.loaderPath,
      mainlineRoute:
        entry.mainlineRoute === 'mainline_nand' ||
        entry.mainlineRoute === 'mainline_emmc' ||
        entry.mainlineRoute === 'mainline_spinor'
          ? entry.mainlineRoute
          : fallback.mainlineRoute,
    };
  } catch {
    return fallback;
  }
}
