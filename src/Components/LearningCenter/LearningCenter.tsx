import { useCallback, useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { mergeLearnCatalog, publishedLearnCatalog } from './learn/catalog';
import { invokeCommand } from '../../Platform/IPC/Client';
import type {
  CloudLearningEnrollment,
  InstalledLocalCourse,
  LocalLearningSession,
  WorkbenchProject,
  LearnPreviewList,
} from '../../Platform/IPC/Commands';
import { formatLearnError } from './learn/errors';
import { readCloudLearningCatalog } from './cloud';
import { MyHardware } from './MyHardware';
import { LocalCourseWorkspace } from './LocalCourseWorkspace';
import { LearnRecords } from './learn/LearnRecords';
import { LearnGuide, type LearnGuideSelection } from './learn/LearnGuide';
import { LearnLabs } from './learn/LearnLabs';
import { LearnGenericWorkspace } from './learn/LearnGenericWorkspace';
import { LearnPreviewEntry } from './learn/LearnPreviewEntry';
import { LearnPreviewShelf } from './learn/LearnPreviewShelf';
import { learnDeepLinkDisposition } from './learn/deepLink';
import { learnGenericCatalog } from './learn/genericModel';
import { learnServiceError } from './learn/serviceStatus';
import { LearnCloudWorkspace } from './learn/LearnCloudWorkspace';
import { LearnLabPreparation } from './learn/LearnLabPreparation';
import { learnAccess, learnLocalCatalog, learnEnrollmentCatalog } from './learn/model';
import type {
  LearningCourse,
  LearningDeepLinkTarget,
  LearningLab,
  LearningSection,
  LearningProgress,
} from './types';
import './LearningCenter.css';

interface LearningCenterPageProps {
  isActive: boolean;
  section: LearningSection;
  authUser: { id: number; name: string } | null;
  onRequestLogin: () => void;
  loginPending?: boolean;
  onCancelLogin?: () => void;
  onSectionChange: (section: LearningSection) => void;
  deepLinkTarget: LearningDeepLinkTarget | null;
  onDeepLinkHandled: () => void;
  onOpenLink: (target: LearningDeepLinkTarget) => void;
}
type Destination =
  | { section: 'courses'; kind: 'preview'; previewId: string; autoOpen?: boolean }
  | {
      section: 'labs';
      kind: 'local';
      course?: LearningCourse;
      sessionId?: string;
      reviewStepId?: string;
    }
  | {
      section: 'courses' | 'labs';
      kind: 'cloud';
      course: LearningCourse;
      initialChapterId?: string;
    }
  | { section: 'labs'; kind: 'precheck'; lab: LearningLab };

export function LearningCenterPage(props: LearningCenterPageProps) {
  return <LearningCenterAccount key={props.authUser?.id ?? 'anonymous'} {...props} />;
}

function LearningCenterAccount({
  isActive,
  section,
  authUser,
  onRequestLogin,
  loginPending = false,
  onCancelLogin,
  onSectionChange,
  deepLinkTarget,
  onDeepLinkHandled,
  onOpenLink,
}: LearningCenterPageProps) {
  const [syncedTarget, setSyncedTarget] = useState<LearningDeepLinkTarget | null>(null);
  const [publishedCourses, setPublishedCourses] = useState<LearningCourse[]>([]);
  const [previews, setPreviews] = useState<LearnPreviewList | null>(null);
  const [genericCourses, setGenericCourses] = useState<LearningCourse[]>([]);
  const [cloud, setCloud] = useState<LearningCourse[]>([]);
  const [cloudProgress, setCloudProgress] = useState<LearningProgress[]>([]);
  const [enrollments, setEnrollments] = useState<CloudLearningEnrollment[]>([]);
  const [projects, setProjects] = useState<WorkbenchProject[]>([]);
  const [installed, setInstalled] = useState<InstalledLocalCourse[]>([]);
  const [sessions, setSessions] = useState<LocalLearningSession[]>([]);
  const [loading, setLoading] = useState(false);
  const [notice, setNotice] = useState('');
  const [routeNotice, setRouteNotice] = useState('');
  useEffect(() => {
    if (deepLinkTarget) setRouteNotice('');
  }, [deepLinkTarget]);
  const [loaded, setLoaded] = useState(false);
  const [destination, setDestination] = useState<Destination | null>(null);
  const [labQuery, setLabQuery] = useState('');
  const [selection, setSelection] = useState<LearnGuideSelection>({
    query: '',
    access: 'all',
    track: '',
    courseId: '',
  });
  const generation = useRef(0);
  const lock = useRef(false);
  const sync = useCallback(async () => {
    if (!isActive || lock.current) return;
    lock.current = true;
    const request = ++generation.current;
    setLoading(true);
    const legacyHome = authUser ? invokeCommand('learning_get_home') : Promise.resolve(null);
    const results = await Promise.allSettled([
      invokeCommand('learning_list_local_courses'),
      invokeCommand('learning_list_local_sessions'),
      legacyHome.then((home) => (home ? readCloudLearningCatalog(home.enrollments) : null)),
      legacyHome,
      invokeCommand('workbench_list_projects'),
      authUser ? invokeCommand('learn_get_home') : Promise.resolve(null),
      invokeCommand('learn_list_courses'),
      authUser ? invokeCommand('learn_list_previews') : Promise.resolve(null),
    ]);
    if (request !== generation.current) return;
    const errors: string[] = [];
    const [packages, history, catalog, home, savedProjects, generic, published, invitations] =
      results;
    if (invitations.status === 'fulfilled') setPreviews(invitations.value);
    else {
      setPreviews(null);
      errors.push(`试学课程同步失败：${learnServiceError(invitations.reason)}`);
    }
    if (savedProjects.status === 'fulfilled') setProjects(savedProjects.value);
    else errors.push(`工程名称暂不可用：${formatLearnError(savedProjects.reason)}`);
    if (packages.status === 'fulfilled') setInstalled(packages.value);
    else errors.push(`本地课程读取失败：${formatLearnError(packages.reason)}`);
    if (history.status === 'fulfilled') setSessions(history.value);
    else errors.push(`学习会话读取失败：${formatLearnError(history.reason)}`);
    if (catalog.status === 'fulfilled' && catalog.value) {
      setCloud(catalog.value.courses);
      setCloudProgress(catalog.value.progress);
      if (catalog.value.progressWarning) errors.push(catalog.value.progressWarning);
    } else if (catalog.status === 'rejected')
      errors.push(`官网课程刷新失败，继续显示上次同步结果：${formatLearnError(catalog.reason)}`);
    if (home.status === 'fulfilled' && home.value) setEnrollments(home.value.enrollments);
    else if (home.status === 'rejected')
      errors.push(`已购课时读取失败：${formatLearnError(home.reason)}`);
    if (generic.status === 'fulfilled' && generic.value)
      setGenericCourses(learnGenericCatalog(generic.value.enrolled));
    else if (generic.status === 'rejected')
      errors.push(`新版课程同步失败：${learnServiceError(generic.reason)}`);
    if (published.status === 'fulfilled')
      setPublishedCourses(publishedLearnCatalog(published.value));
    else
      errors.push(
        `AI 学习整课目录同步失败，保留上次专属目录：${learnServiceError(published.reason)}`
      );
    setNotice(errors.join('；'));
    const targetSynced = deepLinkTarget?.courseSlug?.startsWith('class-')
      ? generic.status === 'fulfilled'
      : home.status === 'fulfilled';
    if (authUser && targetSynced) setSyncedTarget(deepLinkTarget);
    setLoaded(true);
    setLoading(false);
    lock.current = false;
  }, [authUser, isActive, deepLinkTarget]);
  useEffect(() => {
    const counter = generation;
    void sync();
    return () => {
      counter.current++;
      lock.current = false;
    };
  }, [sync]);

  const localCourses = learnLocalCatalog(installed, sessions);
  const enrolledCourses = learnEnrollmentCatalog(enrollments);
  const courses = mergeLearnCatalog(
    publishedCourses,
    genericCourses,
    enrolledCourses,
    localCourses
  );
  // Legacy lab lookup is only for an actual whole-course enrollment, never discovery.
  const enrolledCloud = cloud.filter((c) =>
    enrollments.some((e) => e.bundleKey === c.slug && e.version === c.version)
  );
  const openLocal = (course?: LearningCourse, sessionId?: string, reviewStepId?: string) => {
    setDestination({
      section: 'labs',
      kind: 'local',
      course,
      sessionId: sessionId || course?.resumeSessionId,
      reviewStepId,
    });
    onSectionChange('labs');
  };
  const openCourse = (course: LearningCourse) => {
    if (course.catalogOnly) {
      void openUrl(`https://www.100ask.net/lynx-learn/${encodeURIComponent(course.slug)}/`).catch(
        () => setNotice('无法打开官网课程详情，请稍后重试。')
      );
    } else if (course.source === 'local') openLocal(course);
    else {
      const target = section === 'labs' && course.cloudProtocol === 'v2' ? 'labs' : 'courses';
      setDestination({ section: target, kind: 'cloud', course });
      onSectionChange(target);
    }
  };
  const openLab = (lab: LearningLab) => {
    setDestination({ section: 'labs', kind: 'precheck', lab });
    onSectionChange('labs');
  };
  const back = () => {
    setDestination(null);
    void sync();
  };
  const current = destination?.section === section ? destination : null;
  const previewContent = authUser ? (
    <LearnPreviewShelf
      list={previews}
      query={section === 'labs' ? labQuery : selection.query}
      busy={loading}
      onOpen={(previewId) => {
        setDestination({ section: 'courses', kind: 'preview', previewId, autoOpen: true });
        onSectionChange('courses');
      }}
    />
  ) : null;

  useEffect(() => {
    const route = learnDeepLinkDisposition(
      deepLinkTarget,
      isActive,
      Boolean(authUser),
      loaded && (Boolean(deepLinkTarget?.previewId) || syncedTarget === deepLinkTarget),
      loading
    );
    if (!deepLinkTarget || route === 'wait' || route === 'login') return;
    if (route === 'preview' && deepLinkTarget.previewId) {
      setDestination({ section: 'courses', kind: 'preview', previewId: deepLinkTarget.previewId });
      onSectionChange('courses');
      onDeepLinkHandled();
      return;
    }
    const course = [...genericCourses, ...learnEnrollmentCatalog(enrollments)].find(
      (c) => c.slug === deepLinkTarget.courseSlug
    );
    const enrollment = enrollments.find((e) => e.bundleKey === deepLinkTarget.courseSlug);
    if (deepLinkTarget.labKey) {
      const lab = cloud
        .filter(
          (c) =>
            c.slug === deepLinkTarget.courseSlug &&
            enrollments.some((e) => e.bundleKey === c.slug && e.version === c.version)
        )
        .flatMap((c) => c.labs)
        .find((l) => l.key === deepLinkTarget.labKey);
      if (lab) {
        setDestination({ section: 'labs', kind: 'precheck', lab });
        onSectionChange('labs');
      } else setRouteNotice('官网没有返回指定实验，请同步课程后重试。');
    } else if (course || enrollment) {
      const selected =
        course?.cloudProtocol === 'v2'
          ? course
          : enrollment
            ? learnEnrollmentCatalog([enrollment])[0]
            : course!;
      setDestination({
        section: 'courses',
        kind: 'cloud',
        course: selected,
        initialChapterId: deepLinkTarget.chapterId || undefined,
      });
      onSectionChange('courses');
    } else
      setRouteNotice(
        '官网没有返回指定课程。请确认课程已发布，并在官网用当前账号领取后重新打开课程链接。'
      );
    onDeepLinkHandled();
  }, [
    isActive,
    authUser,
    deepLinkTarget,
    loaded,
    loading,
    cloud,
    genericCourses,
    syncedTarget,
    enrollments,
    onDeepLinkHandled,
    onSectionChange,
  ]);

  return (
    <main className="learning-center" data-testid="learning-center">
      {deepLinkTarget && !authUser && (
        <section className="learn-pending-link" aria-label="待恢复的学习入口">
          <strong>{deepLinkTarget.previewId ? '已收到官网试学邀请' : '已收到课程学习链接'}</strong>
          <p>请先登录购买或获邀的官网账号。登录完成后将自动继续，学习入口已为你保留。</p>
          {loginPending && (
            <p role="status">正在等待官网登录返回。若官网要求验证邮箱，请在官网完成后继续授权。</p>
          )}
          <div className="learn-pending-link-actions">
            <button type="button" disabled={loginPending} onClick={onRequestLogin}>
              {loginPending ? '等待登录完成…' : '登录并继续'}
            </button>
            {loginPending && onCancelLogin && (
              <button type="button" onClick={onCancelLogin}>
                取消本次登录
              </button>
            )}
            <button type="button" onClick={onDeepLinkHandled}>
              取消此学习跳转
            </button>
          </div>
        </section>
      )}
      {current?.kind === 'preview' ? (
        <LearnPreviewEntry
          key={current.previewId}
          active={isActive}
          previewId={current.previewId}
          autoOpen={current.autoOpen}
          onBack={back}
        />
      ) : current?.kind === 'local' ? (
        <LocalCourseWorkspace
          key={`${current.course?.id || ''}:${current.sessionId || ''}:${current.reviewStepId || ''}`}
          active={isActive}
          initialCourseId={current.course?.localCourseId}
          initialCourseVersion={current.course?.version}
          initialSessionId={current.sessionId}
          initialReviewStepId={current.reviewStepId}
          onBack={back}
        />
      ) : current?.kind === 'cloud' && current.course.cloudProtocol === 'v2' ? (
        <LearnGenericWorkspace
          key={`${authUser?.id}:${current.course.id}:${current.initialChapterId || ''}`}
          active={isActive}
          accountId={authUser?.id}
          course={current.course}
          initialChapterId={current.initialChapterId}
          onBack={back}
          backLabel={current.section === 'labs' ? '返回实验中心' : '返回我的课程'}
        />
      ) : current?.kind === 'cloud' ? (
        <LearnCloudWorkspace
          key={current.course.id}
          active={isActive}
          course={current.course}
          onBack={back}
          onLab={(labId) => {
            const lab = cloud.flatMap((c) => c.labs).find((l) => l.id === String(labId));
            if (lab) {
              openLab(lab);
              return true;
            }
            return false;
          }}
          onLogin={onRequestLogin}
          loggedIn={Boolean(authUser)}
        />
      ) : current?.kind === 'precheck' ? (
        <LearnLabPreparation
          key={current.lab.id}
          isActive={isActive}
          lab={current.lab}
          loggedIn={Boolean(authUser)}
          onLogin={onRequestLogin}
          onBack={back}
        />
      ) : section === 'hardware' ? (
        <MyHardware
          courses={courses}
          isActive={isActive}
          authUser={authUser}
          onRequestLogin={onRequestLogin}
          onOpenLabs={() => {
            setDestination(null);
            onSectionChange('labs');
          }}
          onSelectCourse={(c) => {
            setSelection({ query: '', access: 'all', track: '', courseId: c.id });
            setDestination(null);
            onSectionChange('home');
          }}
        />
      ) : section === 'history' ? (
        <LearnRecords
          active={isActive}
          onResume={(id, stepId) => openLocal(undefined, id, stepId)}
          cloudProgress={cloudProgress.map((p) => ({
            ...p,
            title:
              cloud.find((c) => p.resourceType === 'course' && c.id === p.resourceId)?.title ||
              cloud
                .flatMap((c) => c.labs)
                .find((l) => p.resourceType === 'lab' && l.id === p.resourceId)?.title ||
              '官网学习记录',
          }))}
        />
      ) : section === 'labs' ? (
        <LearnLabs
          previewContent={previewContent}
          previewCount={previews?.items.length || 0}
          courses={localCourses}
          enrolledCourses={[...genericCourses, ...enrolledCourses]}
          cloudCourses={enrolledCloud}
          query={labQuery}
          onQuery={setLabQuery}
          sessions={sessions}
          projects={projects}
          notice={[routeNotice, notice].filter(Boolean).join('；')}
          loading={loading}
          onRefresh={() => void sync()}
          onCourse={openCourse}
          onResume={(id) => openLocal(undefined, id)}
          onImport={() => openLocal()}
          onGuide={() => {
            setDestination(null);
            onSectionChange('home');
          }}
          onLab={openLab}
        />
      ) : (
        <LearnGuide
          previewContent={previewContent}
          previewCount={previews?.items.length || 0}
          key={section}
          active={isActive}
          accountId={authUser?.id ?? null}
          onRequestLogin={onRequestLogin}
          mode={section === 'courses' ? 'courses' : 'guide'}
          onOpenLink={onOpenLink}
          courses={
            section === 'courses'
              ? courses.filter((c) => !c.catalogOnly && learnAccess(c).available)
              : courses
          }
          selection={selection}
          onSelection={setSelection}
          onCourse={openCourse}
          onImport={() => openLocal()}
          onLocal={() => {
            setDestination(null);
            onSectionChange('labs');
          }}
          onResources={() => onSectionChange('hardware')}
          onSync={() => void sync()}
          loading={loading}
          notice={
            [routeNotice, notice].filter(Boolean).join('；') ||
            (!authUser ? '登录后同步已购课程和权益；本地已安装课程仍可学习。' : '')
          }
        />
      )}
    </main>
  );
}
