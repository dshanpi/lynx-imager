import type { LocalLearningSession, WorkbenchProject } from '../../../Platform/IPC/Commands';
import type { LearningCourse, LearningLab } from '../types';
import type { ReactNode } from 'react';
import { LearnBreadcrumb, LearnFooter, LearnCourseArt } from './LearnChrome';
import './LearnJourney.css';
import './LearnHub.css';

export function LearnLabs({
  previewContent,
  previewCount = 0,
  courses,
  enrolledCourses,
  cloudCourses,
  query,
  onQuery,
  sessions,
  projects,
  notice,
  loading,
  onRefresh,
  onCourse,
  onResume,
  onImport,
  onGuide,
  onLab,
}: {
  previewContent?: ReactNode;
  previewCount?: number;
  courses: LearningCourse[];
  enrolledCourses: LearningCourse[];
  cloudCourses: LearningCourse[];
  query: string;
  onQuery: (query: string) => void;
  sessions: LocalLearningSession[];
  projects: WorkbenchProject[];
  notice: string;
  loading: boolean;
  onRefresh: () => void;
  onCourse: (c: LearningCourse) => void;
  onResume: (id: string) => void;
  onImport: () => void;
  onGuide: () => void;
  onLab: (l: LearningLab) => void;
}) {
  const search = query.trim().toLowerCase();
  const matches = (...values: string[]) => values.join(' ').toLowerCase().includes(search);
  const installedMatches = courses.filter((c) => matches(c.title, c.summary));
  const enrolledMatches = enrolledCourses.filter((c) => matches(c.title, c.summary));
  const labs = cloudCourses.flatMap((course) => course.labs.map((lab) => ({ course, lab })));
  const labMatches = labs.filter(({ course, lab }) =>
    matches(course.title, lab.title, lab.summary)
  );
  const title = (s: LocalLearningSession) =>
    courses.find((c) => c.localCourseId === s.courseId && c.version === s.courseVersion)?.title ||
    s.courseId;
  const chapter = (s: LocalLearningSession) =>
    courses
      .find((c) => c.localCourseId === s.courseId && c.version === s.courseVersion)
      ?.chapters.find((c) => c.key === s.currentStep)?.title || s.currentStep;
  const states = {
    reading: '阅读教材',
    checking_understanding: '理解检查',
    practicing: '开发实践',
    collecting_evidence: '采集证据',
    verifying: '结果核对',
    reflecting: '章节复盘',
    completed: '课程完成',
  };
  const matching = sessions.filter((s) => matches(title(s), chapter(s)));
  const hasEntries =
    courses.length + enrolledCourses.length + sessions.length + labs.length + previewCount > 0;
  const hasMatches =
    installedMatches.length + enrolledMatches.length + matching.length + labMatches.length > 0;
  return (
    <div className="learn-journey learn-v2-page" data-testid="learn-labs">
      <LearnBreadcrumb items={['学习中心', '实验中心']}>
        <button className="learning-text-button" disabled={loading} onClick={onRefresh}>
          刷新记录
        </button>
        <button onClick={onGuide}>选择在线课程</button>
      </LearnBreadcrumb>
      <div className="learn-v2-body">
        {previewContent}
        <header className="learn-guide-heading">
          <h1>从上次实践继续</h1>
          <p>选定课程与工程，在同一个学习工作台中开发、验证和复盘</p>
        </header>
        <label className="learn-search">
          <input
            aria-label="搜索实验课程"
            placeholder="搜索课程或当前实验"
            value={query}
            onChange={(e) => onQuery(e.target.value)}
          />
        </label>
        {notice && (
          <p className="learn-notice" role="status">
            {notice}
          </p>
        )}
        {loading && <p role="status">正在刷新课程与本地记录…</p>}
        {search && !hasMatches && !loading && (
          <section className="learn-panel learn-empty">
            <h2>没有匹配的课程或实验</h2>
            <p>试试课程名称、实验名称或教材中的关键词。</p>
            <button className="learning-secondary" onClick={() => onQuery('')}>
              清除搜索
            </button>
          </section>
        )}
        {!hasEntries && !loading && (
          <section className="learn-panel learn-empty">
            <h2>先选择一门课程</h2>
            <p>在学习指南同步官网 AI 课程与权益，进入课程后按实际要求准备环境和设备。</p>
            <button onClick={onGuide}>去学习指南</button>
          </section>
        )}
        {enrolledCourses.length > 0 && (
          <section className="learn-hub-section" aria-label="已报名课程">
            <h2>已报名课程</h2>
            <p>进入课程后恢复学习位置，查看本节任务与环境要求。理论学习无需先连接开发板。</p>
            <div className="learn-course-grid">
              {enrolledMatches.map((c) => (
                <button className="learn-course-card" key={c.id} onClick={() => onCourse(c)}>
                  <LearnCourseArt course={c} />
                  <div className="learn-course-copy">
                    <h3>{c.title}</h3>
                    <p>{c.summary}</p>
                    <small>v{c.version} · 已报名版本</small>
                    <p>{c.currentUnitId ? '回到学习位置' : '进入课程'} →</p>
                  </div>
                </button>
              ))}
            </div>
            {!enrolledMatches.length && <p>没有匹配的已报名课程。</p>}
          </section>
        )}
        <details className="learn-course-link">
          <summary>离线课程包（可选）</summary>
          <p>已有 .lynx-course 文件时才需要导入；官网在线课程从已报名课程进入。</p>
          <button className="learning-secondary" onClick={onImport}>
            导入离线课程包
          </button>
        </details>
        <section className="learn-panel learn-hub-section">
          <h2>本地实验与学习记录</h2>
          {matching.map((s) => (
            <button
              className="learn-session-row learning-secondary"
              key={s.sessionId}
              onClick={() => onResume(s.sessionId)}
            >
              <span>
                <strong>{title(s)}</strong>
                <small>
                  v{s.courseVersion} · {chapter(s)} ·{' '}
                  {projects.find((p) => p.id === s.projectId)?.name || '原绑定工程待恢复'}
                </small>
              </span>
              <span>{states[s.state]}</span>
              <span>{s.state === 'completed' ? '查看总结' : '继续实验'} →</span>
            </button>
          ))}
          {!matching.length && (
            <div className="learn-empty" data-testid="learning-labs-empty">
              <h3>{sessions.length ? '没有匹配的实验' : '还没有学习会话'}</h3>
              <p>
                {sessions.length
                  ? '请调整搜索词查看本地学习记录。'
                  : '当前设备没有本地学习会话。已报名课程从上方进入；本地课程包从下方选择。'}
              </p>
            </div>
          )}
        </section>
        <section className="learn-hub-section">
          <h2>已安装的实践课程</h2>
          <div className="learn-course-grid">
            {installedMatches.map((c) => (
              <button className="learn-course-card" key={c.id} onClick={() => onCourse(c)}>
                <LearnCourseArt course={c} />
                <div className="learn-course-copy">
                  <h3>{c.title}</h3>
                  <p>{c.summary}</p>
                  <small>
                    v{c.version} · {c.resumeSessionId ? '继续已有会话' : '选择工程开始'}
                  </small>
                </div>
              </button>
            ))}
          </div>
          {!courses.length && (
            <p>
              当前设备尚未安装课程包。已报名课程无需重复导入；老师提供的 .lynx-course 包可单独导入。
            </p>
          )}
          {courses.length > 0 && !installedMatches.length && <p>没有匹配的已安装课程。</p>}
        </section>
        {labs.length > 0 && (
          <section className="learn-panel learn-hub-section">
            <h2>官网配套实验</h2>
            <p>先检查权益和签名实验包，再按当前课程准备环境。</p>
            {labMatches.map(({ course: c, lab: l }) => (
              <button
                key={`${c.id}:${l.id}`}
                className="learn-session-row learning-secondary"
                onClick={() => onLab(l)}
              >
                <span>
                  <strong>{l.title}</strong>
                  <small>
                    {c.title} · {l.packageVersion || '版本待发布'}
                  </small>
                </span>
                <span>查看环境要求 →</span>
              </button>
            ))}
            {!labMatches.length && <p>没有匹配的配套实验。</p>}
          </section>
        )}
      </div>
      <LearnFooter />
    </div>
  );
}
