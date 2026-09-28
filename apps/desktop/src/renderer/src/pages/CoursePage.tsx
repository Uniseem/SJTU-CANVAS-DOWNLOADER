import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { Button, Chip, SearchField, Tabs, cn, toast } from '@heroui/react';
import type { Key } from 'react-aria-components';
import { ArrowLeft, Download, FileText, RotateCw, Video } from 'lucide-react';

import type { Course, CourseFile, Lesson, NewDownload, Track, TrackSize } from '@shared/protocol';
import { TRACKS } from '@shared/protocol';

import { engine, errorMessage, shell } from '../api';
import { CheckBox, EmptyBlock, ErrorNotice, LoadingBlock, PageHeader } from '../components/common';
import { TRACK_LABELS, courseSubtitle, fileKind, formatBytes, formatDate, formatLessonTime, runPool } from '../format';
import { useStore } from '../store';

type Tab = 'lessons' | 'files';
type SizeMap = Record<string, Partial<Record<Track, TrackSize>>>;

interface CourseData {
  lessons: Lesson[] | null;
  lessonsError: string | null;
  files: CourseFile[] | null;
  filesError: string | null;
  sizes: SizeMap;
}

/** Loaded course content survives navigating back and forth. */
const cache = new Map<string, CourseData>();

function emptyData(): CourseData {
  return { lessons: null, lessonsError: null, files: null, filesError: null, sizes: {} };
}

