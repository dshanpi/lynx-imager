import { useCallback, useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type { LearnVerifiedPreview, LearnPreviewReadReceipt } from '../../../Platform/IPC/Commands';
import { learnServiceError } from './serviceStatus';
import { LearnGenericWorkspace } from './LearnGenericWorkspace';
import './LearnPreviewEntry.css';

export function LearnPreviewEntry({
  active,
  previewId,
  onBack,
  autoOpen = false,
}: {
  active: boolean;
  previewId: string;
  onBack: () => void;
  autoOpen?: boolean;
}) {
  const [preview, setPreview] = useState<LearnVerifiedPreview | null>(null);
  const [receipt, setReceipt] = useState<LearnPreviewReadReceipt | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const locked = useRef(false);
  const requestId = useRef('');
  useEffect(() => {
    generation.current++;
    locked.current = false;
    requestId.current = '';
    setBusy(false);
    setPreview(null);
    setReceipt(null);
    setError('');
    return () => {
      generation.current++;
      locked.current = false;
    };
  }, [active, previewId]);
  useEffect(() => {
    if (!preview) return;
    const timer = setTimeout(
      () => {
        generation.current++;
        locked.current = false;
        setBusy(false);
        setPreview(null);
        setReceipt(null);
        setError('试学邀请已过期，请在官网重新生成。');
      },
      Math.max(0, Math.min(2147483647, Date.parse(preview.binding.expiresAt) - Date.now()))
    );
    return () => clearTimeout(timer);
  }, [preview]);

  const open = useCallback(async () => {
    if (!active || locked.current) return;
    locked.current = true;
    const current = ++generation.current;
    setBusy(true);
    setPreview(null);
    setReceipt(null);
    setError('');
    requestId.current = '';
    try {
      const next = await invokeCommand('learn_get_preview', { previewId });
      if (current === generation.current) setPreview(next);
    } catch (e) {
      if (current === generation.current) setError(learnServiceError(e));
    } finally {
      if (current === generation.current) {
        setBusy(false);
        locked.current = false;
      }
    }
  }, [active, previewId]);
  useEffect(() => {
    if (autoOpen) void open();
  }, [autoOpen, open]);
  const check = async () => {
    if (!active || !preview || locked.current) return;
    locked.current = true;
    const current = ++generation.current;
    requestId.current ||= crypto.randomUUID();
    setBusy(true);
    setReceipt(null);
    setError('');
    try {
      const next = await invokeCommand('learn_check_preview_read', {
        previewId,
        contentDigest: preview.binding.contentDigest,
        requestId: requestId.current,
      });
      if (current === generation.current) setReceipt(next);
    } catch (e) {
      if (current === generation.current) setError(learnServiceError(e));
    } finally {
      if (current === generation.current) {
        setBusy(false);
        locked.current = false;
      }
    }
  };
  if (preview)
    return (
      <LearnGenericWorkspace
        active={active}
        preview={preview}
        onBack={onBack}
        onRefresh={() => void open()}
      >
        <section className="learn-preview-check" aria-label="官网试学联调">
          <div>
            <strong>官网试学 · 已核验签名</strong>
            <small className="learn-preview-scope">测试预览 · 不计入正式成绩</small>
          </div>
          <details className="learn-preview-details">
            <summary>试学说明</summary>
            <p>指定账号预览 · 有效至 {new Date(preview.binding.expiresAt).toLocaleString()}</p>
            <p>联调仅核验内容读取与服务器资源，不计入课程成绩，也不代替构建、部署和板卡验证。</p>
          </details>
          <button
            type="button"
            className="learning-secondary"
            disabled={!active || busy}
            onClick={() => void check()}
          >
            {busy
              ? '正在核验全部教材并回传…'
              : receipt
                ? '重新核对联调回执'
                : '检查教材并回传联调结果'}
          </button>
          {receipt && (
            <p role="status">
              教材联调回执已保存 · {receipt.documents.length} 份单元教材 · 实操验收尚未完成。
              <br />
              <small>回执编号：{receipt.receiptId}</small>
            </p>
          )}
          {error && <p role="alert">{error}</p>}
        </section>
      </LearnGenericWorkspace>
    );
  return (
    <div
      className="learn-journey learn-v2-page learn-preview-entry"
      data-testid="learn-preview-entry"
    >
      <div className="learn-breadcrumb">
        <button type="button" onClick={onBack}>
          ← 返回我的课程
        </button>
        <span>/ 官网课程试学</span>
      </div>
      <header className="learn-workspace-topbar">
        <h2>官网课程试学</h2>
        <span className="learn-chip">等待核验邀请</span>
      </header>
      <section className="learn-preview-card" aria-labelledby="learn-preview-title">
        <h3 id="learn-preview-title">已接收试学邀请</h3>
        <p>登录已完成。打开后会核验你的访问范围、课程签名、草稿版本和有效期。</p>
        <ol className="learn-preview-steps">
          <li>
            <strong>1 · 接收链接</strong>
            <span>定位当前预览</span>
          </li>
          <li>
            <strong>2 · 登录官网</strong>
            <span>核对指定试学账号</span>
          </li>
          <li>
            <strong>3 · 阅读课程</strong>
            <span>签名目录与教材逐节核验</span>
          </li>
          <li>
            <strong>4 · 回传联调</strong>
            <span>保存有明确验收范围的回执</span>
          </li>
        </ol>
        <p>本次试学不创建正式报名或成绩。课程实操仍须独立验证。</p>
        <details>
          <summary>联调信息</summary>
          <p>
            预览编号：<code>{previewId}</code>
          </p>
          <p>此编号用于定位预览，访问权限由官网核验。</p>
        </details>
        <div className="learn-preview-actions">
          <button
            type="button"
            className="learn-primary"
            disabled={!active || busy}
            onClick={() => void open()}
          >
            {busy ? '正在核验并打开课程…' : '打开签名课程预览'}
          </button>
          <button type="button" onClick={onBack}>
            返回我的课程
          </button>
        </div>
        {error && <p role="alert">{error}</p>}
      </section>
    </div>
  );
}
