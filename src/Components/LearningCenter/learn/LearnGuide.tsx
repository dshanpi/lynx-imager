import { useMemo, useState, type ReactNode } from 'react';
import { LynxIcon } from '../../../CoreUI';
import type { LearningCourse, LearningDeepLinkTarget } from '../types';
import { LearnCourseLink } from './LearnCourseLink';
import { filterLearnCourses, learnAccess } from './model';
import { LearnBreadcrumb, LearnFooter, LearnCourseArt, LearnCourseAccess } from './LearnChrome';
import './LearnJourney.css';
import { LearnServiceStatus } from './LearnServiceStatus';

export interface LearnGuideSelection {
  query: string;
  access: string;
  track: string;
  courseId: string;
}
export function LearnGuide({
  previewContent,
  previewCount = 0,
  courses,
  active = false,
  accountId = null,
  onRequestLogin = () => {},
  mode = 'guide',
  selection,
  onSelection,
  onCourse,
  onLocal,
  onImport,
  onResources,
  onSync,
  loading,
  notice,
  onOpenLink,
}: {
  previewContent?: ReactNode;
  previewCount?: number;
  courses: LearningCourse[];
  active?: boolean;
  accountId?: number | null;
  onRequestLogin?: () => void;
  mode?: 'guide' | 'courses';
  selection: LearnGuideSelection;
  onSelection: (value: LearnGuideSelection) => void;
  onCourse: (course: LearningCourse) => void;
  onLocal: () => void;
  onImport: () => void;
  onResources: () => void;
  onSync: () => void;
  loading: boolean;
  notice: string;
  onOpenLink?: (target: LearningDeepLinkTarget) => void;
}) {
  const [showContents, setShowContents] = useState(false);
  const [view, setView] = useState<'courses' | 'resources' | 'plan'>('courses');
  const filtered = useMemo(
    () => filterLearnCourses(courses, selection.query, selection.access, selection.track),
    [courses, selection]
  );
  const selected = filtered.find((c) => c.id === selection.courseId) || filtered[0];
  const outlinePending = selected?.cloudProtocol === 'v2' && !selected.chapters.length;
  const continuing = courses.find((c) => c.progress > 0 && c.progress < 100);
  const update = (patch: Partial<LearnGuideSelection>) => onSelection({ ...selection, ...patch });
  return (
    <div className="learn-journey learn-v2-page learn-guide" data-testid="learn-guide">
      <LearnBreadcrumb items={['学习中心', mode === 'courses' ? '我的课程' : '学习指南']}>
        <button className="learning-text-button" disabled={loading} onClick={onSync}>
          <LynxIcon name="refresh" size={16} spin={loading} />
          同步AI辅助课程
        </button>
      </LearnBreadcrumb>
      <div className="learn-v2-body">
        <header className="learn-guide-heading">
          <h1>{mode === 'courses' ? '我的课程' : '今天想学什么？'}</h1>
          <p>
            {mode === 'courses'
              ? '已报名和本地已安装的 AI 辅助学习整课，在这里继续学习'
              : '选择支持嵌入式 AI 辅助学习的完整课程，或继续上次学习'}
          </p>
        </header>
        {onOpenLink && <LearnCourseLink active={active} onOpen={onOpenLink} />}
        {active && (
          <LearnServiceStatus
            key={accountId ?? 'signed-out'}
            active={active}
            accountId={accountId}
            onRequestLogin={onRequestLogin}
          />
        )}
        <div className="learn-filter-bar">
          <label className="learn-search">
            <LynxIcon name="search" size={20} />
            <input
              aria-label="搜索课程与章节"
              placeholder="搜索课程、章节、资料"
              value={selection.query}
              onChange={(e) => update({ query: e.target.value })}
            />
          </label>
          <select
            aria-label="课程方向"
            value={selection.track}
            onChange={(e) => update({ track: e.target.value })}
          >
            <option value="">全部方向</option>
            {[...new Set(courses.map((c) => c.track).filter(Boolean))].map((track) => (
              <option key={track}>{track}</option>
            ))}
          </select>
          <select
            aria-label="课程权益"
            value={selection.access}
            onChange={(e) => update({ access: e.target.value })}
          >
            <option value="all">全部权益</option>
            <option value="owned">我已拥有</option>
            <option value="free">免费课程</option>
          </select>
          <button className="learning-text-button" onClick={onLocal}>
            进入实验中心
          </button>
        </div>
        <nav className="learn-guide-tabs" aria-label="学习指南分类">
          {(
            [
              ['courses', 'AI辅助课程', 'book'],
              ['resources', '配套资料', 'document'],
              ['plan', '我的计划', 'list-check'],
            ] as const
          ).map(([key, label, icon]) => (
            <button key={key} aria-pressed={view === key} onClick={() => setView(key)}>
              <LynxIcon name={icon} size={24} />
              {label}
            </button>
          ))}
        </nav>
        {notice && (
          <p className="learn-notice" role="status">
            {notice}
          </p>
        )}
        {view === 'resources' ? (
          <section className="learn-panel learn-tab-empty">
            <LynxIcon name="document" size={40} />
            <h2>按产品查看配套资料</h2>
            <p>从已绑定硬件查看快速上手、SDK、系统镜像和手册。</p>
            <button onClick={onResources}>查看我的配套资源 →</button>
            <p>也可以在课程目录中选择教材，查看课程的原始资料入口。</p>
          </section>
        ) : view === 'plan' ? (
          <section className="learn-panel learn-plan">
            <h2>继续你的学习计划</h2>
            {courses
              .filter((c) => c.progress > 0)
              .map((c) => (
                <button className="learn-plan-row" key={c.id} onClick={() => onCourse(c)}>
                  <LynxIcon name="history" size={24} />
                  <strong>{c.title}</strong>
                  <span>{c.progress}%</span>
                  <span>继续学习 →</span>
                </button>
              ))}
            {!continuing && (
              <p>尚无进行中的课程。从课程目录选择想学习的内容，或恢复本地签名课程。</p>
            )}
            <button className="learning-secondary" onClick={onLocal}>
              恢复本地学习会话 →
            </button>
          </section>
        ) : (
          <div className="learn-guide-grid">
            <section className="learn-guide-list">
              {previewCount > 0 && previewContent}
              <div className="learn-continue">
                <LynxIcon name="history" size={40} />
                <div>
                  <strong>{continuing ? '继续上次学习' : '从官网开始 AI 辅助学习'}</strong>
                  <p>{continuing?.title || '同步当前账号可访问的 AI 辅助课程'}</p>
                  <small>
                    {continuing
                      ? `已完成 ${continuing.progress}% · 进入课程确认续学位置`
                      : '在线课程无需导入课程包；获邀试学可从列表直接进入'}
                  </small>
                </div>
                <button
                  disabled={!active || (!continuing && loading)}
                  onClick={() =>
                    continuing ? onCourse(continuing) : accountId ? onSync() : onRequestLogin()
                  }
                >
                  {continuing ? '继续学习' : accountId ? '同步官网课程' : '登录并同步课程'} →
                </button>
              </div>
              <details className="learn-course-link">
                <summary>离线课程包（可选）</summary>
                <p>仅在已取得 .lynx-course 文件时使用。官网在线课程不需要经过此入口。</p>
                <button className="learning-secondary" onClick={onImport}>
                  导入离线课程包
                </button>
              </details>
              <div className="learn-course-grid">
                {!previewCount && previewContent}
                {filtered.map((course) => (
                  <button
                    className={`learn-course-card ${selected?.id === course.id ? 'selected' : ''}`}
                    key={course.id}
                    aria-pressed={selected?.id === course.id}
                    onClick={() => {
                      update({ courseId: course.id });
                      setShowContents(false);
                    }}
                  >
                    <LearnCourseArt course={course} />
                    <div className="learn-course-copy">
                      <h3>{course.title}</h3>
                      <p>{course.summary}</p>
                      <div>
                        <LearnCourseAccess course={course} />
                        <small>
                          {course.cloudProtocol === 'v2' && !course.chapters.length
                            ? '打开后读取目录'
                            : `${course.chapters.length} 个章节`}
                        </small>
                      </div>
                    </div>
                    <LynxIcon name="chevron-right" size={20} />
                  </button>
                ))}
              </div>
              {!filtered.length && !previewCount && (
                <div className="learn-empty">
                  <h3>
                    {courses.length
                      ? '暂无符合条件的课程'
                      : mode === 'courses'
                        ? '暂无已报名的 AI 学习整课'
                        : '暂无可展示的 AI 学习整课'}
                  </h3>
                  {!courses.length && !previewCount && (
                    <p>
                      {mode === 'courses'
                        ? '在学习指南选择官网 AI 整课并领取权益后同步；获邀试学使用官网邀请链接。'
                        : '这里仅展示已发布的嵌入式 AI 辅助学习整课。普通视频课、单个课时和零散实验不会显示。'}
                    </p>
                  )}
                  {courses.length > 0 && (
                    <button
                      className="learning-secondary"
                      onClick={() => update({ query: '', access: 'all', track: '' })}
                    >
                      清除筛选
                    </button>
                  )}
                </div>
              )}
            </section>
            <aside className="learn-guide-detail">
              {selected ? (
                <>
                  <div className="learn-course-overview">
                    <LearnCourseArt course={selected} />
                    <div>
                      <h2>
                        {selected.title} <LearnCourseAccess course={selected} />
                      </h2>
                      <p>{selected.summary}</p>
                    </div>
                  </div>
                  <section>
                    <h3>学完能做什么</h3>
                    {outlinePending && <p>打开课程后，从核验通过的课程版本读取学习目标与任务。</p>}
                    <div className="learn-outcomes">
                      {(selected.labs.length
                        ? selected.labs
                            .slice(0, 3)
                            .map((l) => ({ title: l.title, detail: l.summary }))
                        : selected.chapters.slice(0, 3).map((c) => ({
                            title: c.title,
                            detail:
                              c.kind === 'lab'
                                ? '按课程步骤完成实践与验证'
                                : '阅读课程内容，建立基础理解',
                          }))
                      ).map((item, i) => (
                        <div key={i}>
                          <LynxIcon
                            name={i === 0 ? 'wrench' : i === 1 ? 'terminal' : 'document'}
                            size={24}
                          />
                          <div>
                            <strong>{item.title}</strong>
                            <p>{item.detail}</p>
                          </div>
                        </div>
                      ))}
                    </div>
                  </section>
                  <section>
                    <h3>所需环境</h3>
                    <div className="learn-environment-summary">
                      <LynxIcon name="board" size={24} />
                      <div>
                        {selected.labs[0]?.sdkRequirement || '按课程要求准备开发环境'}
                        <p>
                          {selected.labs[0]?.boardProfiles.map((b) => b.displayName).join('、') ||
                            '设备与工具要求见课程教材'}
                        </p>
                      </div>
                    </div>
                  </section>
                  <section>
                    <div className="learn-section-title">
                      <h3>章节预览</h3>
                      <button
                        className="learning-text-button"
                        onClick={() =>
                          outlinePending ? onCourse(selected) : setShowContents((v) => !v)
                        }
                      >
                        {showContents ? '收起目录' : '查看完整目录'} ›
                      </button>
                    </div>
                    {outlinePending && (
                      <p>目录尚未加载。点击查看完整目录，核对课程权限与签名后展示。</p>
                    )}
                    <ol className="learn-chapter-preview">
                      {selected.chapters.slice(0, showContents ? undefined : 3).map((c) => (
                        <li key={c.id}>
                          {c.title}
                          <small>
                            {c.kind === 'lab' ? '实践' : c.kind === 'video' ? '视频' : '教材'}
                          </small>
                        </li>
                      ))}
                    </ol>
                  </section>
                  <div className="learn-guide-actions">
                    <button onClick={() => onCourse(selected)}>
                      <LynxIcon name="play" size={20} />
                      {selected.catalogOnly
                        ? '前往官网领取课程'
                        : learnAccess(selected).available
                          ? selected.progress > 0
                            ? '继续学习'
                            : '开始学习'
                          : '查看课程'}
                    </button>
                    <button
                      className="learning-secondary"
                      onClick={() => (outlinePending ? onCourse(selected) : setShowContents(true))}
                    >
                      <LynxIcon name="list" size={20} />
                      先看课程目录
                    </button>
                  </div>
                  <small>
                    v{selected.version} ·{' '}
                    {selected.source === 'cloud' ? '官网课程' : '本地签名课程'}
                  </small>
                </>
              ) : (
                <p>选择左侧课程后，查看目标、环境与目录。</p>
              )}
            </aside>
          </div>
        )}
      </div>
      <LearnFooter />
    </div>
  );
}
