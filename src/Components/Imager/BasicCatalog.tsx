import { useCallback, useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, type WorkbenchDevice } from '../../Platform/IPC';
import type {
  FirmwareCandidate,
  FirmwareCatalogVersion,
} from '../../Platform/IPC/contracts/Workbench';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';
import { LynxIcon } from '../../CoreUI';
import { formatErrorMessage } from '../../Utils';

interface Props {
  device?: WorkbenchDevice | null;
  hasCatalog?: boolean;
  selected?: FirmwareCandidate | null;
  onInspect?: (entry: FirmwareCatalogVersion | null) => void;
  onChoose: (candidate: FirmwareCandidate) => void;
  onLocal?: () => void;
}
export default function BasicCatalog({
  device,
  hasCatalog = true,
  selected,
  onChoose,
  onInspect,
  onLocal,
}: Props) {
  const [entries, setEntries] = useState<FirmwareCatalogVersion[]>([]);
  const [inspected, setInspected] = useState(false);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [system, setSystem] = useState('');
  const [sort, setSort] = useState('new');
  const [storage, setStorage] = useState('');
  const active = useRef(false);
  const lock = useRef(false);
  const deviceId = device?.id;
  const boardUid = device?.boardUid;
  const readCatalog = useCallback(async () => {
    if (!hasCatalog || !deviceId || lock.current) return;
    lock.current = true;
    setReading(true);
    setError('');
    setInspected(false);
    try {
      const catalog = await invokeCommand('workbench_firmware_catalog', {
        deviceId,
        channel: 'stable',
      });
      if (!active.current) return;
      if (catalog.deviceId !== deviceId || catalog.boardUid !== boardUid)
        throw new Error('目标设备已变化，请重新读取目录。');
      setEntries(
        catalog.entries.filter(
          (v) =>
            v.entry.status === 'active' &&
            v.entry.manifest.applicationId === 'nekobox' &&
            v.entry.manifest.boardModel.toLowerCase() === 'yuzukineko'
        )
      );
      setInspected(true);
    } catch (e) {
      if (active.current) {
        setEntries([]);
        setError(formatErrorMessage(e));
      }
    } finally {
      lock.current = false;
      if (active.current) setReading(false);
    }
  }, [hasCatalog, deviceId, boardUid]);
  useEffect(() => {
    active.current = true;
    const start = window.setTimeout(() => void readCatalog(), 0);
    return () => {
      active.current = false;
      window.clearTimeout(start);
    };
  }, [readCatalog]);
  async function obtain(v: FirmwareCatalogVersion) {
    if (!deviceId || lock.current) return;
    lock.current = true;
    setReading(true);
    setError('');
    try {
      const loader = await open({
        title: '选择这块 F101 对应的 RAM FES Loader',
        multiple: false,
        filters: [
          { name: 'RAM FES 引导来源', extensions: ['img', 'fex', 'bin'] },
          { name: '所有文件', extensions: ['*'] },
        ],
      });
      if (typeof loader !== 'string' || !active.current) return;
      const c = await invokeCommand('workbench_firmware_download', {
        deviceId,
        request: {
          releaseId: v.releaseId,
          applicationId: v.entry.manifest.applicationId,
          version: v.entry.manifest.version,
          channel: 'stable',
          loaderPath: loader,
        },
      });
      if (!active.current) return;
      if (c.deviceId !== deviceId || c.boardUid !== boardUid)
        throw new Error('候选包的设备作用域已改变。');
      onInspect?.(v);
      onChoose(c);
    } catch (e) {
      if (active.current) setError(formatErrorMessage(e));
    } finally {
      lock.current = false;
      if (active.current) setReading(false);
    }
  }
  const filtered = entries
    .filter(
      (v) =>
        (!system || system === v.entry.manifest.applicationId) &&
        (!storage || storage === v.entry.manifest.route) &&
        [v.entry.manifest.applicationId, v.entry.manifest.version, v.entry.manifest.image.fileName]
          .join(' ')
          .toLowerCase()
          .includes(search.trim().toLowerCase())
    )
    .sort((a, b) =>
      sort === 'small'
        ? a.entry.manifest.image.sizeBytes - b.entry.manifest.image.sizeBytes
        : b.releaseId - a.releaseId
    );
  return (
    <section className="v2c-resources" aria-label="板型资源目录">
      <div className="v2c-resource-filter">
        <label className="v2c-search">
          <LynxIcon name="search" size={16} />
          <input
            type="search"
            aria-label="搜索系统或版本"
            placeholder="搜索系统或版本…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </label>
        <select aria-label="筛选系统" value={system} onChange={(e) => setSystem(e.target.value)}>
          <option value="">全部系统</option>
          {Array.from(new Set(entries.map((v) => v.entry.manifest.applicationId))).map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        <select
          aria-label="筛选存储路线"
          value={storage}
          onChange={(e) => setStorage(e.target.value)}
        >
          <option value="">全部路线</option>
          {Array.from(new Set(entries.map((v) => v.entry.manifest.route))).map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        <select aria-label="排序" value={sort} onChange={(e) => setSort(e.target.value)}>
          <option value="new">最新优先</option>
          <option value="small">最小优先</option>
        </select>
        <button
          type="button"
          title="刷新发布目录"
          aria-label="刷新发布目录"
          disabled={!deviceId || reading || !hasCatalog}
          onClick={() => void readCatalog()}
        >
          <LynxIcon name="refresh" size={16} />
        </button>
      </div>
      {filtered.map((v) => (
        <div
          key={v.releaseId + '-' + v.entry.manifest.version}
          className={
            'v2c-resource-row' +
            (selected?.manifest.image.sha256 === v.entry.manifest.image.sha256 ? ' selected' : '')
          }
        >
          <button
            type="button"
            disabled={reading}
            className="v2c-get"
            onClick={() => void obtain(v)}
            aria-label={
              '获取并核对 ' + v.entry.manifest.applicationId + ' ' + v.entry.manifest.version
            }
          >
            <span className="v2c-radio-dot" aria-hidden="true" />
            <span className="v2c-resource-icon">
              <LynxIcon name="box" size={24} />
            </span>
            <span className="v2c-entry-copy">
              <strong>
                {v.entry.manifest.applicationId} {v.entry.manifest.version}
              </strong>
              <small>
                {v.entry.manifest.image.fileName} · {v.entry.manifest.boardModel}
              </small>
            </span>
            <small className="v2c-format">{v.entry.manifest.route}</small>
            <span>{formatLinuxBytes(v.entry.manifest.image.sizeBytes)}</span>
          </button>
          <button
            type="button"
            aria-label={'查看 ' + v.entry.manifest.version + ' 资源详情'}
            title="查看资源详情"
            onClick={() => onInspect?.(v)}
          >
            <LynxIcon name="info" size={16} />
          </button>
        </div>
      ))}
      {!filtered.length && (
        <div className="v2c-catalog-empty">
          <LynxIcon name="archive" size={40} />
          <strong>
            {!hasCatalog
              ? '此板型尚未配置发布目录'
              : reading
                ? '正在读取板型资源…'
                : !deviceId
                  ? '选择资源对应的单板'
                  : error
                    ? '发布目录读取失败'
                    : inspected && entries.length
                      ? '没有匹配的系统'
                      : '暂无可用的发布资源'}
          </strong>
          <p>
            {!hasCatalog
              ? '可使用与该开发板匹配的本地镜像。'
              : !deviceId
                ? '也可直接选择本地文件，无需连接 USB。'
                : reading
                  ? '正在核对发布清单。'
                  : error
                    ? '检查网络后刷新，或选择本地镜像。'
                    : '可调整筛选条件，或选择本地镜像。'}
          </p>
          {onLocal && !reading && (
            <button type="button" onClick={onLocal}>
              <LynxIcon name="folder" size={16} />
              选择本地文件
            </button>
          )}
        </div>
      )}
      {error && (
        <p role="alert" className="v2c-inline-error">
          {error}
        </p>
      )}
    </section>
  );
}
