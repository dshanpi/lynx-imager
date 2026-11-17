import React, { useCallback, useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC/Client';
import type { CloudHardwareGuide, CloudHardwareItem } from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import './MyHardware.css';
import { LearnResources } from './learn/LearnResources';
import { LearnBreadcrumb, LearnFooter, LearnCourseArt, LearnBadge } from './learn/LearnChrome';
import { learnRedeemNeedsSync } from './learn/model';
import type { LearningCourse } from './types';

const HARDWARE_SITE = 'https://www.100ask.net/ai-agent';

interface MyHardwareProps {
  courses: LearningCourse[];
  onSelectCourse: (course: LearningCourse) => void;
  isActive: boolean;
  authUser: { id: number; name: string } | null;
  onRequestLogin: () => void;
  onOpenLabs: () => void;
}

function hardwareErrorMessage(error: unknown): string {
  const structured =
    typeof error === 'object' && error !== null
      ? (error as { machineCode?: unknown; message?: unknown })
      : null;
  const raw =
    typeof structured?.message === 'string' ? structured.message : formatErrorMessage(error);
  const legacyCode = raw.split(':', 1)[0];
  const code = typeof structured?.machineCode === 'string' ? structured.machineCode : legacyCode;
  return (
    (
      {
        AUTH_REQUIRED: '请先登录 100ASK 账号。',
        AUTH_TOKEN_EXPIRED: '登录已过期，请重新登录。',
        HARDWARE_SERVICE_NOT_DEPLOYED: '官网“我的硬件”接口尚未部署，资料引导仍可正常使用。',
        HARDWARE_API_UNAVAILABLE: '我的硬件服务暂不可用，请稍后重试。',
        HARDWARE_API_INVALID_RESPONSE: '官网硬件服务响应无效。',
        HARDWARE_API_SCHEMA_MISMATCH: '官网硬件字段与当前 LYNX 不兼容。',
        HARDWARE_GUIDE_URL_INVALID: '官网返回了不安全或无效的资料链接，LYNX 已阻止打开。',
        ORDER_CODE_INVALID: '请输入 6–64 位订单号或兑换码，只能包含字母、数字、点、横线和下划线。',
        ORDER_REDEEM_INVALID: '订单号或兑换码无效，请核对后重试。',
        ORDER_REDEEM_FORBIDDEN: '该订单不能兑换到当前账号。',
        ORDER_ALREADY_REDEEMED: '该订单已经绑定到账号，请刷新硬件列表。',
        LEARNING_RATE_LIMITED: '操作过于频繁，请稍后重试。',
      } as Record<string, string>
    )[code] || '我的硬件操作失败，请稍后重试。'
  );
}

function statusLabel(status: string): string {
  return (
    (
      {
        active: '可用',
        bound: '已绑定',
        pending: '待确认',
        disabled: '已停用',
      } as Record<string, string>
    )[status] ||
    status ||
    '已绑定'
  );
}

export const MyHardware: React.FC<MyHardwareProps> = ({
  courses,
  onSelectCourse,
  isActive,
  authUser,
  onRequestLogin,
  onOpenLabs,
}) => {
  const redeemLock = useRef(false);
  const [needsSync, setNeedsSync] = useState(false);
  const [resourcesOpen, setResourcesOpen] = useState(false);
  const [items, setItems] = useState<CloudHardwareItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState('');
  const [orderCode, setOrderCode] = useState('');
  const [redeeming, setRedeeming] = useState(false);
  const [redeemMessage, setRedeemMessage] = useState('');
  const [redeemError, setRedeemError] = useState('');
  const [guideError, setGuideError] = useState('');
  const [lastRedeemed, setLastRedeemed] = useState<CloudHardwareItem | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);

  const loadHardware = useCallback(async () => {
    if (!authUser || loading) return;
    setLoading(true);
    setLoadError('');
    try {
      const overview = await invokeCommand('learning_list_hardware');
      setItems(overview.items);
      setNeedsSync(false);
    } catch (error) {
      setLoadError(hardwareErrorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [authUser, loading]);

  useEffect(() => {
    if (!authUser) {
      setItems([]);
      setLoadError('');
      setRedeemMessage('');
      setRedeemError('');
      return;
    }
    if (isActive) void loadHardware();
    // One account-scoped refresh when this page becomes active.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authUser?.id, isActive]);

  const redeem = async () => {
    if (redeemLock.current || needsSync) return;
    if (!authUser) {
      onRequestLogin();
      return;
    }
    const normalized = orderCode.trim();
    if (!/^[A-Za-z0-9._-]{6,64}$/.test(normalized)) {
      setRedeemError('请输入 6–64 位订单号或兑换码，只能包含字母、数字、点、横线和下划线。');
      return;
    }
    redeemLock.current = true;
    setRedeeming(true);
    setRedeemError('');
    setRedeemMessage('');
    try {
      const result = await invokeCommand('learning_redeem_hardware_order', {
        orderCode: normalized,
      });
      setItems((current) => [
        result.hardware,
        ...current.filter((item) => item.id !== result.hardware.id),
      ]);
      setLastRedeemed(result.hardware);
      setOrderCode('');
      setRedeemMessage(result.message || `${result.hardware.name || '硬件'}已绑定到当前账号。`);
    } catch (error) {
      setNeedsSync(learnRedeemNeedsSync(error));
      setRedeemError(hardwareErrorMessage(error));
    } finally {
      redeemLock.current = false;
      setRedeeming(false);
    }
  };

  const openGuide = async (guide: Pick<CloudHardwareGuide, 'url'>) => {
    setGuideError('');
    try {
      await openUrl(guide.url);
    } catch (error) {
      setGuideError(`无法打开资料链接：${formatErrorMessage(error)}`);
    }
  };

  if (resourcesOpen)
    return (
      <LearnResources
        onSync={() => void loadHardware()}
        syncing={loading}
        items={items}
        courses={courses}
        onBack={() => setResourcesOpen(false)}
        onSelectCourse={onSelectCourse}
      />
    );

  return (
    <div
      className="learning-hardware-page learn-journey learn-v2-page"
      data-testid="learning-hardware-page"
    >
      <LearnBreadcrumb items={['学习中心', '我的硬件', '订单兑换']}>
        <button className="learning-secondary" onClick={onRequestLogin}>
          <i className={`learn-status-dot ${authUser ? 'online' : ''}`} />
          {authUser ? `已登录 · ${authUser.name}` : '登录 100ASK'}
        </button>
        <span className="learn-env-pill">
          <i className="learn-status-dot" />
          开发板 · 连接独立核验
        </span>
      </LearnBreadcrumb>
      <div className="learn-v2-body learn-redeem-layout">
        <section
          className="learning-hardware-redeem learn-panel"
          data-testid="learning-order-redeem"
        >
          <h1>兑换你的产品与课程权益</h1>
          <p>输入购买订单号，核验并兑换对应的硬件与配套资料。</p>
          <div className="learn-redeem-form">
            <label htmlFor="learn-redeem-method">选择兑换方式</label>
            <select id="learn-redeem-method">
              <option>购买订单号 / 兑换码</option>
            </select>
            <span />
            <label htmlFor="learning-order-code">订单号</label>
            <input
              id="learning-order-code"
              data-testid="learning-order-code"
              value={orderCode}
              onChange={(event) => {
                setOrderCode(event.target.value);
                setRedeemError('');
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void redeem();
              }}
              disabled={!authUser || redeeming}
              maxLength={64}
              autoComplete="off"
              spellCheck={false}
              placeholder={authUser ? '输入购买订单号或兑换码' : '请先登录后兑换'}
            />
            <button className="learning-text-button" onClick={() => setHelpOpen((value) => !value)}>
              在哪里找到订单号？
            </button>
            <span />
            <button
              onClick={() => void redeem()}
              disabled={!authUser || redeeming || needsSync || !orderCode.trim()}
              data-testid="learning-order-redeem-submit"
            >
              {redeeming ? '核验中…' : '核验并兑换'}
            </button>
            <span />
            <small>权益将绑定到当前账号。请确认使用正确的购买账号进行兑换。</small>
            {helpOpen && (
              <p className="learn-notice">
                在购买平台的订单详情中复制订单编号；若收到独立兑换码，可直接粘贴。无法核验时请联系购买渠道核对。
              </p>
            )}
          </div>
          {redeemMessage && lastRedeemed && (
            <section className="learn-redemption-result" role="status">
              <h2>✓ 权益已到账</h2>
              <p>{redeemMessage}</p>
              <div>
                <div className="learn-redeemed-product">
                  <LearnCourseArt />
                  <strong>{lastRedeemed.name || lastRedeemed.model}</strong>
                  <small>订单：{lastRedeemed.orderNumberMasked || '已脱敏'}</small>
                </div>
                <div>
                  <h3>已获得的资料入口</h3>
                  {lastRedeemed.guides.map((guide) => (
                    <button
                      className="learning-text-button"
                      key={guide.id}
                      onClick={() => void openGuide(guide)}
                    >
                      <LynxIcon name="document" size={24} />
                      {guide.title}
                    </button>
                  ))}
                  {!lastRedeemed.guides.length && <p>官网尚未返回配套资料。可同步权益后查看。</p>}
                  <button className="learning-secondary" onClick={() => setResourcesOpen(true)}>
                    查看本次权益与资源 →
                  </button>
                  <small>课程权益在资源页按账号实际结果展示。</small>
                </div>
              </div>
            </section>
          )}
          {!redeemMessage && (
            <section className="learn-redeem-ready">
              <LynxIcon name="box" size={40} />
              <div>
                <h3>{authUser ? '输入订单，查看属于你的资源' : '先登录，再兑换购买权益'}</h3>
                <p>配套资料与课程会按官网返回结果展示。</p>
              </div>
              {!authUser && <button onClick={onRequestLogin}>登录 100ASK</button>}
            </section>
          )}
          {redeemError && (
            <div className="learning-hardware-result error" role="alert">
              <LynxIcon name="error" size={16} />
              {redeemError}
            </div>
          )}
          {needsSync && (
            <p role="status" className="learn-notice">
              本次兑换结果需要确认。请先“同步硬件”核对已有权益，再决定是否重新提交。
            </p>
          )}
        </section>
        <aside
          className="learning-hardware-onboarding learn-panel"
          data-testid="learning-hardware-guides"
        >
          <h2>如何兑换？</h2>
          <ol className="learn-redeem-steps">
            {[
              ['输入订单号', '选择兑换方式并输入购买时的订单号。'],
              ['核验权益', '系统将核验订单信息与可领取的资源。'],
              ['查看配套资源', '兑换成功后，可在资源页查看资料与账号课程。'],
            ].map(([title, detail], i) => (
              <li key={title}>
                <b>{i + 1}</b>
                <div>
                  <strong>{title}</strong>
                  <p>{detail}</p>
                </div>
              </li>
            ))}
          </ol>
          <div className="learn-notice">
            <strong>拥有课程权益 ≠ 已连接开发板</strong>
            <p>即使暂时没有开发板，也可以先学习课程与查阅资料，连接后再完成实践。</p>
          </div>
          <button className="learning-secondary" onClick={() => setResourcesOpen(true)}>
            查看配套资源 →
          </button>
          <div className="learn-button-row">
            <button
              className="learning-text-button"
              onClick={() => void openGuide({ url: HARDWARE_SITE })}
            >
              打开官网
            </button>
            <button className="learning-text-button" onClick={onOpenLabs}>
              实验中心
            </button>
          </div>
        </aside>
        <section className="learn-panel learn-redemption-history">
          <div className="learn-section-title">
            <div>
              <h2>已兑换资源记录</h2>
              <small>这里只展示官网返回的账号硬件，不生成示例订单或设备。</small>
            </div>
            <button
              className="learning-secondary"
              onClick={() => void loadHardware()}
              disabled={!authUser || loading}
              data-testid="learning-hardware-refresh"
            >
              <LynxIcon name="refresh" size={16} spin={loading} />
              {loading ? '同步中' : '同步硬件'}
            </button>
          </div>
          {loadError && (
            <div
              className="learning-inline-error"
              role="alert"
              data-testid="learning-hardware-error"
            >
              <strong>硬件列表暂不可用</strong>
              <span>{loadError}</span>
              <button onClick={() => void loadHardware()} disabled={loading}>
                重试
              </button>
            </div>
          )}
          <div className="learn-table-wrap">
            <table className="learn-table">
              <thead>
                <tr>
                  <th>绑定时间</th>
                  <th>订单号</th>
                  <th>产品 / 权益</th>
                  <th>状态</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => (
                  <tr key={item.id}>
                    <td>
                      {item.boundAt ? new Date(item.boundAt * 1000).toLocaleDateString() : '未提供'}
                    </td>
                    <td>{item.orderNumberMasked || '未提供'}</td>
                    <td>{item.name || item.model}</td>
                    <td>
                      <LearnBadge
                        tone={['bound', 'active'].includes(item.status) ? 'green' : 'gray'}
                      >
                        {statusLabel(item.status)}
                      </LearnBadge>
                    </td>
                    <td>
                      <button
                        className="learning-text-button"
                        onClick={() => setResourcesOpen(true)}
                      >
                        查看配套资源 →
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!items.length && (
            <p className="learn-empty" data-testid="learning-hardware-empty">
              {authUser
                ? '当前没有已绑定硬件。输入订单号或同步权益后查看。'
                : '登录后查看我的硬件与兑换记录。'}
            </p>
          )}
        </section>
        {guideError && (
          <p className="learn-notice" role="alert">
            {guideError}
          </p>
        )}
      </div>
      <LearnFooter>Access Token 始终保留在 LYNX Rust 后端</LearnFooter>
    </div>
  );
};
