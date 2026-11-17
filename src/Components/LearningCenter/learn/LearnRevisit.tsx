import type {
  LearnJourney,
  LocalLearningLab,
  LocalActivityAccess,
} from '../../../Platform/IPC/Commands';
import { localPrerequisiteStatus } from './model';

export function LearnPrerequisites({
  access,
  onReview,
}: {
  access?: LocalActivityAccess;
  onReview?: (stepId: string) => void;
}) {
  if (!access?.prerequisites.length) return null;
  return (
    <div className="learn-notice" data-testid="local-prerequisites">
      <strong>前置条件</strong>
      <ul>
        {access.prerequisites.map((requirement) => (
          <li key={requirement.stepId}>
            {onReview ? (
              <button className="learning-text-button" onClick={() => onReview(requirement.stepId)}>
                {requirement.title}
              </button>
            ) : (
              requirement.title
            )}
            {' · '}
            {localPrerequisiteStatus[requirement.status]}
          </li>
        ))}
      </ul>
      <p>
        {access.available
          ? '前置完成记录已具备；执行前还会重新检查声明源码、工程绑定和环境。'
          : '请先补齐前置活动。可以继续阅读教材，当前不能开始或推进本课；旧记录仍保留。'}
      </p>
    </div>
  );
}

export function LearnRevisit({
  lab,
  attempt,
  current,
  busy,
  onContinue,
  access,
  onReview,
}: {
  lab: LocalLearningLab;
  attempt: LearnJourney['attempts'][number] | undefined;
  current: boolean;
  busy: boolean;
  onContinue: () => void;
  access?: LocalActivityAccess;
  onReview?: (stepId: string) => void;
}) {
  const ended = attempt && ['completed', 'skipped'].includes(attempt.state);
  return (
    <section className="learn-panel" data-testid="local-activity-revisit">
      <h3>
        {lab.title} · {attempt ? `尝试 ${attempt.attempt}` : '尚未开始'}
      </h3>
      <p className="learn-notice">
        {ended
          ? '重新学习会创建新的尝试，从阅读开始。旧答案、草稿和证据保留；本次需要重新完成，不能沿用历史通过。'
          : attempt
            ? current
              ? '这是当前学习课时，继续下方学习步骤即可。'
              : '继续本课将恢复原尝试和已保存草稿。当前课时的输入会先保存，不会执行命令或自动验证。'
            : access
              ? '可先阅读教材。前置条件满足后，可自行开始本课，无需先完成无关课时；开始不会自动执行命令。'
              : '可先阅读教材。开始实践仍按当前课程顺序进行，不会因预览而跳过前面的课时。'}
      </p>
      <LearnPrerequisites access={access} onReview={onReview} />
      {(!!attempt || !!access) && (ended || !current) && (
        <button disabled={busy || access?.available === false} onClick={onContinue}>
          {!attempt ? '开始本课学习' : ended ? '重新学习本课' : '继续本课学习'}
        </button>
      )}
    </section>
  );
}
