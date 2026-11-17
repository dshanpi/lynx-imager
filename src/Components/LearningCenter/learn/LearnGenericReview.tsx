import { useEffect, useRef, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LearnCoursePin,
  LearnCourseUnit,
  LearnLearningSummary,
  LearnAssessmentResult,
  LearnCourseReport,
  LearnReviewRequest,
} from '../../../Platform/IPC/Commands';
import { PendingWriteNotice, useWriteRecovery } from './writeRecovery';
import { reviewDraftKey, readReviewDraft, writeReviewDraft } from './reviewDraft';
import { learnServiceError } from './serviceStatus';
import {
  reportTaskKey,
  readReportTask,
  writeReportTask,
  validateReportJob,
  type LearnReportTask,
} from './reportRecovery';
const labels: Record<string, string> = {
  passed: '规则已通过',
  failed: '评测未通过',
  needs_evidence: '待补证据',
  running: '进行中',
  created: '尚未评测',
  cancelled: '已取消',
  not_started: '尚未开始',
};
interface ReviewProps {
  accountId: number;
  active: boolean;
  pin: LearnCoursePin;
  chapterId: string;
  unit: LearnCourseUnit;
}
export function LearnGenericReview(props: ReviewProps) {
  return (
    <ReviewPanel
      key={JSON.stringify([
        props.accountId,
        props.pin.enrollmentId,
        props.pin.releaseId,
        props.pin.manifestSha256,
        props.chapterId,
        props.unit.unitId,
      ])}
      {...props}
    />
  );
}
function ReviewPanel({ accountId, active, pin, chapterId, unit }: ReviewProps) {
  const [summary, setSummary] = useState<LearnLearningSummary | null>(null);
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [reflection, setReflection] = useState('');
  const [assessment, setAssessment] = useState<LearnAssessmentResult | null>(null);
  const [report, setReport] = useState<LearnCourseReport | null>(null);
  const [reportTask, setReportTask] = useState<LearnReportTask | null>(null);
  const [reportState, setReportState] = useState<'pending' | 'complete' | 'failed'>('pending');
  const [recoveryReady, setRecoveryReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [reload, setReload] = useState(0);
  const sequence = useRef(0);
  const {
    load: loadWriteRecovery,
    reset: resetWriteRecovery,
    ...writeRecovery
  } = useWriteRecovery<LearnReviewRequest>({ accountId, pin, unitId: unit.unitId, kind: 'review' });
  const { enrollmentId, releaseId, manifestSha256 } = pin;
  const recoveryKey = reportTaskKey(pin, chapterId);
  useEffect(() => {
    const seq = ++sequence.current;
    resetWriteRecovery();
    setSummary(null);
    setAssessment(null);
    setReport(null);
    setNotice('');
    setBusy(false);
    setRecoveryReady(false);
    if (!active) return;
    setBusy(true);
    setError('');
    void invokeCommand('learn_get_learning_summary', {
      request: { enrollmentId, releaseId, manifestSha256 },
    })
      .then((v) => {
        if (seq !== sequence.current) return;
        loadWriteRecovery();
        setSummary(v);
        const task = readReportTask(
          localStorage,
          recoveryKey,
          { enrollmentId, releaseId, manifestSha256 },
          chapterId
        );
        setReportTask(task);
        setReportState('pending');
        setRecoveryReady(true);
        if (task) setNotice('已恢复本章报告请求，可查询原任务；不会自动创建新报告。');
      })
      .catch((e) => {
        if (seq === sequence.current) setError(learnServiceError(e));
      })
      .finally(() => {
        if (seq === sequence.current) setBusy(false);
      });
    return () => {
      sequence.current++;
    };
  }, [
    active,
    enrollmentId,
    releaseId,
    manifestSha256,
    reload,
    recoveryKey,
    chapterId,
    loadWriteRecovery,
    resetWriteRecovery,
  ]);
  const restored = useRef('');
  const [draftNotice, setDraftNotice] = useState('');
  const draftKey = reviewDraftKey(enrollmentId, releaseId, manifestSha256, unit.unitId);
  useEffect(() => {
    if (!active || !summary || restored.current === draftKey) return;
    restored.current = draftKey;
    try {
      const draft = readReviewDraft(localStorage, draftKey);
      if (draft) {
        setAnswers(draft.answers);
        setReflection(draft.reflection);
        setDraftNotice('已恢复本机输入，请核对同步结果后再提交。');
      }
    } catch {
      setDraftNotice('本机草稿读取失败，未覆盖已有草稿；请保留当前输入。');
    }
  }, [active, summary, draftKey]);
  function changeDraft(nextAnswers: Record<string, string>, nextReflection: string) {
    setAnswers(nextAnswers);
    setReflection(nextReflection);
    if (!active || !summary) return;
    try {
      writeReviewDraft(localStorage, draftKey, {
        answers: nextAnswers,
        reflection: nextReflection,
      });
      setDraftNotice('草稿已保存到本机；提交后才同步到官网。');
    } catch {
      setDraftNotice('草稿无法写入本机，离开前请复制保存输入。');
    }
  }
  const current = summary?.units.find((u) => u.unitId === unit.unitId);
  function ruleTitles(ids: string[]) {
    return ids
      .map(
        (id, index) =>
          unit.activities
            .filter((a) => a.ruleRefs.includes(id))
            .map((a) => a.title)
            .join('、') || `第 ${index + 1} 项验收`
      )
      .join('；');
  }
  async function submit() {
    if (!active || busy || !summary || !writeRecovery.ready || writeRecovery.pending) return;
    const body = {
      ...pin,
      expectedRevision: summary.revision,
      unitId: unit.unitId,
      attemptId:
        current && !['passed', 'failed', 'cancelled', 'not_started'].includes(current.state)
          ? current.currentAttemptId
          : '',
      answers: Object.fromEntries(
        unit.activities
          .filter((a) => a.completionMode === 'quiz')
          .map((a) => [a.activityId, answers[a.activityId] || ''])
      ),
      reflection,
    };
    await sendReview({ ...body, requestId: crypto.randomUUID() });
  }
  async function sendReview(request: LearnReviewRequest) {
    if (!active || busy || !summary || !writeRecovery.ready) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    setNotice('');
    const replaying = !!writeRecovery.pending;
    let began = false;
    try {
      if (!replaying) {
        const gate = await invokeCommand('learn_get_practice_review_gate', {
          request: { ...pin, unitId: unit.unitId },
        });
        if (seq !== sequence.current) return;
        if (gate.pending) throw { machineCode: 'LEARN_SYNC_PENDING' };
        if (gate.attemptId && gate.attemptId !== request.attemptId)
          throw { machineCode: 'LEARN_SYNC_ATTEMPT_MISMATCH' };
      }
      writeRecovery.begin(request);
      began = true;
      const v = await invokeCommand('learn_submit_review', {
        request,
      });
      if (seq !== sequence.current) return;
      writeRecovery.complete(request);
      setSummary(v.summary);
      setAssessment(v.assessment);
      setReport(null);
      setDraftNotice('本次请求已确认；本机输入仍保留，之后修改的内容需另行提交。');
      setNotice('本次评测已保存。阅读、理解题和实操证据分别判定。');
    } catch (e) {
      if (seq === sequence.current) {
        if (
          typeof e === 'object' &&
          e !== null &&
          'machineCode' in e &&
          began &&
          (e.machineCode === 'LEARN_REVIEW_CONFLICT' ||
            (!replaying &&
              [
                'LEARN_SYNC_PENDING',
                'LEARN_SYNC_ATTEMPT_MISMATCH',
                'LEARN_EVIDENCE_STALE',
                'LEARN_EVIDENCE_CHECK_UNAVAILABLE',
              ].includes(String(e.machineCode))))
        ) {
          try {
            writeRecovery.complete(request);
          } catch (storageFailure) {
            setError(learnServiceError(storageFailure));
            setSummary(null);
            return;
          }
        }
        setError(learnServiceError(e));
        setSummary(null);
        setAssessment(null);
        setReport(null);
      }
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  async function generate(scope: 'chapter' | 'course') {
    if (!active || busy || !summary || !recoveryReady || (reportTask && reportState === 'pending'))
      return;
    const task: LearnReportTask = {
      ...pin,
      scope,
      chapterId: scope === 'chapter' ? chapterId : '',
      inputRevision: summary.revision,
      requestId: crypto.randomUUID(),
      jobId: '',
    };
    await continueReport(task);
  }
  async function continueReport(task: LearnReportTask) {
    if (!active || busy || !summary || !recoveryReady) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    setNotice('');
    setReport(null);
    setReportState('pending');
    try {
      // Keep the original idempotency key even if POST succeeded but its response was lost.
      writeReportTask(localStorage, recoveryKey, task);
      setReportTask(task);
      let saved = task;
      if (!saved.jobId) {
        const { jobId: _jobId, ...request } = saved;
        const job = await invokeCommand('learn_request_report', { request });
        if (seq !== sequence.current) return;
        saved = { ...saved, jobId: job.jobId };
        setReportTask(saved);
        writeReportTask(localStorage, recoveryKey, saved);
      }
      for (let i = 0; i < 15; i++) {
        if (seq !== sequence.current) return;
        const value = await invokeCommand('learn_get_report_job', {
          request: pin,
          jobId: saved.jobId,
        });
        if (seq !== sequence.current) return;
        validateReportJob(saved, value);
        if (value.status === 'failed' || value.status === 'cancelled') {
          setReportState('failed');
          throw {
            machineCode:
              value.status === 'failed' ? 'LEARN_REPORT_FAILED' : 'LEARN_REPORT_CANCELLED',
          };
        }
        if (value.report) {
          setReport(value.report);
          setReportState('complete');
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 2000));
      }
      if (seq === sequence.current) setNotice('报告仍在生成；稍后点击“查询原报告任务”继续。');
    } catch (e) {
      if (seq === sequence.current) setError(learnServiceError(e));
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  async function exportReport(format: 'json' | 'md') {
    if (!active || !report || busy) return;
    const seq = sequence.current;
    setBusy(true);
    try {
      const file = await save({
        defaultPath: `learning-report-${report.reportId}.${format}`,
        filters: [{ name: format === 'json' ? 'JSON' : 'Markdown', extensions: [format] }],
      });
      if (file && active && seq === sequence.current) {
        const text =
          format === 'json'
            ? JSON.stringify(report, null, 2)
            : `# ${report.scope === 'chapter' ? '章节' : '整课'}学习报告\n\n报告编号：${report.reportId}\n\n课程版本：${report.releaseId}\n\n章节：${report.chapterId || '整课'}\n\n输入记录修订号：${report.inputRevision}\n\n查询时记录修订号：${summary?.revision ?? '未核验'}\n\n${report.inputRevision === summary?.revision ? '对应本次已同步的记录快照；不保证之后没有新变化。' : '历史报告：不代表当前评测结果，请同步后重新生成。'}\n\n${report.units.map((u) => `- ${u.title}：${labels[u.state] || u.state}${u.required ? '（必修）' : '（选修）'}`).join('\n')}\n\n## 学习复盘\n\n${report.reflections.map((v) => v.text).join('\n\n')}\n\n## 验证边界\n\n${report.unverified.join('\n')}\n`;
        await writeTextFile(file, text);
        if (seq === sequence.current) setNotice('报告已导出。');
      }
    } catch (e) {
      if (seq === sequence.current) setError(learnServiceError(e));
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  return (
    <section className="learn-generic-environment" aria-label="章节评测与报告">
      <h3>章节评测与报告</h3>
      <p>
        本节：
        {current
          ? labels[current.state] || current.state
          : busy
            ? '正在读取'
            : '尚未核验，请同步评测结果'}
        。成绩按课程规则和当前有效证据计算。
      </p>
      {error && (
        <p role="alert" className="learn-notice">
          {error} 未提交输入仍保留。
        </p>
      )}
      {notice && <p role="status">{notice}</p>}
      {draftNotice && <p role="status">{draftNotice}</p>}
      <PendingWriteNotice
        request={writeRecovery.pending}
        disabled={!active || busy || !summary || !writeRecovery.ready}
        retry={() => {
          if (writeRecovery.pending) void sendReview(writeRecovery.pending);
        }}
      />
      {unit.activities
        .filter((a) => a.completionMode === 'quiz')
        .map((a) => (
          <label key={a.activityId}>
            {a.title}
            <textarea
              aria-label={a.title}
              value={answers[a.activityId] || ''}
              disabled={!active || busy || !summary}
              maxLength={4096}
              onChange={(e) =>
                changeDraft({ ...answers, [a.activityId]: e.target.value }, reflection)
              }
            />
          </label>
        ))}
      <label>
        本次复盘
        <textarea
          aria-label="本次复盘"
          disabled={!active || busy || !summary}
          maxLength={4096}
          value={reflection}
          onChange={(e) => changeDraft(answers, e.target.value)}
          placeholder="解释本次结果和待解决问题"
        />
      </label>
      <div className="learn-hub-actions">
        <button
          className="learning-primary"
          disabled={!active || busy || !summary || !writeRecovery.ready || !!writeRecovery.pending}
          onClick={() => void submit()}
        >
          {current && ['passed', 'failed'].includes(current.state)
            ? '提交新一次评测'
            : '提交本节评测'}
        </button>
        <button
          className="learning-secondary"
          disabled={!active || busy}
          onClick={() => setReload((v) => v + 1)}
        >
          同步评测结果
        </button>
      </div>
      {assessment && (
        <>
          <p>{labels[assessment.status] || assessment.status}</p>
          {assessment.missingEvidence.length > 0 && (
            <p>待补证据：{ruleTitles(assessment.missingEvidence)}</p>
          )}
          {assessment.failedRules.length > 0 && (
            <p>未通过规则：{ruleTitles(assessment.failedRules)}</p>
          )}
        </>
      )}
      <div className="learn-hub-actions">
        <button
          className="learning-secondary"
          disabled={
            !active ||
            busy ||
            !summary ||
            !recoveryReady ||
            !!(reportTask && reportState === 'pending')
          }
          onClick={() => void generate('chapter')}
        >
          生成章节报告
        </button>
        <button
          className="learning-secondary"
          disabled={
            !active ||
            busy ||
            !summary ||
            !recoveryReady ||
            !!(reportTask && reportState === 'pending')
          }
          onClick={() => void generate('course')}
        >
          生成整课报告
        </button>
      </div>
      {reportTask && (
        <div className="learn-hub-actions">
          <p>
            {reportTask.scope === 'chapter' ? '章节' : '整课'}报告 · 记录修订{' '}
            {reportTask.inputRevision} ·{' '}
            {reportState === 'pending'
              ? '待查询确认'
              : reportState === 'failed'
                ? '生成未完成'
                : '已获取'}
          </p>
          <button
            className="learning-secondary"
            disabled={!active || busy || !summary || !recoveryReady}
            onClick={() => void continueReport(reportTask)}
          >
            查询原报告任务
          </button>
        </div>
      )}
      {report && (
        <>
          <p>
            报告记录修订：{report.inputRevision} · 本次已同步修订：{summary?.revision ?? '未核验'}
          </p>
          <p>
            {report.inputRevision !== summary?.revision
              ? '历史报告：不代表当前评测结果，请同步后重新生成。'
              : report.allRequiredPassed
                ? '当前必修规则已满足'
                : '还有必修内容待完成或验证'}
          </p>
          <ul>
            {report.units.map((u) => (
              <li key={u.unitId}>
                {u.title} · {labels[u.state] || u.state}
              </li>
            ))}
          </ul>
          {report.unverified.map((v) => (
            <small key={v}>{v}</small>
          ))}
          <div className="learn-hub-actions">
            <button disabled={!active || busy} onClick={() => void exportReport('md')}>
              导出可读报告
            </button>
            <button disabled={!active || busy} onClick={() => void exportReport('json')}>
              导出 JSON
            </button>
          </div>
        </>
      )}
    </section>
  );
}