export function CoursePage({ courseId }: { courseId: string }) {
  const course = useStore((state) => state.courses?.find((item) => item.id === courseId) ?? null);
  const settings = useStore((state) => state.settings);
  const closeCourse = useStore((state) => state.closeCourse);
  const navigate = useStore((state) => state.navigate);

  const [data, setData] = useState<CourseData>(() => cache.get(courseId) ?? emptyData());
  const [tab, setTab] = useState<Tab>('lessons');
  const [query, setQuery] = useState('');
  const [tracks, setTracks] = useState<Track[]>(() => settings?.preferences.default_tracks ?? ['slides', 'teacher']);
  const [selectedLessons, setSelectedLessons] = useState<Set<string>>(() => new Set());
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(() => new Set());
  const [loading, setLoading] = useState<{ lessons: boolean; files: boolean }>({ lessons: false, files: false });
  const [submitting, setSubmitting] = useState(false);
  const requestedSizes = useRef(new Set<string>());

  useEffect(() => {
    cache.set(courseId, data);
  }, [courseId, data]);

  const loadLessons = useCallback(
    async (force = false) => {
      if (!force && (data.lessons || loading.lessons)) {
        return;
      }
      setLoading((state) => ({ ...state, lessons: true }));
      setData((state) => ({ ...state, lessonsError: null }));
      try {
        const lessons = await engine.lessons(courseId);
        setData((state) => ({ ...state, lessons, sizes: force ? {} : state.sizes }));
        if (force) {
          requestedSizes.current.clear();
        }
      } catch (error) {
        setData((state) => ({ ...state, lessonsError: errorMessage(error) }));
      } finally {
        setLoading((state) => ({ ...state, lessons: false }));
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [courseId, data.lessons, loading.lessons],
  );

  const loadFiles = useCallback(
    async (force = false) => {
      if (!force && (data.files || loading.files)) {
        return;
      }
      setLoading((state) => ({ ...state, files: true }));
      setData((state) => ({ ...state, filesError: null }));
      try {
        const files = await engine.files(courseId);
        setData((state) => ({ ...state, files }));
      } catch (error) {
        setData((state) => ({ ...state, filesError: errorMessage(error) }));
      } finally {
        setLoading((state) => ({ ...state, files: false }));
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [courseId, data.files, loading.files],
  );

  useEffect(() => {
    if (tab === 'lessons') {
      void loadLessons();
    } else {
      void loadFiles();
    }
  }, [tab, loadLessons, loadFiles]);

  // Sizes of the selected recordings, for the selected tracks. The engine
  // caches them and limits its own concurrency; this keeps requests unique.
  useEffect(() => {
    const pending: { lessonId: string; tracks: Track[] }[] = [];
    for (const lessonId of selectedLessons) {
      const missing = tracks.filter((track) => !data.sizes[lessonId]?.[track] && !requestedSizes.current.has(`${lessonId}:${track}`));
      if (missing.length > 0) {
        pending.push({ lessonId, tracks: missing });
        for (const track of missing) {
          requestedSizes.current.add(`${lessonId}:${track}`);
        }
      }
    }
    if (pending.length === 0) {
      return;
    }
    let cancelled = false;
    void runPool(pending, 3, async ({ lessonId, tracks: wanted }) => {
      try {
        const result = await engine.sizes(courseId, lessonId, wanted);
        if (!cancelled) {
          setData((state) => ({
            ...state,
            sizes: { ...state.sizes, [lessonId]: { ...state.sizes[lessonId], ...(result.tracks as SizeMap[string]) } },
          }));
        }
      } catch {
        for (const track of wanted) {
          requestedSizes.current.delete(`${lessonId}:${track}`);
        }
      }
    });
    return () => {
      cancelled = true;
    };
  }, [courseId, selectedLessons, tracks, data.sizes]);

  const needle = query.trim().toLowerCase();
  const visibleLessons = useMemo(
    () =>
      (data.lessons ?? []).filter(
        (lesson) => !needle || `${lesson.title} ${lesson.classroom} ${lesson.begin_time}`.toLowerCase().includes(needle),
      ),
    [data.lessons, needle],
  );
  const visibleFiles = useMemo(
    () => (data.files ?? []).filter((file) => !needle || `${file.display_name} ${file.filename}`.toLowerCase().includes(needle)),
    [data.files, needle],
  );
  const availableLessonIds = visibleLessons.filter((lesson) => lesson.available).map((lesson) => lesson.video_id);
  const allLessonsSelected = availableLessonIds.length > 0 && availableLessonIds.every((id) => selectedLessons.has(id));
  const someLessonsSelected = availableLessonIds.some((id) => selectedLessons.has(id));
  const allFilesSelected = visibleFiles.length > 0 && visibleFiles.every((file) => selectedFiles.has(file.id));
  const someFilesSelected = visibleFiles.some((file) => selectedFiles.has(file.id));

  const plan = useMemo(() => {
    let files = 0;
    let bytes = 0;
    let unknown = 0;
    if (tab === 'lessons') {
      for (const lessonId of selectedLessons) {
        for (const track of tracks) {
          const size = data.sizes[lessonId]?.[track];
          if (size?.status === 'missing') {
            continue;
          }
          files += 1;
          if (size?.status === 'ready' && size.size) {
            bytes += size.size;
          } else {
            unknown += 1;
          }
        }
      }
    } else {
      for (const file of data.files ?? []) {
        if (selectedFiles.has(file.id)) {
          files += 1;
          if (file.size > 0) {
            bytes += file.size;
          } else {
            unknown += 1;
          }
        }
      }
    }
    return { files, bytes, unknown };
  }, [tab, selectedLessons, selectedFiles, tracks, data.sizes, data.files]);

  const selectedCount = tab === 'lessons' ? selectedLessons.size : selectedFiles.size;

  const download = async (): Promise<void> => {
    if (!course || plan.files === 0) {
      return;
    }
    const items: NewDownload[] = [];
    if (tab === 'lessons') {
      for (const lesson of data.lessons ?? []) {
        if (!selectedLessons.has(lesson.video_id)) {
          continue;
        }
        for (const track of tracks) {
          const size = data.sizes[lesson.video_id]?.[track];
          if (size?.status === 'missing') {
            continue;
          }
          items.push({
            kind: 'video',
            course_id: course.id,
            course_name: course.name,
            lesson_id: lesson.video_id,
            title: lesson.title,
            begin_time: lesson.begin_time.trim() || undefined,
            track,
            size: size?.status === 'ready' && size.size ? size.size : undefined,
          });
        }
      }
    } else {
      for (const file of data.files ?? []) {
        if (selectedFiles.has(file.id)) {
          items.push({
            kind: 'file',
            course_id: course.id,
            course_name: course.name,
            file_id: file.id,
            title: file.display_name.trim() || file.filename,
            size: file.size > 0 ? file.size : undefined,
          });
        }
      }
    }
    let destination: string | undefined;
    if (settings?.preferences.ask_destination) {
      const chosen = await shell.chooseFolder(settings.preferences.download_dir);
      if (!chosen) {
        return;
      }
      destination = chosen;
    }
    setSubmitting(true);
    try {
      const result = await engine.createDownloads(items, destination);
      const skipped = result.skipped.length;
      if (result.created.length > 0) {
        toast.success(`已添加 ${result.created.length} 个下载任务`, {
          description: skipped > 0 ? `${skipped} 个已在列表中或已下载，未重复添加` : undefined,
          actionProps: { children: '查看下载', onPress: () => navigate('downloads') },
        });
      } else {
        toast.info('没有新的下载任务', { description: skipped > 0 ? '所选内容都已在下载列表中或已下载' : undefined });
      }
      if (tab === 'lessons') {
        setSelectedLessons(new Set());
      } else {
        setSelectedFiles(new Set());
      }
    } catch (error) {
      toast.danger('无法添加下载', { description: errorMessage(error) });
    } finally {
      setSubmitting(false);
    }
  };

  if (!course) {
    return (
      <>
        <PageHeader title="课程" leading={<BackButton onPress={closeCourse} />} />
        <div className="px-8">
          <ErrorNotice message="找不到这门课程，请返回课程列表重新打开。" onRetry={closeCourse} retryLabel="返回" />
        </div>
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={course.name}
        subtitle={courseSubtitle(course)}
        leading={<BackButton onPress={closeCourse} />}
        actions={
          <Button
            variant="secondary"
            isIconOnly
            aria-label="刷新"
            isDisabled={tab === 'lessons' ? loading.lessons : loading.files}
            onPress={() => void (tab === 'lessons' ? loadLessons(true) : loadFiles(true))}
          >
            <RotateCw size={16} className={(tab === 'lessons' ? loading.lessons : loading.files) ? 'animate-spin' : ''} />
          </Button>
        }
      />

      <div className="flex shrink-0 flex-wrap items-center gap-3 px-8 pb-3">
        <Tabs
          selectedKey={tab}
          onSelectionChange={(key: Key) => setTab(key as Tab)}
          variant="secondary"
          aria-label="课程内容"
        >
          <Tabs.ListContainer>
            <Tabs.List aria-label="课程内容">
              <Tabs.Tab id="lessons" className="whitespace-nowrap">
                课堂录像
                {data.lessons ? <span className="ml-1 text-xs text-muted">{data.lessons.length}</span> : null}
                <Tabs.Indicator />
              </Tabs.Tab>
              <Tabs.Tab id="files" className="whitespace-nowrap">
                课程文件
                {data.files ? <span className="ml-1 text-xs text-muted">{data.files.length}</span> : null}
                <Tabs.Indicator />
              </Tabs.Tab>
            </Tabs.List>
          </Tabs.ListContainer>
        </Tabs>
        <div className="flex-1" />
        <SearchField aria-label="搜索" value={query} onChange={setQuery} className="w-60" variant="secondary">
          <SearchField.Group>
            <SearchField.SearchIcon />
            <SearchField.Input placeholder={tab === 'lessons' ? '搜索讲次或教室' : '搜索文件'} />
            <SearchField.ClearButton />
          </SearchField.Group>
        </SearchField>
      </div>

      {tab === 'lessons' ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2 px-8 pb-3">
          <span className="text-xs text-muted">下载画面：</span>
          {TRACKS.map((track) => {
            const active = tracks.includes(track);
            return (
              <button
                key={track}
                type="button"
                aria-pressed={active}
                onClick={() =>
                  setTracks((current) =>
                    active ? (current.length > 1 ? current.filter((item) => item !== track) : current) : TRACKS.filter((item) => item === track || current.includes(item)),
                  )
                }
                className={cn(
                  'rounded-full border px-3 py-1 text-xs transition-colors outline-none focus-visible:ring-2 focus-visible:ring-focus',
                  active
                    ? 'border-accent bg-accent-soft text-accent-soft-foreground'
                    : 'border-border text-muted hover:bg-surface-secondary',
                )}
              >
                {TRACK_LABELS[track]}
              </button>
            );
          })}
          <span className="ml-2 text-xs text-muted">合成画面并非每节课都有</span>
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-8">
        {tab === 'lessons' ? (
          <LessonList
            lessons={visibleLessons}
            total={data.lessons?.length ?? 0}
            error={data.lessonsError}
            loading={loading.lessons && !data.lessons}
            selected={selectedLessons}
            sizes={data.sizes}
            tracks={tracks}
            allSelected={allLessonsSelected}
            someSelected={someLessonsSelected}
            onToggleAll={(selected) =>
              setSelectedLessons((current) => {
                const next = new Set(current);
                for (const id of availableLessonIds) {
                  if (selected) {
                    next.add(id);
                  } else {
                    next.delete(id);
                  }
                }
                return next;
              })
            }
            onToggle={(id, selected) =>
              setSelectedLessons((current) => {
                const next = new Set(current);
                if (selected) {
                  next.add(id);
                } else {
                  next.delete(id);
                }
                return next;
              })
            }
            onRetry={() => void loadLessons(true)}
          />
        ) : (
          <FileList
            files={visibleFiles}
            total={data.files?.length ?? 0}
            error={data.filesError}
            loading={loading.files && !data.files}
            selected={selectedFiles}
            allSelected={allFilesSelected}
            someSelected={someFilesSelected}
            onToggleAll={(selected) =>
              setSelectedFiles((current) => {
                const next = new Set(current);
                for (const file of visibleFiles) {
                  if (selected) {
                    next.add(file.id);
                  } else {
                    next.delete(file.id);
                  }
                }
                return next;
              })
            }
            onToggle={(id, selected) =>
              setSelectedFiles((current) => {
                const next = new Set(current);
                if (selected) {
                  next.add(id);
                } else {
                  next.delete(id);
                }
                return next;
              })
            }
            onRetry={() => void loadFiles(true)}
          />
        )}
      </div>

      <div className="flex shrink-0 items-center gap-4 border-t border-border bg-surface-secondary px-8 py-3">
        <div className="min-w-0 flex-1 text-sm">
          {selectedCount === 0 ? (
            <span className="text-muted">{tab === 'lessons' ? '勾选讲次后点击“下载”。' : '勾选文件后点击“下载”。'}</span>
          ) : (
            <span>
              已选 {selectedCount} {tab === 'lessons' ? '讲' : '个文件'}
              {tab === 'lessons' ? `，共 ${plan.files} 个视频` : ''}
              {plan.bytes > 0 ? ` · 约 ${formatBytes(plan.bytes)}` : ''}
              {plan.unknown > 0 && plan.bytes > 0 ? `（${plan.unknown} 个大小未知）` : ''}
              {plan.unknown > 0 && plan.bytes === 0 ? ' · 正在查询大小…' : ''}
            </span>
          )}
        </div>
        <Button
          variant="primary"
          isDisabled={plan.files === 0 || submitting}
          onPress={() => void download()}
        >
          <Download size={16} />
          {settings?.preferences.ask_destination ? '下载到…' : '下载'}
        </Button>
      </div>
    </>
  );
}

function BackButton({ onPress }: { onPress: () => void }) {
  return (
    <Button variant="ghost" isIconOnly aria-label="返回课程列表" onPress={onPress} className="mt-0.5">
      <ArrowLeft size={18} />
    </Button>
  );
}

function LessonList({
  lessons,
  total,
  error,
  loading,
  selected,
  sizes,
  tracks,
  allSelected,
  someSelected,
  onToggleAll,
  onToggle,
  onRetry,
}: {
  lessons: Lesson[];
  total: number;
  error: string | null;
  loading: boolean;
  selected: Set<string>;
  sizes: SizeMap;
  tracks: Track[];
  allSelected: boolean;
  someSelected: boolean;
  onToggleAll: (selected: boolean) => void;
  onToggle: (id: string, selected: boolean) => void;
  onRetry: () => void;
}) {
  if (error) {
    return <ErrorNotice title="无法读取课堂录像" message={error} onRetry={onRetry} className="mb-4" />;
  }
  if (loading) {
    return <LoadingBlock label="正在从课堂视频平台读取讲次…" />;
  }
  if (total === 0) {
    return (
      <EmptyBlock
        icon={<Video size={40} />}
        title="还没有课堂录像"
        description="这门课程在新版课堂视频平台上没有已开放的录像，或者课程还没有安排直录播。"
      />
    );
  }
  if (lessons.length === 0) {
    return <EmptyBlock title="没有匹配的讲次" />;
  }
  return (
    <div className="overflow-hidden rounded-2xl border border-border">
      <div className="flex items-center gap-3 border-b border-border bg-surface-secondary px-4 py-2 text-xs text-muted">
        <CheckBox
          ariaLabel="全选"
          isSelected={allSelected}
          isIndeterminate={!allSelected && someSelected}
          isDisabled={!lessons.some((lesson) => lesson.available)}
          onChange={onToggleAll}
        />
        <span className="w-8">#</span>
        <span className="flex-1">讲次</span>
        <span className="w-44">时间</span>
        <span className="w-32">教室</span>
        <span className="w-56 text-right">大小</span>
      </div>
      {lessons.map((lesson, index) => {
        const time = formatLessonTime(lesson.begin_time, lesson.end_time);
        const isSelected = selected.has(lesson.video_id);
        return (
          <div
            key={lesson.video_id}
            className={cn(
              'flex items-center gap-3 border-b border-border px-4 py-2.5 text-sm last:border-b-0',
              isSelected ? 'bg-accent-soft/40' : 'hover:bg-surface-secondary',
              !lesson.available && 'opacity-60',
            )}
            onClick={() => lesson.available && onToggle(lesson.video_id, !isSelected)}
          >
            <div onClick={(event) => event.stopPropagation()}>
              <CheckBox
                ariaLabel={`选择 ${lesson.title}`}
                isSelected={isSelected}
                isDisabled={!lesson.available}
                onChange={(value) => onToggle(lesson.video_id, value)}
              />
            </div>
            <span className="w-8 text-xs text-muted">{index + 1}</span>
            <span className="flex min-w-0 flex-1 items-center gap-2">
              <span className="truncate">{lesson.title}</span>
              {lesson.source === 'historical' ? (
                <Chip size="sm" color="default" variant="soft" className="shrink-0" title="迁移前的录像，来自“课堂视频旧版”">
                  旧版
                </Chip>
              ) : null}
            </span>
            <span className="w-44 text-xs text-muted">
              {time.date}
              {time.time ? <span className="ml-2">{time.time}</span> : null}
            </span>
            <span className="w-32 truncate text-xs text-muted">{lesson.classroom}</span>
            <span className="w-56 text-right text-xs text-muted">
              {!lesson.available ? (
                <Chip size="sm" color="warning" variant="soft">
                  未开放
                </Chip>
              ) : isSelected ? (
                <SizeSummary sizes={sizes[lesson.video_id]} tracks={tracks} />
              ) : (
                ''
              )}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function SizeSummary({ sizes, tracks }: { sizes: Partial<Record<Track, TrackSize>> | undefined; tracks: Track[] }) {
  return (
    <span className="inline-flex flex-wrap justify-end gap-x-2">
      {tracks.map((track) => {
        const size = sizes?.[track];
        let text: string;
        if (!size) {
          text = '…';
        } else if (size.status === 'ready') {
          text = formatBytes(size.size);
        } else if (size.status === 'missing') {
          text = '无';
        } else {
          text = '未知';
        }
        return (
          <span key={track} title={TRACK_LABELS[track]}>
            {TRACK_LABELS[track].slice(0, 2)} {text}
          </span>
        );
      })}
    </span>
  );
}

function FileList({
  files,
  total,
  error,
  loading,
  selected,
  allSelected,
  someSelected,
  onToggleAll,
  onToggle,
  onRetry,
}: {
  files: CourseFile[];
  total: number;
  error: string | null;
  loading: boolean;
  selected: Set<string>;
  allSelected: boolean;
  someSelected: boolean;
  onToggleAll: (selected: boolean) => void;
  onToggle: (id: string, selected: boolean) => void;
  onRetry: () => void;
}) {
  if (error) {
    return <ErrorNotice title="无法读取课程文件" message={error} onRetry={onRetry} className="mb-4" />;
  }
  if (loading) {
    return <LoadingBlock label="正在读取课程文件…" />;
  }
  if (total === 0) {
    return <EmptyBlock icon={<FileText size={40} />} title="没有课程文件" description="这门课程没有对学生开放的文件。" />;
  }
  if (files.length === 0) {
    return <EmptyBlock title="没有匹配的文件" />;
  }
  return (
    <div className="overflow-hidden rounded-2xl border border-border">
      <div className="flex items-center gap-3 border-b border-border bg-surface-secondary px-4 py-2 text-xs text-muted">
        <CheckBox
          ariaLabel="全选"
          isSelected={allSelected}
          isIndeterminate={!allSelected && someSelected}
          onChange={onToggleAll}
        />
        <span className="flex-1">文件</span>
        <span className="w-20">类型</span>
        <span className="w-28">更新时间</span>
        <span className="w-24 text-right">大小</span>
      </div>
      {files.map((file) => {
        const isSelected = selected.has(file.id);
        const name = file.display_name.trim() || file.filename;
        return (
          <div
            key={file.id}
            className={cn(
              'flex items-center gap-3 border-b border-border px-4 py-2.5 text-sm last:border-b-0',
              isSelected ? 'bg-accent-soft/40' : 'hover:bg-surface-secondary',
            )}
            onClick={() => onToggle(file.id, !isSelected)}
          >
            <div onClick={(event) => event.stopPropagation()}>
              <CheckBox ariaLabel={`选择 ${name}`} isSelected={isSelected} onChange={(value) => onToggle(file.id, value)} />
            </div>
            <span className="min-w-0 flex-1 truncate" title={file.filename}>
              {name}
            </span>
            <span className="w-20 text-xs text-muted">{fileKind(file)}</span>
            <span className="w-28 text-xs text-muted">{formatDate(file.updated_at)}</span>
            <span className="w-24 text-right text-xs text-muted">{file.size > 0 ? formatBytes(file.size) : '—'}</span>
          </div>
        );
      })}
    </div>
  );
}

export type { Course };
