import type { ReactNode } from 'react';
import { LynxIcon } from '../../../CoreUI';
import type { LearningCourse } from '../types';
import { learnAccess } from './model';
import './LearnFidelity.css';

export function LearnBreadcrumb({ items, children }: { items: string[]; children?: ReactNode }) {
  return (
    <header className="learn-breadcrumb">
      <div>
        <LynxIcon name="book" size={24} />
        {items.map((item, i) => (
          <span key={`${i}-${item}`}>
            {i > 0 && <i>/</i>}
            {i === items.length - 1 ? <strong>{item}</strong> : item}
          </span>
        ))}
      </div>
      <aside>{children}</aside>
    </header>
  );
}
export function LearnFooter({ children }: { children?: ReactNode }) {
  return (
    <footer className="learn-footer">
      <span>
        <LynxIcon name="document" size={20} />
        {children || '学习记录以当前课程版本与实际操作为准'}
      </span>
      <span>LYNX · 学习中心</span>
    </footer>
  );
}
export function LearnBadge({
  children,
  tone = 'blue',
}: {
  children: ReactNode;
  tone?: 'blue' | 'green' | 'orange' | 'red' | 'gray';
}) {
  return <span className={`learn-badge ${tone}`}>{children}</span>;
}
export function LearnCourseArt({ course }: { course?: LearningCourse }) {
  const kind = /音视频|编码|解码/.test(course?.title || '')
    ? 'video'
    : /Linux.*应用/.test(course?.title || '')
      ? 'linux'
      : /KVM/i.test(course?.title || '')
        ? 'terminal'
        : 'board';
  return (
    <div className={`learn-cover learn-cover-${kind}`} aria-hidden="true">
      {course?.cover ? (
        <img
          src={course.cover}
          alt=""
          onError={(e) => {
            e.currentTarget.style.display = 'none';
          }}
        />
      ) : null}
      <LynxIcon
        name={
          kind === 'video'
            ? 'play'
            : kind === 'linux'
              ? 'code'
              : kind === 'terminal'
                ? 'terminal'
                : 'board'
        }
        size={48}
      />
    </div>
  );
}
export function LearnCourseAccess({ course }: { course: LearningCourse }) {
  const access = learnAccess(course);
  return (
    <LearnBadge tone={access.available ? 'green' : course.source === 'cloud' ? 'red' : 'gray'}>
      {access.label}
    </LearnBadge>
  );
}
