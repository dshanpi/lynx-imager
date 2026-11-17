import { useEffect, useRef, useState } from 'react';
import type {
  LearnJourney,
  LocalLearningStepView,
  LocalCommandDiagnostic,
} from '../../../Platform/IPC/Commands';
import { invokeCommand } from '../../../Platform/IPC/Client';
import { formatErrorMessage } from '../../../Utils';
import { LynxIcon } from '../../../CoreUI';
import { LocalCommandDiagnostics } from './LocalCommandDiagnostics';
import { LocalArtifacts } from './LocalArtifacts';

export function LearnEvidence({
  step,
  journey,
  active,
  onSaved,
  onSelectDiagnostic,
}: {
  active: boolean;
  onSaved: () => Promise<void>;
  step: LocalLearningStepView;
  journey: LearnJourney | null;
  onSelectDiagnostic?: (value: LocalCommandDiagnostic) => void;
}) {
  const reflectionActivity = step.lab.activityKind === 'reflection';
  const stream = useRef<MediaStream | null>(null);
  const video = useRef<HTMLVideoElement>(null);
  const captureGeneration = useRef(0);
  const [preview, setPreview] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [photo, setPhoto] = useState('');
  const stop = () => {
    captureGeneration.current++;
    stream.current?.getTracks().forEach((track) => track.stop());
    stream.current = null;
    setPreview(false);
  };
  useEffect(
    () => () => {
      captureGeneration.current++;
      stream.current?.getTracks().forEach((track) => track.stop());
    },
    []
  );
  useEffect(() => {
    if (!active || reflectionActivity) {
      captureGeneration.current++;
      stream.current?.getTracks().forEach((track) => track.stop());
      stream.current = null;
      setPreview(false);
    }
  }, [active, reflectionActivity]);
  const enable = async () => {
    if (reflectionActivity || busy || stream.current) return;
    const generation = ++captureGeneration.current;
    setBusy(true);
    setMessage('');
    try {
      if (!navigator.mediaDevices?.getUserMedia)
        throw new Error('当前桌面 WebView 未提供摄像头权限接口。请使用已有板端证据流程。');
      const acquired = await navigator.mediaDevices.getUserMedia({
        video: { width: { ideal: 1280 }, height: { ideal: 720 } },
        audio: false,
      });
      if (generation !== captureGeneration.current) {
        acquired.getTracks().forEach((track) => track.stop());
        return;
      }
      stream.current = acquired;
      setPreview(true);
      if (video.current) {
        video.current.srcObject = acquired;
        await video.current.play();
      }
    } catch (reason) {
      stream.current?.getTracks().forEach((track) => track.stop());
      stream.current = null;
      setPreview(false);
      setMessage(formatErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  };
  const capture = async () => {
    if (!video.current || !journey || busy) return;
    setBusy(true);
    setMessage('');
    try {
      if (!video.current.videoWidth) throw new Error('摄像头尚未输出画面，请稍后重试。');
      const canvas = document.createElement('canvas');
      const scale = Math.min(1, 1280 / video.current.videoWidth);
      canvas.width = Math.round(video.current.videoWidth * scale);
      canvas.height = Math.round(video.current.videoHeight * scale);
      const context = canvas.getContext('2d');
      if (!context) throw new Error('无法读取摄像头画面。');
      context.drawImage(video.current, 0, 0, canvas.width, canvas.height);
      const data = canvas.toDataURL('image/png');
      const digest = await invokeCommand('learning_attach_photo', {
        sessionId: step.session.sessionId,
        stepId: step.session.currentStep,
        attempt: journey.attempt,
        pngBase64: data.split(',')[1],
      });
      setPhoto(data);
      stop();
      await onSaved();
      setMessage(
        `已保存 1 张本机照片，摘要 ${digest}。来源为用户拍摄，尚未做视觉判断，不计入课程通过条件。`
      );
    } catch (reason) {
      setMessage(formatErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  };
  const evidence =
    journey?.evidence.filter(
      (item) => item.stepId === step.session.currentStep && item.attempt === journey.attempt
    ) || [];
  return (
    <section className="learn-evidence" data-testid="learn-evidence">
      <span className="learn-eyebrow">OBSERVE · 当前尝试的真实记录</span>
      <h1>{reflectionActivity ? '本活动的学习记录' : '观察运行现象，核对课程规则'}</h1>
      {reflectionActivity ? (
        <p>本活动只记录理解回答与复盘；无需执行目标，不采集设备或摄像头证据。</p>
      ) : (
        <p>
          执行目标：工程 #{step.session.projectId} · {step.session.boardUid || '未绑定开发板'} ·
          尝试 {journey?.attempt ?? '—'}
        </p>
      )}
      <div className="learn-notice">
        <strong>本课期待看到什么？</strong>
        <p>{step.lab.expectedObservation}</p>
        {!!step.lab.artifactOutputs?.length && (
          <p>
            本课校验成功后，将按课程声明保存这些产物到本机：
            {step.lab.artifactOutputs.map((item) => `${item.title}（${item.path}）`).join('、')}。
            保存的是执行时快照，不自动上传或发送给 AI。
          </p>
        )}
      </div>
      {!reflectionActivity && (
        <div className="learn-observation-grid">
          <section className="learn-camera-column">
            <div className="learn-section-title">
              <h2>实验观察摄像头</h2>
              <span>单次拍照 · 不录音 · 不发送给 AI</span>
            </div>
            <div className="learn-camera">
              <video ref={video} hidden={!preview} muted playsInline />
              <div hidden={preview}>
                {photo ? (
                  <img src={photo} alt="本次拍摄的原始观察照片，尚未验证" />
                ) : (
                  <>
                    <LynxIcon name="board" size={40} />
                    <p>先将目标放入画面，再明确启用摄像头。</p>
                    <small>此照片只作为补充观察，不能代替板端日志与确定性验收。</small>
                  </>
                )}
              </div>
            </div>
            <div className="learn-button-row">
              <button
                className="learning-secondary"
                disabled={busy || preview}
                onClick={() => void enable()}
              >
                授权并预览本机摄像头
              </button>
              <button disabled={!preview || busy} onClick={() => void capture()}>
                拍摄 1 张并停止
              </button>
              <button className="learning-secondary" disabled={!preview && !busy} onClick={stop}>
                停止预览
              </button>
            </div>
            {message && (
              <p className="learn-notice" role="status">
                {message}
              </p>
            )}
            <small>仅手动拍照。自动触发、区域裁剪、录像及视觉分析尚未接入。</small>
          </section>
          <section className="learn-runtime-column">
            <h3>
              <LynxIcon name="terminal" size={20} />
              本次执行记录
            </h3>
            <pre className="learn-log-output">
              {evidence
                .filter((item) => item.kind === 'command' || item.source === 'software')
                .map((item) => `[${item.status}] ${item.summary}`)
                .join('\n\n') ||
                '本次尚未采集执行记录。\n\n完成理解检查后，按课程规则执行并观察结果。\n\n这里只显示证据摘要，不是实时串口终端。'}
            </pre>
            <small>摘要来自当前课时与本次尝试；在下方命令记录中打开已保存的诊断。</small>
          </section>
        </div>
      )}
      <div className="learn-section-title">
        <h2>{reflectionActivity ? '已提交的自述记录（未评分）' : '证据采集结果（本次运行）'}</h2>
        <span>{evidence.length} 条 · 历史尝试保留在学习档案</span>
      </div>
      <div className="learn-table-wrap">
        <table className="learn-table">
          <thead>
            <tr>
              <th>来源</th>
              <th>观察 / 结果</th>
              <th>状态</th>
            </tr>
          </thead>
          <tbody>
            {evidence.map((item, i) => (
              <tr key={`${item.digest}-${i}`}>
                <td>
                  {item.source}
                  <small>{item.kind}</small>
                </td>
                <td>
                  {item.summary}
                  <details>
                    <summary>查看摘要与时间</summary>
                    <code>{item.digest}</code>
                    <p>{new Date(item.createdAt * 1000).toLocaleString()}</p>
                  </details>
                  {item.kind === 'command' && item.source === 'software' && journey && (
                    <LocalCommandDiagnostics
                      active={active}
                      onSelectForTutor={onSelectDiagnostic}
                      scope={{
                        sessionId: step.session.sessionId,
                        stepId: item.stepId,
                        attempt: item.attempt,
                        digest: item.digest,
                      }}
                    />
                  )}
                  {item.kind === 'command' && item.source === 'software' && journey && (
                    <LocalArtifacts
                      active={active}
                      scope={{
                        sessionId: step.session.sessionId,
                        stepId: item.stepId,
                        attempt: item.attempt,
                        digest: item.digest,
                      }}
                    />
                  )}
                </td>
                <td>{item.status}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {!evidence.length && (
        <p className="learn-notice">
          {reflectionActivity
            ? '本活动尚未提交复盘。请回到教材完成理解回答，再提交复盘。'
            : '本次尚无证据。完成理解检查后，点击下方执行本课校验。'}
        </p>
      )}
    </section>
  );
}
