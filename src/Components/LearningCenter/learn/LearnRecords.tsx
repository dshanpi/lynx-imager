import { useEffect, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LearnJourney,
  LocalLearningReport,
  LocalLearningSession,
} from '../../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../../Utils';
import { LynxIcon } from '../../../CoreUI';
import { LearnPrerequisites } from './LearnRevisit';
import { LearnBreadcrumb, LearnFooter, LearnBadge } from './LearnChrome';
import type { LearningProgress } from '../types';
import {
  learnLatestAttempt,
  learnReportTitle,
  localReportCurrencyText,
  localReportSoftwareResult,
  localReportProgress,
  localEnvironmentStatus,
  localSourceStatus,
  localPrerequisiteStatus,
} from './model';
import './LearnJourney.css';

export function LearnRecords({
  active,
  initialSessionId,
  onResume,
  cloudProgress = [],
}: {
  active: boolean;
  cloudProgress?: (LearningProgress & { title: string })[];
  initialSessionId?: string;
  onResume?: (sessionId: string, stepId?: string) => void;
}) {
  const [sessions, setSessions] = useState<LocalLearningSession[]>([]);
  const [sessionId, setSessionId] = useState(initialSessionId || '');
  const [record, setRecord] = useState<{
    journey: LearnJourney;
    report: LocalLearningReport;
  } | null>(null);
  const [error, setError] = useState('');
  const [format, setFormat] = useState('md');
  const [includeNotes, setIncludeNotes] = useState(false);
  const [includeEvents, setIncludeEvents] = useState(true);
  const [includeEvidence, setIncludeEvidence] = useState(true);
  const [showPreview, setShowPreview] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    void invokeCommand('learning_list_local_sessions')
      .then((items) => {
        if (!cancelled) {
          setSessions(items);
          setSessionId((current) => current || items[0]?.sessionId || '');
        }
      })
      .catch((reason) => {
        if (!cancelled) setError(formatErrorMessage(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [active]);
  useEffect(() => {
    if (!active || !sessionId) return;
    let cancelled = false;
    setRecord(null);
    setError('');
    setMessage('');
    void Promise.all([
      invokeCommand('learning_get_journey', { sessionId }),
      invokeCommand('learning_generate_local_report', { sessionId }),
    ])
      .then(([journey, report]) => {
        if (!cancelled) setRecord({ journey, report });
      })
      .catch((reason) => {
        if (!cancelled) setError(formatErrorMessage(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [active, sessionId]);
  const exportRecord = async () => {
    if (!record || busy) return;
    setBusy(true);
    setError('');
    try {
      const { journey, report } = record;
      const payload = {
        formatVersion: 1,
        course: {
          id: report.session.courseId,
          version: report.session.courseVersion,
          packageSha256: report.session.coursePackageSha256,
        },
        sessionId,
        title: journey.title,
        summary: {
          prerequisiteChecks: report.prerequisiteChecks || [],
          completed: report.completedSteps,
          total: report.totalSteps,
          required: report.requiredSteps ?? report.totalSteps,
          requiredCompleted: report.requiredCompletedSteps ?? report.completedSteps,
          optional: report.optionalSteps ?? 0,
          optionalCompleted: report.optionalCompletedSteps ?? 0,
          optionalSkipped: report.optionalSkippedSteps ?? 0,
          software: localReportSoftwareResult(report),
          recordedSoftware: report.recordedSoftwareResult || report.softwareResult,
          currentWorkspace: report.currentWorkspace || {
            status: 'NOT_CHECKED',
            checkedAt: null,
            reference: null,
          },
          hil: report.hilResult,
          recordedHil: report.recordedHilResult ?? report.hilResult,
          recordedClassification:
            report.session.recordedClassification ?? report.session.classification,
          provenance: report.provenanceResult,
          reflectionAssessment: journey.labs.some((lab) => lab.activityKind === 'reflection')
            ? 'NOT_GRADED'
            : 'NOT_APPLICABLE',
        },
        attempts: (includeEvents ? journey.attempts : []).map((attempt) => ({
          stepId: attempt.stepId,
          attempt: attempt.attempt,
          state: attempt.state,
          updatedAt: attempt.updatedAt,
        })),
        answers: (includeNotes ? journey.attempts : []).map((attempt) => ({
          stepId: attempt.stepId,
          attempt: attempt.attempt,
          answer: attempt.answer,
          reflection: attempt.reflection,
        })),
        evidence: includeEvidence ? journey.evidence : [],
        notes: includeNotes ? journey.notes || [] : [],
        savedAt: journey.savedAt,
        aiSummary: 'NOT_GENERATED',
      };
      const text =
        format === 'json'
          ? JSON.stringify(payload, null, 2)
          : `# ${journey.title}\n\n${learnReportTitle(report)}\n\n课程版本：${report.session.courseVersion}\n\n进度：${localReportProgress(report)}；当前软件：${localReportSoftwareResult(report)}；当前 HIL：${report.hilResult}；旧规则记录：${report.recordedHilResult ?? report.hilResult}（历史记录不替代当前实机验证）\n\n## 学习过程\n\n${payload.attempts.map((a) => `- ${a.stepId} · 尝试 ${a.attempt} · ${a.state}`).join('\n')}\n\n## 证据与问题\n\n${payload.evidence.map((e) => `- ${e.stepId} / ${e.attempt} · ${e.source} · ${e.status}：${e.summary}${e.recorded ? `（原记录 ${e.recorded.source} / ${e.recorded.status}）` : ''}\n  - 摘要：${e.digest}`).join('\n')}\n\n## 个人回答与复盘\n\n${payload.answers.map((a) => `- ${a.stepId} · 尝试 ${a.attempt}\n  - 回答：${a.answer}\n  - 复盘：${a.reflection}`).join('\n') || '未选择导出个人回答。'}\n\n## AI 问答记录\n\n${payload.notes.map((note) => `- 问题：${note.question}\n  - 提示：${note.answer}`).join('\n') || '未选择导出问答正文。'}\n\n## 后续建议\n\n${(report.requiredCompletedSteps ?? report.completedSteps) < (report.requiredSteps ?? report.totalSteps) ? '继续当前未完成课程，并复查失败或未验证的证据。' : report.hilRequiredSteps > 0 && report.hilResult !== 'PASS' ? '回顾各课反思与失败记录，优先补充尚未验证的实机项目。' : '回顾各课理解与反思，继续查证尚未解决的问题。'}\n\n理论自述不自动评分。\n\n本档案依据本地事实生成；AI 总结尚未生成。未包含原始终端日志、媒体或订单信息。\n`;
      const exportedText =
        format === 'json'
          ? text
          : text +
            `\n## 当前适用性\n\n历史软件结果：${report.recordedSoftwareResult || report.softwareResult}\n\n${localReportCurrencyText(report)}\n` +
            (report.prerequisiteChecks || [])
              .filter((a) => a.prerequisites.length)
              .map(
                (a) =>
                  `\n- 活动 ${a.stepId} 前置条件：${a.available ? '已具备完成记录' : '未满足'}\n${a.prerequisites.map((r) => `  - ${r.stepId} · ${r.title}：${localPrerequisiteStatus[r.status]}`).join('\n')}\n`
              )
              .join('') +
            (report.currentWorkspace?.sourceChecks || [])
              .map(
                (check) =>
                  `\n- 源码 ${check.stepId} / 尝试 ${check.attempt}：${localSourceStatus[check.status]}；执行证据摘要：${check.digest}\n`
              )
              .join('') +
            (report.currentWorkspace?.environmentChecks || [])
              .map(
                (check) =>
                  `\n- 环境 ${check.stepId} / 尝试 ${check.attempt}：${localEnvironmentStatus[check.status]}；执行证据摘要：${check.digest}\n`
              )
              .join('');
      const path = await save({
        defaultPath: `lynx-learning-${report.session.courseId}.${format}`,
        filters: [{ name: format === 'md' ? 'Markdown' : 'JSON', extensions: [format] }],
      });
      if (path) {
        await writeTextFile(path, exportedText);
        setMessage(`已导出：${path}`);
      }
    } catch (reason) {
      setError(formatErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="learn-journey learn-v2-page learn-records" data-testid="learn-records">
      <LearnBreadcrumb
        items={['学习中心', '学习记录', record?.journey.title || '课程档案', '课程总结']}
      >
        <select
          aria-label="学习会话"
          value={sessionId}
          onChange={(e) => setSessionId(e.target.value)}
        >
          {!sessions.length && <option value="">暂无会话</option>}
          {sessions.map((session) => (
            <option key={session.sessionId} value={session.sessionId}>
              {session.courseId} · {session.courseVersion} · {session.sessionId.slice(-8)}
            </option>
          ))}
        </select>
      </LearnBreadcrumb>
      <div className="learn-v2-body">
        {cloudProgress.length > 0 && (
          <section className="learn-panel" style={{ padding: 20, marginBottom: 20 }}>
            <h2>官网学习记录</h2>
            <p>保留官网同步的历史进度，与本地实验档案分别展示。</p>
            {cloudProgress.map((p) => (
              <div
                className="learn-session-row"
                key={`${p.resourceType}:${p.resourceId}:${p.sessionId}`}
              >
                <strong>{p.title}</strong>
                <span>
                  {p.percent}% · {p.state}
                </span>
                <small>官网记录 · revision {p.revision}</small>
              </div>
            ))}
          </section>
        )}

        {error && (
          <p role="alert" className="learn-notice">
            {error}
          </p>
        )}
        {!record && !error && (
          <p className="learn-empty">
            {sessionId ? '正在读取学习事实…' : '导入签名课程并开始学习后，记录会保存在这里。'}
          </p>
        )}
        {record && (
          <>
            <div className="learn-summary-layout">
              <div className="learn-summary-main learn-panel">
                <header>
                  <h1>{learnReportTitle(record.report)}</h1>
                  <LearnBadge
                    tone={
                      (record.report.requiredCompletedSteps ?? record.report.completedSteps) ===
                      (record.report.requiredSteps ?? record.report.totalSteps)
                        ? 'green'
                        : 'orange'
                    }
                  >
                    {localReportProgress(record.report)}
                  </LearnBadge>
                  <p>记录范围：当前课程版本与学习会话 · v{record.report.session.courseVersion}</p>
                  {record.journey.labs.some((lab) => lab.activityKind === 'reflection') && (
                    <p>
                      理论活动仅记录理解回答与复盘的提交：自述完成，未评分，不代表软件或实机通过。
                    </p>
                  )}
                </header>
                <section className="learn-summary-section">
                  <b>1</b>
                  <div>
                    <h2>我完成了什么</h2>
                    <p>按各课最新一次尝试，回顾当前已记录的学习内容：</p>
                    <ul className="learn-completed-list">
                      {record.journey.labs.map((lab) => {
                        const state = learnLatestAttempt(
                          record.journey.attempts,
                          lab.stepId
                        )?.state;
                        const access = record.report.prerequisiteChecks?.find(
                          (a) => a.stepId === lab.stepId
                        );
                        const complete = state === 'completed' && (access?.completed ?? true);
                        return (
                          <li key={lab.stepId}>
                            <LynxIcon name={complete ? 'check-circle' : 'circle'} size={20} />
                            {lab.title}
                            {lab.required === false ? ' · 选修' : ''}
                            <LearnBadge tone={complete ? 'green' : 'gray'}>
                              {complete
                                ? '已完成'
                                : access?.available === false
                                  ? '前置未满足'
                                  : state === 'skipped'
                                    ? '已跳过（未通过）'
                                    : '待完成'}
                            </LearnBadge>
                          </li>
                        );
                      })}
                    </ul>
                    {!!record.report.prerequisiteChecks?.some((a) => a.prerequisites.length) && (
                      <div data-testid="local-report-prerequisites">
                        {record.report.prerequisiteChecks
                          .filter((a) => a.prerequisites.length)
                          .map((access) => (
                            <section key={access.stepId}>
                              <h3>
                                {record.journey.labs.find((l) => l.stepId === access.stepId)
                                  ?.title || access.stepId}
                              </h3>
                              <LearnPrerequisites
                                access={access}
                                onReview={onResume ? (id) => onResume(sessionId, id) : undefined}
                              />
                            </section>
                          ))}
                      </div>
                    )}
                    <small>
                      当前软件：{localReportSoftwareResult(record.report)} · 当前实机：
                      {record.report.hilResult}
                      {record.report.recordedHilResult &&
                        record.report.recordedHilResult !== record.report.hilResult && (
                          <> · 旧规则 {record.report.recordedHilResult}（实机未验证）</>
                        )}{' '}
                      · 来源：{record.report.provenanceResult}
                    </small>
                    <p className="learn-notice" data-testid="local-report-currency">
                      历史软件：
                      {record.report.recordedSoftwareResult || record.report.softwareResult}。{' '}
                      {localReportCurrencyText(record.report)}
                    </p>
                    {!!record.report.currentWorkspace?.environmentChecks?.length && (
                      <div data-testid="local-report-environments">
                        <h3>必修实践的环境</h3>
                        <ul>
                          {record.report.currentWorkspace.environmentChecks.map((check) => (
                            <li key={check.stepId}>
                              {record.journey.labs.find((lab) => lab.stepId === check.stepId)
                                ?.title || check.stepId}
                              {' · '}尝试 {check.attempt}
                              {' · '}
                              {localEnvironmentStatus[check.status]}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                    {!!record.report.currentWorkspace?.sourceChecks?.length && (
                      <div data-testid="local-report-sources">
                        <strong>
                          {record.report.prerequisiteChecks?.length
                            ? '必修及其前置实践的源码'
                            : '重学后各必修实践的源码'}
                        </strong>
                        <ul>
                          {record.report.currentWorkspace.sourceChecks.map((check) => (
                            <li key={check.stepId}>
                              {record.journey.labs.find((lab) => lab.stepId === check.stepId)
                                ?.title || check.stepId}{' '}
                              · 尝试 {check.attempt} · {localSourceStatus[check.status]}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </div>
                </section>
                <section className="learn-summary-section">
                  <b>2</b>
                  <div>
                    <h2>问题与解决过程</h2>
                    <p>保留每次尝试的现象、记录来源与结果。</p>
                    <table className="learn-table">
                      <thead>
                        <tr>
                          <th>课时 / 尝试</th>
                          <th>事实与处理记录</th>
                          <th>结果</th>
                        </tr>
                      </thead>
                      <tbody>
                        {record.journey.evidence.slice(0, 8).map((e, i) => (
                          <tr key={i}>
                            <td>
                              {e.stepId} / {e.attempt}
                            </td>
                            <td>
                              <details>
                                <summary>{e.summary}</summary>
                                <code>{e.digest}</code>
                                <small>
                                  {e.source} · {new Date(e.createdAt * 1000).toLocaleString()}
                                </small>
                              </details>
                            </td>
                            <td>
                              <LearnBadge tone={e.status === 'PASS' ? 'green' : 'orange'}>
                                {e.status}
                              </LearnBadge>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                    {!record.journey.evidence.length && (
                      <p className="learn-notice">暂无执行证据，阅读进度不等于实验通过。</p>
                    )}
                    {(record.journey.notes || []).map((note, i) => (
                      <details className="learn-notice" key={i}>
                        <summary>{note.question}</summary>
                        <p>{note.answer}</p>
                        <small>{note.stepId} · AI 提示，未自动判定问题已解决</small>
                      </details>
                    ))}
                    <small>页面显示最近 8 条；完整导出最多包含最近 1000 条证据。</small>
                  </div>
                </section>
                <section className="learn-summary-section">
                  <b>3</b>
                  <div>
                    <h2>学习建议</h2>
                    <p>基于当前记录，安排下一步学习：</p>
                    <ol className="learn-recommendations">
                      <li>
                        <strong>
                          {(record.report.requiredCompletedSteps ?? record.report.completedSteps) <
                          (record.report.requiredSteps ?? record.report.totalSteps)
                            ? '继续完成当前学习任务'
                            : '回顾各课反思与验证依据'}
                        </strong>
                        <p>区分软件通过、实机验证与理解回答，补齐仍未满足的条件。</p>
                      </li>
                      <li>
                        <strong>整理版本与排查方法</strong>
                        <p>复查错误现象、修改记录与最终证据，把过程整理为可复用笔记。</p>
                      </li>
                      <li>
                        <strong>选择下一项实践</strong>
                        <p>返回学习指南选择适合的课程，课程权益以账号实际结果为准。</p>
                      </li>
                    </ol>
                    {onResume && (
                      <button className="learning-secondary" onClick={() => onResume(sessionId)}>
                        回到当前学习位置 →
                      </button>
                    )}
                  </div>
                </section>
              </div>
              <aside className="learn-guide-detail learn-export-panel">
                <h2>
                  <LynxIcon name="document" size={24} />
                  自动生成的学习档案
                </h2>
                <p>事实已保存到本地 · 可预览或导出</p>
                <label className="learn-export-item">
                  <LynxIcon name="document" size={24} />
                  <div>
                    <strong>学习记录</strong>
                    <small>学习过程的时间线与关键操作</small>
                  </div>
                  <input
                    type="checkbox"
                    checked={includeEvents}
                    onChange={(e) => setIncludeEvents(e.target.checked)}
                  />
                </label>
                <label className="learn-export-item">
                  <LynxIcon name="document" size={24} />
                  <div>
                    <strong>问题与证据</strong>
                    <small>本次尝试与实际验证结果</small>
                  </div>
                  <input
                    type="checkbox"
                    checked={includeEvidence}
                    onChange={(e) => setIncludeEvidence(e.target.checked)}
                  />
                </label>
                <div className="learn-export-item">
                  <LynxIcon name="document" size={24} />
                  <div>
                    <strong>课程总结与下一步建议</strong>
                    <small>基于学习事实 · 始终包含</small>
                  </div>
                  <LynxIcon name="check-circle" size={20} />
                </div>
                <label className="learn-export-item">
                  <LynxIcon name="document" size={24} />
                  <div>
                    <strong>个人回答与 AI 问答</strong>
                    <small>可选 · 导出前检查笔记隐私</small>
                  </div>
                  <input
                    type="checkbox"
                    checked={includeNotes}
                    onChange={(e) => setIncludeNotes(e.target.checked)}
                  />
                </label>
                <h3>导出前可预览范围</h3>
                <p>所选内容合并为一份档案：</p>
                <div className="learn-export-formats">
                  <label>
                    <input
                      type="radio"
                      name="learn-format"
                      checked={format === 'md'}
                      onChange={() => setFormat('md')}
                    />
                    Markdown（推荐）
                  </label>
                  <label>
                    <input
                      type="radio"
                      name="learn-format"
                      checked={format === 'json'}
                      onChange={() => setFormat('json')}
                    />
                    JSON
                  </label>
                </div>
                <button disabled={busy} onClick={() => void exportRecord()}>
                  <LynxIcon name="download" size={20} />
                  {busy ? '导出中…' : '导出学习档案'}
                </button>
                <button className="learning-secondary" onClick={() => setShowPreview((v) => !v)}>
                  查看导出范围
                </button>
                {showPreview && (
                  <div className="learn-notice">
                    当前课程与摘要；{includeEvents ? record.journey.attempts.length : 0}{' '}
                    次学习尝试；{includeEvidence ? record.journey.evidence.length : 0} 条证据；
                    {includeNotes ? '包含个人回答与 AI 问答' : '不包含笔记正文'}。
                  </div>
                )}
                <p className="learn-notice">
                  AI
                  综合总结尚未生成。原始事实可离线查看；导出不包含订单原文、凭据、完整日志或媒体。
                </p>
                {message && <p role="status">{message}</p>}
              </aside>
            </div>
            <nav className="learn-chapter-review" aria-label="章节回顾">
              <h2>
                <LynxIcon name="book" size={24} />
                章节回顾
              </h2>
              {record.journey.labs.map((lab) => (
                <button
                  key={lab.stepId}
                  className="learning-secondary"
                  disabled={!onResume}
                  onClick={() => onResume?.(sessionId, lab.stepId)}
                >
                  <strong>
                    {String(lab.number).padStart(2, '0')} {lab.title}
                  </strong>
                  <small>
                    {record.report.prerequisiteChecks?.find((a) => a.stepId === lab.stepId)
                      ?.available === false
                      ? '前置未满足 · 查看课程卡点'
                      : learnLatestAttempt(record.journey.attempts, lab.stepId)?.state ===
                            'completed' &&
                          (record.report.prerequisiteChecks?.find((a) => a.stepId === lab.stepId)
                            ?.completed ??
                            true)
                        ? '已完成 · 回到会话查看'
                        : learnLatestAttempt(record.journey.attempts, lab.stepId)?.state ===
                            'skipped'
                          ? '已跳过（未通过） · 回到会话查看'
                          : '待完成 · 回到学习位置'}
                  </small>
                </button>
              ))}
            </nav>
          </>
        )}
      </div>
      <LearnFooter>原始记录已保留 · AI 总结状态单独显示</LearnFooter>
    </section>
  );
}
