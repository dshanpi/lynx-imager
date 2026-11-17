import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import type {
  RecordCenterBackup,
  RecordCenterEvent,
  RecordCenterFeedbackDraft,
  RecordCenterRestorePreview,
  RecordCenterSession,
} from '../../Platform/IPC/Commands';
import { appendRecordEvent } from './recordCenterService';
import './RecordCenter.css';

type Section = 'overview' | 'events' | 'sessions' | 'memory' | 'feedback' | 'backup' | 'access';

const sections: Array<[Section, string, LynxIconName]> = [
  ['overview', '记录总览', 'history'],
  ['events', '全部事件', 'list'],
  ['sessions', '会话记录', 'link'],
  ['memory', '操作记忆', 'database'],
  ['feedback', '问题反馈', 'error'],
  ['backup', '备份与恢复', 'archive'],
  ['access', '接口与权限', 'shield'],
];

const emptyOverview = {
  todayEventCount: 0,
  activeSerialSessions: 0,
  aiSessionCount: 0,
  deviceErrorCount: 0,
  pendingFeedbackCount: 0,
  gapCount: 0,
  latestBackupAtMs: null as number | null,
  lanServiceEnabled: false,
};

function time(value: number | null) {
  return value ? new Date(value).toLocaleString() : '暂无';
}

function resultLabel(result: string) {
  return { success: '成功', failed: '失败', denied: '拒绝', pending: '处理中' }[result] ?? result;
}

function scopeLabel(event: RecordCenterEvent) {
  const boardName = event.metadata.boardName;
  return typeof boardName === 'string' && boardName ? boardName : event.scopeId || '全局';
}

function sessionDomainLabel(session: RecordCenterSession) {
  if (session.scopeId.startsWith('linux:')) return 'Linux / SoC';
  if (session.scopeId.startsWith('mcu:')) return 'MCU';
  return session.kind;
}

