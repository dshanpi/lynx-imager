import { useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useAppVersion } from '../../../Hooks/useAppVersion';
import { invokeCommand } from '../../../Platform/IPC/Client';
import { formatLearnError } from './errors';
import {
  buildLearningPrecheck,
  readLearningInventory,
  summarizeLearningInventory,
} from '../precheck';
import type { LearningInventory, LearningLab } from '../types';
import { LearnBreadcrumb, LearnFooter } from './LearnChrome';
import './LearnHub.css';
export function LearnLabPreparation({
  isActive,
  lab,
  loggedIn,
  onLogin,
  onBack,
}: {
  isActive: boolean;
  lab: LearningLab;
  loggedIn: boolean;
  onLogin: () => void;
  onBack: () => void;
}) {
  const appVersion = useAppVersion();
  const [inventory, setInventory] = useState<LearningInventory | null>(null);
  const [board, setBoard] = useState<number | null>(null);
  const [host, setHost] = useState<number | null>(null);
  const [checkState, setCheckState] = useState<'idle' | 'loading'>('idle');
  const [packageState, setPackageState] = useState<'idle' | 'ready' | 'installed'>('idle');
  const [message, setMessage] = useState('');
  const lock = useRef(false);
  const run = async (fn: () => Promise<void>) => {
    if (!isActive || lock.current) return;
    lock.current = true;
    setCheckState('loading');
    try {
      await fn();
    } catch (e) {
      setMessage(formatLearnError(e));
    } finally {
      lock.current = false;
      setCheckState('idle');
    }
  };
  const runPrecheck = () => {
    if (!isActive || checkState === 'loading') return;
    void run(async () => setInventory(summarizeLearningInventory(await readLearningInventory())));
  };
  const checkPackage = () =>
    void run(async () => {
      if (packageState === 'ready') {
        const installed = await invokeCommand('learning_install_lab_package', {
          identifier: lab.key,
        });
        setPackageState('installed');
        setMessage(`实验包 ${installed.version} 已验签并安装。`);
      } else {
        const result = await invokeCommand('learning_check_lab_package', { identifier: lab.key });
        setPackageState('ready');
        setMessage(`版本 ${result.version} 可下载并安全安装。`);
      }
    });
  const checks = inventory
    ? buildLearningPrecheck(
        appVersion,
        loggedIn,
        { ...lab, packageState: packageState === 'installed' ? 'downloaded' : lab.packageState },
        inventory,
        board,
        host
      )
    : [];
  return (
    <div className="learn-journey learn-v2-page" data-testid="learn-lab-preparation">
      <LearnBreadcrumb items={['学习中心', '实验中心', lab.title]}>
        <button className="learning-secondary" onClick={onBack}>
          返回实验中心
        </button>
      </LearnBreadcrumb>
      <div className="learn-v2-body">
        <header className="learn-guide-heading">
          <h1>{lab.title}</h1>
          <p>{lab.summary}</p>
        </header>
        <div className="learn-preparation-grid">
          <section className="learn-panel learn-hub-section">
            <h2>01 · 课程权益与实验包</h2>
            <p>100ASK 账号：{loggedIn ? '已登录' : '未登录'}</p>
            <p>
              课程权益：
              {lab.entitlementRequired ? (lab.hasEntitlement ? '已拥有' : '尚未解锁') : '公开实验'}
            </p>
            {!loggedIn && <button onClick={onLogin}>登录账号</button>}
            <p>版本：{lab.packageVersion || '待发布'}</p>
            <button
              data-testid="learning-primary-launch-action"
              disabled={checkState === 'loading' || packageState === 'installed' || !loggedIn}
              onClick={checkPackage}
            >
              {packageState === 'installed'
                ? '已安装'
                : packageState === 'ready'
                  ? '下载并安全安装'
                  : '检查实验包'}
            </button>
          </section>
          <section className="learn-panel learn-hub-section">
            <h2>02 · SDK 与开发板</h2>
            <p>{lab.sdkRequirement}</p>
            <button onClick={runPrecheck} disabled={checkState === 'loading'}>
              读取已保存环境
            </button>
            {inventory && (
              <>
                <label>
                  Ubuntu 主机
                  <select
                    aria-label="Ubuntu 主机"
                    value={host ?? ''}
                    onChange={(e) => setHost(e.target.value ? Number(e.target.value) : null)}
                  >
                    <option value="">请选择</option>
                    {inventory.hosts.map((h) => (
                      <option key={h.id} value={h.id}>
                        {h.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  开发板
                  <select
                    aria-label="开发板"
                    value={board ?? ''}
                    onChange={(e) => setBoard(e.target.value ? Number(e.target.value) : null)}
                  >
                    <option value="">请选择</option>
                    {inventory.devices.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            )}
          </section>
        </div>
        {message && (
          <p className="learn-notice" role="status">
            {message}
          </p>
        )}
        <section className="learn-panel learn-hub-section">
          <h2>03 · 环境检查结果</h2>
          {checks.map((c) => (
            <div className="learn-session-row" key={c.id}>
              <strong>{c.label}</strong>
              <span>{c.value}</span>
              <small>{c.detail}</small>
            </div>
          ))}
          <p>
            本页读取保存的配置，不连接
            SSH、不扫描串口。官网实验包尚未提供到本地学习会话的转换接口；可按源教程操作，或在实验中心导入配套
            .lynx-course 课程。
          </p>
          <button
            className="learning-secondary"
            onClick={() =>
              void openUrl(`https://www.100ask.net/ai-agent/lab/${encodeURIComponent(lab.key)}`)
            }
          >
            查看实验源教程
          </button>
        </section>
      </div>
      <LearnFooter />
    </div>
  );
}
