import { useState } from 'react';
import type { CloudHardwareItem } from '../../../Platform/IPC/Commands';
import { openUrl } from '@tauri-apps/plugin-opener';
import { formatErrorMessage } from '../../../Utils';
import { LynxIcon } from '../../../CoreUI';
import type { LearningCourse } from '../types';
import {
  LearnBreadcrumb,
  LearnFooter,
  LearnBadge,
  LearnCourseArt,
  LearnCourseAccess,
} from './LearnChrome';
import './LearnJourney.css';

export function LearnResources({
  items,
  courses,
  onBack,
  onSelectCourse,
  onSync,
  syncing = false,
}: {
  items: CloudHardwareItem[];
  courses: LearningCourse[];
  onBack: () => void;
  onSelectCourse: (course: LearningCourse) => void;
  onSync?: () => void;
  syncing?: boolean;
}) {
  const [productId, setProductId] = useState(items[0]?.id);
  const [courseId, setCourseId] = useState('');
  const [query, setQuery] = useState('');
  const [error, setError] = useState('');
  const product = items.find((item) => item.id === productId) || items[0];
  const availableCourses = courses.filter((course) => course.source === 'cloud');
  const selected = availableCourses.find((course) => course.id === courseId) || availableCourses[0];
  const guides = (product?.guides || []).filter((guide) =>
    `${guide.title} ${guide.summary}`.includes(query.trim())
  );
  return (
    <div className="learn-journey learn-v2-page learn-resources" data-testid="learn-resources">
      <LearnBreadcrumb items={['学习中心', '我的硬件', '配套资源']}>
        <button className="learning-text-button" onClick={onBack}>
          返回兑换中心
        </button>
      </LearnBreadcrumb>
      <div className="learn-v2-body learn-resource-layout">
        <div className="learn-resource-main">
          <section className="learn-product-banner">
            <LearnCourseArt />
            <div>
              <h1>{product?.name || '账号课程与配套资源'}</h1>
              <p className="learn-positive">
                {product ? '✓ 已拥有该硬件' : '查看账号可访问的课程'}
              </p>
              <small>{product?.model || '纯课程学习无需硬件权益'} · 资料与实机连接分别确认</small>
            </div>
            <button className="learning-secondary" onClick={onSync} disabled={!onSync || syncing}>
              <LynxIcon name="refresh" size={20} spin={syncing} />
              同步权益
            </button>
          </section>
          <section className="learn-panel">
            <div className="learn-section-title">
              <div>
                <h2>配套资源</h2>
                <p>按型号查看资料与工具，建议从“快速上手”开始。</p>
              </div>
              {items.length > 0 && (
                <select
                  aria-label="选择产品"
                  value={product?.id}
                  onChange={(e) => setProductId(e.target.value)}
                >
                  {items.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.name || item.model}
                    </option>
                  ))}
                </select>
              )}
            </div>
            <input
              aria-label="检索资料"
              placeholder="搜索手册、SDK、镜像"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <div className="learn-resource-grid">
              {guides.map((guide, index) => (
                <article
                  className={`learn-resource-card resource-tone-${index % 4}`}
                  key={guide.id}
                >
                  <div>
                    <LynxIcon
                      name={
                        index % 4 === 0
                          ? 'document'
                          : index % 4 === 1
                            ? 'book'
                            : index % 4 === 2
                              ? 'code'
                              : 'database'
                      }
                      size={40}
                    />
                    <h3>{guide.title}</h3>
                  </div>
                  <p>{guide.summary || '查看官方资料与适用版本'}</p>
                  <small>{guide.kind} · 官方资料</small>
                  <button
                    className="learning-secondary"
                    onClick={() => {
                      setError('');
                      void openUrl(guide.url).catch((reason) =>
                        setError(formatErrorMessage(reason))
                      );
                    }}
                  >
                    查看资料 ›
                  </button>
                </article>
              ))}
            </div>
            {!guides.length && (
              <p className="learn-empty">
                {query
                  ? '未找到匹配资料。'
                  : '官网尚未返回该产品的资料，请同步权益或查看官方产品说明。'}
              </p>
            )}
            {error && (
              <p className="learn-notice" role="alert">
                {error}
              </p>
            )}
          </section>
          <section className="learn-panel">
            <div className="learn-section-title">
              <h2>账号课程</h2>
              <small>课程与产品的配套关系以官网为准</small>
            </div>
            <div className="learn-table-wrap">
              <table className="learn-table">
                <thead>
                  <tr>
                    <th>课程</th>
                    <th>类型</th>
                    <th>我的权益</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {availableCourses.map((course) => (
                    <tr key={course.id} aria-selected={selected?.id === course.id}>
                      <td>
                        <div className="learn-table-course">
                          <LynxIcon name="book" size={24} />
                          <div>
                            <strong>{course.title}</strong>
                            <small>{course.summary}</small>
                          </div>
                        </div>
                      </td>
                      <td>
                        <LearnBadge tone={course.entitlementRequired ? 'orange' : 'green'}>
                          {course.entitlementRequired === undefined
                            ? '待确认'
                            : course.entitlementRequired
                              ? '需权益'
                              : '免费'}
                        </LearnBadge>
                      </td>
                      <td>
                        <LearnCourseAccess course={course} />
                      </td>
                      <td>
                        <button
                          className="learning-secondary"
                          onClick={() => setCourseId(course.id)}
                        >
                          查看课程 ›
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {!availableCourses.length && (
              <p className="learn-notice">尚未取得官网课程目录，请返回学习指南同步。</p>
            )}
          </section>
        </div>
        <aside className="learn-guide-detail learn-resource-detail">
          <h2>课程详情</h2>
          {selected ? (
            <>
              <div className="learn-detail-title">
                <LynxIcon name="book" size={40} />
                <div>
                  <h2>{selected.title}</h2>
                  <LearnCourseAccess course={selected} />
                </div>
              </div>
              <p>{selected.summary}</p>
              <div className="learn-outcomes">
                <div>
                  <LynxIcon name="wrench" size={24} />
                  <div>
                    <strong>适配环境</strong>
                    <p>
                      {selected.labs[0]?.boardProfiles.map((b) => b.displayName).join('、') ||
                        '根据课程目录核对型号要求'}
                    </p>
                  </div>
                </div>
                <div>
                  <LynxIcon name="list" size={24} />
                  <div>
                    <strong>从课程目标开始</strong>
                    <p>{selected.chapters[0]?.title || '查看课程内容与实践任务'}</p>
                  </div>
                </div>
                <div>
                  <LynxIcon name="desktop" size={24} />
                  <div>
                    <strong>开发与验证</strong>
                    <p>{selected.labs[0]?.sdkRequirement || '在课程工作台核对工程与设备'}</p>
                  </div>
                </div>
              </div>
              <button onClick={() => onSelectCourse(selected)}>进入学习指南 →</button>
              <p className="learn-notice">
                可以自由选择其他课程。未解锁课程先查看介绍，取得权益后再学习。
              </p>
            </>
          ) : (
            <p>选择左侧课程，查看学习目标与环境要求。</p>
          )}
        </aside>
      </div>
      <LearnFooter />
    </div>
  );
}
