import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, type WorkbenchDevice } from '../../Platform/IPC';
import type {
  FirmwareCandidate,
  FirmwareCatalogVersion,
} from '../../Platform/IPC/contracts/Workbench';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';
import { formatErrorMessage } from '../../Utils';

export default function BasicCatalog({
  device,
  onChoose,
}: {
  device: WorkbenchDevice;
  onChoose: (candidate: FirmwareCandidate) => void;
}) {
  const [entries, setEntries] = useState<FirmwareCatalogVersion[]>([]);
  const [reading, setReading] = useState(false);
  const [inspected, setInspected] = useState(false);
  const [error, setError] = useState('');
  const active = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);

  async function readCatalog() {
    if (!device.id || reading) return;
    setReading(true);
    setError('');
    setEntries([]);
    setInspected(false);
    try {
      const catalog = await invokeCommand('workbench_firmware_catalog', {
        deviceId: device.id,
        channel: 'stable',
      });
      if (!active.current) return;
      if (device.id !== catalog.deviceId || device.boardUid !== catalog.boardUid)
        throw new Error('目标设备已变化，请重新读取。');
      setEntries(
        catalog.entries.filter(
          (v) =>
            v.entry.manifest.applicationId === 'nekobox' &&
            v.entry.manifest.boardModel.toLowerCase() === 'yuzukineko' &&
            v.entry.status === 'active'
        )
      );
      setInspected(true);
    } catch (e) {
      if (active.current) setError(formatErrorMessage(e));
    } finally {
      if (active.current) setReading(false);
    }
  }

  async function obtain(v: FirmwareCatalogVersion) {
    if (!device.id || reading) return;
    setReading(true);
    setError('');
    try {
      const loader = await open({
        title: '选择这块 F101 对应的 RAM FES Loader',
        multiple: false,
        filters: [{ name: 'RAM FES Loader', extensions: ['fex', 'bin'] }],
      });
      if (typeof loader !== 'string' || !active.current) return;
      const c = await invokeCommand('workbench_firmware_download', {
        deviceId: device.id,
        request: {
          releaseId: v.releaseId,
          applicationId: v.entry.manifest.applicationId,
          version: v.entry.manifest.version,
          channel: 'stable',
          loaderPath: loader,
        },
      });
      if (!active.current) return;
      if (c.deviceId !== device.id || c.boardUid !== device.boardUid)
        throw new Error('候选包的设备作用域已改变。');
      onChoose(c);
    } catch (e) {
      if (active.current) setError(formatErrorMessage(e));
    } finally {
      if (active.current) setReading(false);
    }
  }
  return (
    <section>
      <h3>F101 正式发布目录</h3>
      <p>为 {device.name} 读取 NekoBox 的已发布版本。下载时需要选择该板的 RAM FES Loader。</p>
      <button type="button" disabled={reading} onClick={() => void readCatalog()}>
        {reading ? '正在处理…' : '读取发布目录'}
      </button>
      {inspected && !entries.length && <p role="status">没有可用的已发布版本。可改用本地镜像。</p>}
      {entries.length > 0 && (
        <table className="basic-table">
          <thead>
            <tr>
              <th>固件文件</th>
              <th>版本</th>
              <th>大小</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {entries.map((v) => (
              <tr key={v.releaseId + '-' + v.entry.manifest.version}>
                <td>{v.entry.manifest.image.fileName}</td>
                <td>{v.entry.manifest.version}</td>
                <td>{formatLinuxBytes(v.entry.manifest.image.sizeBytes)}</td>
                <td>
                  <button type="button" disabled={reading} onClick={() => void obtain(v)}>
                    获取并核对
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {error && (
        <p role="alert" className="basic-warning">
          {error}
        </p>
      )}
    </section>
  );
}