export const RecordCenter: React.FC<{ isActive: boolean; actorId: string }> = ({
  isActive,
  actorId,
}) => {
  const [section, setSection] = useState<Section>('overview');
  const [overview, setOverview] = useState(emptyOverview);
  const [events, setEvents] = useState<RecordCenterEvent[]>([]);
  const [sessions, setSessions] = useState<RecordCenterSession[]>([]);
  const [feedback, setFeedback] = useState<RecordCenterFeedbackDraft[]>([]);
  const [backups, setBackups] = useState<RecordCenterBackup[]>([]);
  const [restorePreview, setRestorePreview] = useState<RecordCenterRestorePreview | null>(null);
  const [selectedEventId, setSelectedEventId] = useState('');
  const [query, setQuery] = useState('');
  const [actorType, setActorType] = useState('');
  const [result, setResult] = useState('');
  const [scopeId, setScopeId] = useState('');
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState('');
  const enteredRef = useRef(false);
  const [draft, setDraft] = useState({
    title: '',
    problemType: 'UI 异常',
    page: '',
    expected: '',
    actual: '',
    reproducible: '待确认',
  });

  const selectedEvent = events.find((event) => event.eventId === selectedEventId) ?? events[0];

  const refresh = useCallback(async () => {
    if (!isActive) return;
    setLoading(true);
    setMessage('');
    try {
      const [nextOverview, nextEvents, nextSessions, nextFeedback, nextBackups] = await Promise.all(
        [
          invokeCommand('record_center_overview'),
          invokeCommand('record_center_list_events', {
            filter: { query, actorType, result, scopeId, limit: 300 },
          }),
          invokeCommand('record_center_list_sessions', { limit: 200 }),
          invokeCommand('record_center_list_feedback_drafts'),
          invokeCommand('record_center_list_backups'),
        ]
      );
      setOverview(nextOverview);
      setEvents(nextEvents);
      setSessions(nextSessions);
      setFeedback(nextFeedback);
      setBackups(nextBackups);
      setSelectedEventId((current) =>
        nextEvents.some((event) => event.eventId === current)
          ? current
          : (nextEvents[0]?.eventId ?? '')
      );
    } catch (error) {
      setMessage(formatErrorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [actorType, isActive, query, result, scopeId]);

  useEffect(() => {
    if (!isActive) {
      enteredRef.current = false;
      return;
    }
    if (!enteredRef.current) {
      enteredRef.current = true;
      void appendRecordEvent({
        actorType: 'user',
        actorId,
        category: 'ui',
        action: 'record_center.open',
        scopeType: 'application',
        scopeId: 'record-center',
        result: 'success',
        summary: '打开记录中心',
        metadata: { section: 'overview' },
      })
        .then(() => refresh())
        .catch((error) => setMessage(formatErrorMessage(error)));
      return;
    }
    void refresh();
  }, [actorId, isActive, refresh]);

  useEffect(() => {
    if (isActive && section === 'events') void refresh();
  }, [actorType, query, refresh, result, scopeId, isActive, section]);

  const cards = useMemo(
    () => [
      ['今日操作', overview.todayEventCount],
      ['活跃串口', overview.activeSerialSessions],
      ['AI 会话', overview.aiSessionCount],
      ['设备异常', overview.deviceErrorCount],
      ['未处理问题', overview.pendingFeedbackCount],
      ['记录缺口', overview.gapCount],
      ['最近备份', time(overview.latestBackupAtMs)],
      ['LAN 反馈服务', overview.lanServiceEnabled ? '已启用' : '已关闭'],
    ],
    [overview]
  );

  async function saveDraft() {
    setMessage('');
    try {
      await invokeCommand('record_center_save_feedback_draft', {
        actorId,
        request: {
          ...draft,
          eventIds: selectedEvent ? [selectedEvent.eventId] : [],
          includeSensitiveBody: false,
        },
      });
      setDraft((current) => ({ ...current, title: '', expected: '', actual: '' }));
      await refresh();
      setMessage('已保存为本地反馈草稿；没有上传任何数据。');
    } catch (error) {
      setMessage(formatErrorMessage(error));
    }
  }

  async function createBackup() {
    const target = await open({ directory: true, multiple: false, title: '选择记录备份目录' });
    if (!target || Array.isArray(target)) return;
    setMessage('正在生成非覆盖备份…');
    try {
      const backup = await invokeCommand('record_center_create_backup', {
        targetDir: target,
        actorId,
      });
      await refresh();
      setMessage(`备份完成：${backup.path}`);
    } catch (error) {
      setMessage(formatErrorMessage(error));
    }
  }

  async function previewRestore() {
    const source = await open({
      directory: false,
      multiple: false,
      title: '选择 LYNX 记录备份',
      filters: [{ name: 'LYNX 记录备份', extensions: ['json'] }],
    });
    if (!source || Array.isArray(source)) return;
    setMessage('正在校验备份并生成恢复预览…');
    try {
      const preview = await invokeCommand('record_center_preview_restore', {
        sourcePath: source,
        actorId,
      });
      setRestorePreview(preview);
      setMessage('备份校验通过。请核对新增项和冲突项后再执行恢复。');
    } catch (error) {
      setRestorePreview(null);
      setMessage(formatErrorMessage(error));
    }
  }

  async function restoreBackup() {
    if (!restorePreview) return;
    setMessage('正在执行非覆盖恢复…');
    try {
      const restored = await invokeCommand('record_center_restore_backup', {
        sourcePath: restorePreview.path,
        actorId,
      });
      setRestorePreview(null);
      await refresh();
      setMessage(
        `恢复完成：新增 ${restored.importedEvents} 条事件、${restored.importedSessions} 个会话、${restored.importedFeedback} 个反馈草稿；跳过 ${restored.skippedConflicts} 个冲突项。`
      );
    } catch (error) {
      setMessage(formatErrorMessage(error));
    }
  }

  return (
    <div className="record-center">
      <header className="record-center-header">
        <div>
          <small>TRACEABILITY & RECORDS</small>
          <h1>记录中心</h1>
          <p>用户、AI、服务和设备行为统一记录、关联、备份与回查。</p>
        </div>
        <button type="button" onClick={() => void refresh()} disabled={loading}>
          <LynxIcon name="refresh" size={16} spin={loading} /> {loading ? '刷新中' : '刷新'}
        </button>
      </header>

      <nav className="record-center-tabs" aria-label="记录中心分类">
        {sections.map(([id, label, icon]) => (
          <button
            key={id}
            data-testid={`record-tab-${id}`}
            className={section === id ? 'is-active' : ''}
            onClick={() => setSection(id)}
          >
            <LynxIcon name={icon} size={16} filled={section === id} /> {label}
          </button>
        ))}
      </nav>

      {message && <div className="record-center-message">{message}</div>}

      {section === 'overview' && (
        <main className="record-overview">
          <section className="record-metrics">
            {cards.map(([label, value]) => (
              <article
                key={label}
                className={label === '记录缺口' && Number(value) > 0 ? 'is-danger' : ''}
              >
                <span>{label}</span>
                <strong>{value}</strong>
              </article>
            ))}
          </section>
          <section className="record-panel">
            <header>
              <div>
                <small>UNIFIED TIMELINE</small>
                <h2>最近事件</h2>
              </div>
              <button onClick={() => setSection('events')}>查看全部</button>
            </header>
            <div className="record-timeline">
              {events.slice(0, 12).map((event) => (
                <button
                  key={event.eventId}
                  onClick={() => {
                    setSelectedEventId(event.eventId);
                    setSection('events');
                  }}
                >
                  <time>{new Date(event.occurredAtMs).toLocaleTimeString()}</time>
                  <i className={`is-${event.result}`} />
                  <span>
                    <b>{event.actorId || event.actorType}</b>
                    {event.summary}
                  </span>
                  <em>{scopeLabel(event)}</em>
                </button>
              ))}
              {!events.length && (
                <p className="record-empty">暂无事件。真实操作发生后会显示在这里。</p>
              )}
            </div>
          </section>
        </main>
      )}

      {section === 'events' && (
        <main className="record-event-workspace">
          <aside className="record-filters">
            <h2>筛选</h2>
            <label>
              <span>搜索</span>
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="事件、设备、错误码"
              />
            </label>
            <label>
              <span>操作者</span>
              <select value={actorType} onChange={(event) => setActorType(event.target.value)}>
                <option value="">全部主体</option>
                <option value="user">用户</option>
                <option value="ai">AI</option>
                <option value="service">后台服务</option>
                <option value="device">设备</option>
              </select>
            </label>
            <label>
              <span>设备 / 作用域</span>
              <input
                value={scopeId}
                onChange={(event) => setScopeId(event.target.value)}
                placeholder="板卡名称或作用域"
              />
            </label>
            <label>
              <span>结果</span>
              <select value={result} onChange={(event) => setResult(event.target.value)}>
                <option value="">全部结果</option>
                <option value="success">成功</option>
                <option value="failed">失败</option>
                <option value="denied">拒绝</option>
              </select>
            </label>
          </aside>
          <section className="record-event-list">
            <header>
              <h2>事件时间线</h2>
              <span>{events.length} 条</span>
            </header>
            {events.map((event) => (
              <button
                key={event.eventId}
                className={selectedEvent?.eventId === event.eventId ? 'is-active' : ''}
                onClick={() => setSelectedEventId(event.eventId)}
              >
                <time>{time(event.occurredAtMs)}</time>
                <i className={`is-${event.result}`} />
                <span>
                  <strong>{event.summary}</strong>
                  <small>
                    {event.actorId || event.actorType} · {scopeLabel(event)}
                  </small>
                </span>
                <em>{resultLabel(event.result)}</em>
              </button>
            ))}
            {!events.length && <p className="record-empty">没有符合当前条件的真实事件。</p>}
          </section>
          <aside className="record-event-detail">
            <h2>事件详情</h2>
            {selectedEvent ? (
              <dl>
                <div>
                  <dt>操作者</dt>
                  <dd>
                    {selectedEvent.actorType} · {selectedEvent.actorId || '未命名主体'}
                  </dd>
                </div>
                <div>
                  <dt>目标作用域</dt>
                  <dd>
                    {selectedEvent.scopeType} · {selectedEvent.scopeId || '全局'}
                  </dd>
                </div>
                <div>
                  <dt>操作</dt>
                  <dd>{selectedEvent.action}</dd>
                </div>
                <div>
                  <dt>结果</dt>
                  <dd>{resultLabel(selectedEvent.result)}</dd>
                </div>
                <div>
                  <dt>错误码</dt>
                  <dd>{selectedEvent.errorCode || '无'}</dd>
                </div>
                <div>
                  <dt>会话</dt>
                  <dd>{selectedEvent.sessionId || '未关联'}</dd>
                </div>
                <div>
                  <dt>关联链</dt>
                  <dd>{selectedEvent.correlationId}</dd>
                </div>
                <div>
                  <dt>触发事件</dt>
                  <dd>{selectedEvent.causationId || '根事件'}</dd>
                </div>
                <div>
                  <dt>证据元数据</dt>
                  <dd>
                    <pre>{JSON.stringify(selectedEvent.metadata, null, 2)}</pre>
                  </dd>
                </div>
              </dl>
            ) : (
              <p className="record-empty">选择事件查看完整因果字段。</p>
            )}
          </aside>
        </main>
      )}

      {section === 'sessions' && (
        <main className="record-split-page">
          <section className="record-panel">
            <header>
              <div>
                <small>SESSIONS</small>
                <h2>会话记录</h2>
              </div>
              <span>{sessions.length} 个</span>
            </header>
            <div className="record-session-list">
              {sessions.map((session) => (
                <button
                  key={session.sessionId}
                  onClick={() => {
                    setQuery(session.sessionId);
                    setSection('events');
                  }}
                >
                  <span>
                    <strong>{session.title}</strong>
                    <small>{sessionDomainLabel(session)}</small>
                  </span>
                  <em>
                    {session.eventCount} 条事件
                    <br />
                    完整性：{session.integrity === 'complete' ? '正常' : '存在缺口'}
                  </em>
                  <time>{time(session.startedAtMs)}</time>
                </button>
              ))}
              {!sessions.length && (
                <p className="record-empty">暂无会话记录。串口等通道接入后会在这里建立会话。</p>
              )}
            </div>
          </section>
        </main>
      )}

      {section === 'memory' && (
        <main className="record-placeholder">
          <LynxIcon name="database" size={40} />
          <h2>结构化操作记忆</h2>
          <p>
            第一阶段先完成事件、会话、反馈和备份闭环。操作记忆的数据治理与引用审计进入第二阶段，不会用聊天全文冒充记忆。
          </p>
          <span>当前状态：尚未启用</span>
        </main>
      )}

      {section === 'feedback' && (
        <main className="record-feedback-grid">
          <section className="record-panel record-feedback-form">
            <header>
              <div>
                <small>LOCAL DRAFT</small>
                <h2>创建问题草稿</h2>
              </div>
              <span>不会自动上传</span>
            </header>
            <div className="record-form-grid">
              <label>
                <span>标题</span>
                <input
                  data-testid="record-feedback-title"
                  value={draft.title}
                  onChange={(e) => setDraft({ ...draft, title: e.target.value })}
                />
              </label>
              <label>
                <span>问题类型</span>
                <select
                  value={draft.problemType}
                  onChange={(e) => setDraft({ ...draft, problemType: e.target.value })}
                >
                  <option>UI 异常</option>
                  <option>设备连接</option>
                  <option>串口异常</option>
                  <option>任务失败</option>
                  <option>记录缺口</option>
                </select>
              </label>
              <label>
                <span>当前页面</span>
                <input
                  value={draft.page}
                  onChange={(e) => setDraft({ ...draft, page: e.target.value })}
                />
              </label>
              <label>
                <span>是否可复现</span>
                <select
                  value={draft.reproducible}
                  onChange={(e) => setDraft({ ...draft, reproducible: e.target.value })}
                >
                  <option>待确认</option>
                  <option>可以复现</option>
                  <option>偶发</option>
                  <option>无法复现</option>
                </select>
              </label>
              <label className="is-wide">
                <span>预期结果</span>
                <textarea
                  value={draft.expected}
                  onChange={(e) => setDraft({ ...draft, expected: e.target.value })}
                />
              </label>
              <label className="is-wide">
                <span>实际结果</span>
                <textarea
                  value={draft.actual}
                  onChange={(e) => setDraft({ ...draft, actual: e.target.value })}
                />
              </label>
            </div>
            <div className="record-evidence-note">
              <strong>关联证据</strong>
              <span>{selectedEvent ? `当前选中事件 ${selectedEvent.eventId}` : '未选择事件'}</span>
              <span>串口正文、AI 正文、截图：默认不包含</span>
            </div>
            <footer>
              <button
                data-testid="record-feedback-save"
                className="is-primary"
                disabled={!draft.title.trim()}
                onClick={() => void saveDraft()}
              >
                保存为本地问题
              </button>
            </footer>
          </section>
          <section className="record-panel">
            <header>
              <div>
                <small>DRAFTS</small>
                <h2>本地问题</h2>
              </div>
              <span>{feedback.length} 个</span>
            </header>
            <div className="record-draft-list">
              {feedback.map((item) => (
                <article key={item.id}>
                  <strong>{item.title}</strong>
                  <span>
                    {item.problemType} · {item.page || '未指定页面'}
                  </span>
                  <small>
                    {item.id} · {time(item.updatedAtMs)}
                  </small>
                </article>
              ))}
              {!feedback.length && <p className="record-empty">还没有本地反馈草稿。</p>}
            </div>
          </section>
        </main>
      )}

      {section === 'backup' && (
        <main className="record-split-page">
          <section className="record-panel">
            <header>
              <div>
                <small>LOCAL BACKUP</small>
                <h2>备份与恢复</h2>
              </div>
              <div className="record-panel-actions">
                <button onClick={() => void previewRestore()}>选择备份恢复</button>
                <button className="is-primary" onClick={() => void createBackup()}>
                  创建本地备份
                </button>
              </div>
            </header>
            <div className="record-backup-scope">
              <span>包含：事件索引、会话元数据、反馈草稿</span>
              <span>不包含：API Key、密码、私钥、Token、串口/AI 敏感正文</span>
              <span>写入策略：新文件、禁止覆盖、SHA-256 完整性摘要</span>
              <span>当前切片：非敏感本地索引备份；加密归档尚未开放，不能作为远程传输包</span>
            </div>
            {restorePreview && (
              <section className="record-restore-preview" aria-label="恢复预览">
                <header>
                  <span>
                    <small>RESTORE PREVIEW</small>
                    <strong>非覆盖恢复预览</strong>
                  </span>
                  <button className="is-primary" onClick={() => void restoreBackup()}>
                    执行非覆盖恢复
                  </button>
                </header>
                <div>
                  <span>
                    事件：新增 {restorePreview.newEvents} · 冲突 {restorePreview.conflictingEvents}
                  </span>
                  <span>
                    会话：新增 {restorePreview.newSessions} · 冲突{' '}
                    {restorePreview.conflictingSessions}
                  </span>
                  <span>
                    反馈：新增 {restorePreview.newFeedback} · 冲突{' '}
                    {restorePreview.conflictingFeedback}
                  </span>
                </div>
                <small>{restorePreview.path}</small>
                <small>SHA-256 {restorePreview.contentSha256}</small>
                <p>冲突项只跳过，不修改或覆盖当前记录。</p>
              </section>
            )}
            <div className="record-backup-list">
              {backups.map((item) => (
                <article key={item.id}>
                  <span>
                    <strong>{time(item.createdAtMs)}</strong>
                    <small>{item.path}</small>
                  </span>
                  <em>
                    {item.eventCount} 事件 · {item.sessionCount} 会话 · {item.feedbackCount} 草稿
                    <br />
                    SHA-256 {item.sha256}
                  </em>
                </article>
              ))}
              {!backups.length && <p className="record-empty">尚未在此设备上创建本地备份。</p>}
            </div>
          </section>
        </main>
      )}

      {section === 'access' && (
        <main className="record-placeholder">
          <LynxIcon name="shield" size={40} />
          <h2>接口与权限</h2>
          <p>
            第一阶段不开放局域网接口。后续必须由管理员显式启用、绑定指定网卡并完成设备证书配对。
          </p>
          <span>局域网记录服务：已关闭</span>
        </main>
      )}
    </div>
  );
};
