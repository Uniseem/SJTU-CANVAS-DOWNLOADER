import { useEffect, useMemo, useState } from 'react';

import { Button, Card, Chip, SearchField, Skeleton } from '@heroui/react';
import { BookOpen, RotateCw } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';

import type { Course } from '@shared/protocol';

import { EmptyBlock, ErrorNotice, PageHeader } from '../components/common';
import { courseSubtitle, isCurrentCourse } from '../format';
import { useStore } from '../store';

export function CoursesPage() {
  const { courses, error, loading, load, openCourse } = useStore(
    useShallow((state) => ({
      courses: state.courses,
      error: state.coursesError,
      loading: state.coursesLoading,
      load: state.loadCourses,
      openCourse: state.openCourse,
    })),
  );
  const [query, setQuery] = useState('');

  useEffect(() => {
    void load();
  }, [load]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const list = courses ?? [];
    if (!needle) {
      return list;
    }
    return list.filter((course) =>
      [course.name, course.course_code, course.teacher ?? '', course.term ?? '']
        .join(' ')
        .toLowerCase()
        .includes(needle),
    );
  }, [courses, query]);
  const current = filtered.filter(isCurrentCourse);
  const past = filtered.filter((course) => !isCurrentCourse(course));

  return (
    <>
      <PageHeader
        title="课程"
        subtitle={courses ? `${courses.length} 门课程，点击课程查看课堂录像和课程文件` : '交我办扫码登录后的全部课程'}
        actions={
          <>
            <SearchField
              aria-label="搜索课程"
              value={query}
              onChange={setQuery}
              className="w-64"
              variant="secondary"
            >
              <SearchField.Group>
                <SearchField.SearchIcon />
                <SearchField.Input placeholder="搜索课程、教师或学期" />
                <SearchField.ClearButton />
              </SearchField.Group>
            </SearchField>
            <Button variant="secondary" isIconOnly aria-label="刷新课程" isDisabled={loading} onPress={() => void load(true)}>
              <RotateCw size={16} className={loading ? 'animate-spin' : ''} />
            </Button>
          </>
        }
      />
      <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-8 pb-8">
        {error ? (
          <ErrorNotice title="无法读取课程列表" message={error} onRetry={() => void load(true)} className="mb-4" />
        ) : null}
        {!courses && loading ? (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3">
            {Array.from({ length: 6 }, (_, index) => (
              <Skeleton key={index} className="h-28 rounded-2xl" />
            ))}
          </div>
        ) : null}
        {courses && courses.length === 0 ? (
          <EmptyBlock
            icon={<BookOpen size={40} />}
            title="没有课程"
            description="Canvas 上还没有你参加的课程，或者课程尚未发布。"
          />
        ) : null}
        {courses && courses.length > 0 && filtered.length === 0 ? (
          <EmptyBlock title="没有匹配的课程" description="换个关键词试试。" />
        ) : null}
        {current.length > 0 ? <CourseSection title="本学期" courses={current} onOpen={openCourse} /> : null}
        {past.length > 0 ? <CourseSection title="历史课程" courses={past} onOpen={openCourse} /> : null}
      </div>
    </>
  );
}

function CourseSection({ title, courses, onOpen }: { title: string; courses: Course[]; onOpen: (id: string) => void }) {
  return (
    <section className="mb-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-medium text-muted">
        {title}
        <span className="text-xs">{courses.length}</span>
      </h2>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3">
        {courses.map((course) => (
          <Card
            key={course.id}
            role="button"
            tabIndex={0}
            onClick={() => onOpen(course.id)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                onOpen(course.id);
              }
            }}
            className="cursor-pointer transition-colors outline-none hover:bg-surface-secondary focus-visible:ring-2 focus-visible:ring-focus"
          >
            <Card.Header>
              <Card.Title className="line-clamp-2 text-base leading-snug">{course.name}</Card.Title>
              <Card.Description className="truncate">{courseSubtitle(course) || ' '}</Card.Description>
            </Card.Header>
            <Card.Footer className="flex items-center gap-2">
              {course.enrollment_state === 'invited_or_pending' ? (
                <Chip size="sm" color="warning" variant="soft">
                  待接受邀请
                </Chip>
              ) : course.enrollment_state === 'completed' ? (
                <Chip size="sm" color="default" variant="soft">
                  已结束
                </Chip>
              ) : (
                <Chip size="sm" color="success" variant="soft">
                  进行中
                </Chip>
              )}
            </Card.Footer>
          </Card>
        ))}
      </div>
    </section>
  );
}
