import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import packageJson from '../../package.json';

/** Return the version embedded in the running Tauri binary. */
export function useAppVersion() {
  const [version, setVersion] = useState(packageJson.version);

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => undefined);
  }, []);

  return version;
}
