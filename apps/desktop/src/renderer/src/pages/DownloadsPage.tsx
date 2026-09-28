import { useEffect, useMemo, useState } from 'react';

import { AlertDialog, Button, Chip, ProgressBar, SearchField, Tabs, cn, toast } from '@heroui/react';
import type { Key } from 'react-aria-components';
import {
  CircleCheck,
  Download,
  FileText,
  FolderOpen,
  Pause,
  Play,
  RotateCw,
  Trash2,
  Video,
  X,
} from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';

import type { DownloadFilter, DownloadInfo } from '@shared/protocol';

import { engine, errorMessage, isMac, shell } from '../api';
import { EmptyBlock, ErrorNotice, LoadingBlock, PageHeader } from '../components/common';
import { STATUS_LABELS, formatBytes, formatEta, formatSpeed, isUnfinished, trackLabel } from '../format';
import { countDownloads, useStore } from '../store';

const FILTERS: { key: DownloadFilter; label: string }[] = [
  { key: 'all', label: '全部' },
  { key: 'active', label: '进行中' },
  { key: 'completed', label: '已完成' },
  { key: 'failed', label: '失败或取消' },
];

const STATUS_ORDER: Record<DownloadInfo['status'], number> = {
  downloading: 0,
  queued: 1,
  paused: 2,
  failed: 3,
  cancelled: 4,
  completed: 5,
};

function matches(item: DownloadInfo, filter: DownloadFilter): boolean {
  switch (filter) {
    case 'active':
      return isUnfinished(item.status);
    case 'completed':
      return item.status === 'completed';
    case 'failed':
      return item.status === 'failed' || item.status === 'cancelled';
    default:
      return true;
  }
}

export function DownloadsPage() {
  const { downloads, loaded, error, load, settings } = useStore(
    useShallow((state) => ({
      downloads: state.downloads,
      loaded: state.downloadsLoaded,
      error: state.downloadsError,
      load: state.loadDownloads,
      settings: state.settings,
    })),
  );
  const [filter, setFilter] = useState<DownloadFilter>('all');
  const [query, setQuery] = useState('');
  const [cancelCandidate, setCancelCandidate] = useState<DownloadInfo | null>(null);

  useEffect(() => {
    void load();
  }, [load]);

  const counts = countDownloads(downloads);
  const items = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return Object.values(downloads)
      .filter((item) => matches(item, filter))
      .filter((item) => !needle || `${item.display_name} ${item.title} ${item.course_name}`.toLowerCase().includes(needle))
      .sort((left, right) => {
        const byStatus = STATUS_ORDER[left.status] - STATUS_ORDER[right.status];
        if (byStatus !== 0 && filter === 'all') {
          return byStatus;
        }
        return right.created_at.localeCompare(left.created_at);
      });
  }, [downloads, filter, query]);

  const run = async (action: () => Promise<unknown>, failure: string): Promise<void> => {
    try {
      await action();
    } catch (problem) {
      toast.danger(failure, { description: errorMessage(problem) });
    }
  };

  const countOf = (key: DownloadFilter): number =>
    key === 'all' ? counts.all : key === 'active' ? counts.active : key === 'completed' ? counts.completed : counts.failed;

  return (
    <>
      <PageHeader
        title="下载"
        subtitle={
          counts.running > 0
            ? `${counts.running} 个任务正在下载，同时下载 ${settings?.preferences.concurrency ?? '-'} 个文件`
            : counts.all > 0
              ? `${counts.completed} 个已完成`
              : '在课程中选择课堂录像或课程文件后，下载任务会出现在这里'
        }
        actions={
          <>
            <Button
              variant="secondary"
              size="sm"
              isDisabled={counts.running === 0}
              onPress={() => void run(() => engine.pauseAll(), '无法暂停')}
            >
              <Pause size={14} />
              全部暂停
            </Button>
            <Button
              variant="secondary"
              size="sm"
              isDisabled={!Object.values(downloads).some((item) => item.status === 'paused')}
              onPress={() => void run(() => engine.resumeAll(), '无法继续')}
            >
              <Play size={14} />
              全部继续
            </Button>
            <Button
              variant="secondary"
              size="sm"
              isDisabled={counts.completed === 0}
              onPress={() => void run(() => engine.clearCompleted(), '无法清除')}
            >
              <CircleCheck size={14} />
              清除已完成
            </Button>
            <Button
              variant="secondary"
              size="sm"
              isIconOnly
              aria-label="打开下载文件夹"
              onPress={() => {
                if (settings?.preferences.download_dir) {
                  void shell.openPath(settings.preferences.download_dir);
                }
              }}
            >
              <FolderOpen size={16} />
            </Button>
          </>
        }
      />
      <div className="flex shrink-0 flex-wrap items-center gap-3 px-8 pb-3">
        <Tabs selectedKey={filter} onSelectionChange={(key: Key) => setFilter(key as DownloadFilter)} variant="secondary" aria-label="筛选下载">
          <Tabs.ListContainer>
            <Tabs.List aria-label="筛选下载">
              {FILTERS.map(({ key, label }) => (
                <Tabs.Tab key={key} id={key} className="whitespace-nowrap">
                  {label}
                  {countOf(key) > 0 ? <span className="ml-1 text-xs text-muted">{countOf(key)}</span> : null}
                  <Tabs.Indicator />
                </Tabs.Tab>
              ))}
            </Tabs.List>
          </Tabs.ListContainer>
        </Tabs>
        <div className="flex-1" />
        <SearchField aria-label="搜索下载" value={query} onChange={setQuery} className="w-60" variant="secondary">
          <SearchField.Group>
            <SearchField.SearchIcon />
            <SearchField.Input placeholder="搜索任务或课程" />
            <SearchField.ClearButton />
          </SearchField.Group>
        </SearchField>
      </div>

      <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-8 pb-8">
        {error ? <ErrorNotice title="无法读取下载列表" message={error} onRetry={() => void load()} className="mb-4" /> : null}
        {!loaded && !error ? <LoadingBlock /> : null}
        {loaded && items.length === 0 ? (
          <EmptyBlock
            icon={<Download size={40} />}
            title={
              filter === 'active'
                ? '没有进行中的下载'
                : filter === 'completed'
                  ? '还没有完成的下载'
                  : filter === 'failed'
                    ? '没有失败或取消的下载'
                    : query
                      ? '没有匹配的任务'
                      : '还没有下载任务'
            }
            description={filter === 'all' && !query ? '在课程中选择课堂录像或课程文件，点“下载”后会出现在这里。' : undefined}
          />
        ) : null}
        <div className="flex flex-col gap-2">
          {items.map((item) => (
            <DownloadRow
              key={item.id}
              item={item}
              onPause={() => void run(() => engine.pause(item.id), '无法暂停')}
              onResume={() => void run(() => engine.resume(item.id), '无法继续')}
              onRetry={() => void run(() => engine.retry(item.id), '无法重试')}
              onCancel={() => {
                if (item.received > 0) {
                  setCancelCandidate(item);
                } else {
                  void run(() => engine.cancel(item.id), '无法取消');
                }
              }}
              onRemove={() => void run(() => engine.remove(item.id), '无法移除')}
              onOpen={() => {
                if (item.file_path) {
                  void shell.openPath(item.file_path).then((problem) => {
                    if (problem) {
                      toast.danger('无法打开文件', { description: problem });
                    }
                  });
                }
              }}
              onReveal={() => {
                if (item.file_path) {
                  void shell.showInFolder(item.file_path);
                } else {
                  void shell.openPath(item.destination);
                }
              }}
            />
          ))}
        </div>
      </div>

      <AlertDialog isOpen={cancelCandidate !== null} onOpenChange={(open) => !open && setCancelCandidate(null)}>
        <AlertDialog.Backdrop>
          <AlertDialog.Container size="sm">
            <AlertDialog.Dialog>
              <AlertDialog.Header>
                <AlertDialog.Heading>取消下载？</AlertDialog.Heading>
              </AlertDialog.Header>
              <AlertDialog.Body>
                <p className="text-sm text-muted">
                  {cancelCandidate?.display_name}：已下载的部分会被删除，之后可以重新下载。
                </p>
              </AlertDialog.Body>
              <AlertDialog.Footer>
                <Button variant="tertiary" onPress={() => setCancelCandidate(null)}>
                  继续下载
                </Button>
                <Button
                  variant="danger"
                  onPress={() => {
                    const candidate = cancelCandidate;
                    setCancelCandidate(null);
                    if (candidate) {
                      void run(() => engine.cancel(candidate.id), '无法取消');
                    }
                  }}
                >
                  取消下载
                </Button>
              </AlertDialog.Footer>
            </AlertDialog.Dialog>
          </AlertDialog.Container>
        </AlertDialog.Backdrop>
      </AlertDialog>
    </>
  );
}

function statusColor(status: DownloadInfo['status']): 'accent' | 'default' | 'success' | 'warning' | 'danger' {
  switch (status) {
    case 'downloading':
      return 'accent';
    case 'completed':
      return 'success';
    case 'paused':
      return 'warning';
    case 'failed':
      return 'danger';
    case 'cancelled':
      return 'default';
    default:
      return 'default';
  }
}

function DownloadRow({
  item,
  onPause,
  onResume,
  onRetry,
  onCancel,
  onRemove,
  onOpen,
  onReveal,
}: {
  item: DownloadInfo;
  onPause: () => void;
  onResume: () => void;
  onRetry: () => void;
  onCancel: () => void;
  onRemove: () => void;
  onOpen: () => void;
  onReveal: () => void;
}) {
  const percent = item.total && item.total > 0 ? Math.min(100, (item.received / item.total) * 100) : 0;
  const eta = formatEta(item);
  const unfinished = isUnfinished(item.status);
  const Icon = item.kind === 'video' ? Video : FileText;
  const detail =
    item.status === 'downloading'
      ? `${formatBytes(item.received)} / ${item.total ? formatBytes(item.total) : '未知'} · ${formatSpeed(item.speed)}${eta ? ` · 剩余 ${eta}` : ''}`
      : item.status === 'completed'
        ? formatBytes(item.total ?? item.received)
        : item.status === 'queued'
          ? item.error
            ? item.error
            : `等待中${item.received > 0 ? ` · 已下载 ${formatBytes(item.received)}` : ''}`
          : item.status === 'paused'
            ? `已暂停${item.received > 0 ? ` · ${formatBytes(item.received)}${item.total ? ` / ${formatBytes(item.total)}` : ''}` : ''}`
            : item.status === 'failed'
              ? item.error ?? '下载失败'
              : '已取消';

  return (
    <div className="flex items-center gap-4 rounded-2xl border border-border px-4 py-3">
      <div
        className={cn(
          'flex size-9 shrink-0 items-center justify-center rounded-xl',
          item.kind === 'video' ? 'bg-accent-soft text-accent-soft-foreground' : 'bg-default-soft text-default-soft-foreground',
        )}
      >
        <Icon size={18} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium" title={item.file_path ?? undefined}>
            {item.kind === 'video' ? `${item.title} · ${trackLabel(item.track)}` : item.display_name}
          </span>
          <Chip size="sm" color={statusColor(item.status)} variant="soft" className="shrink-0">
            {STATUS_LABELS[item.status]}
          </Chip>
        </div>
        <div className="mt-0.5 truncate text-xs text-muted">{item.course_name}</div>
        {unfinished ? (
          <ProgressBar
            aria-label="下载进度"
            value={percent}
            minValue={0}
            maxValue={100}
            isIndeterminate={item.status === 'downloading' && !item.total}
            size="sm"
            color={item.status === 'paused' ? 'warning' : 'accent'}
            className="mt-2"
          >
            <ProgressBar.Track>
              <ProgressBar.Fill />
            </ProgressBar.Track>
          </ProgressBar>
        ) : null}
        <div className={cn('mt-1 truncate text-xs', item.status === 'failed' ? 'text-danger' : 'text-muted')}>{detail}</div>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {item.status === 'downloading' || item.status === 'queued' ? (
          <Button variant="ghost" size="sm" isIconOnly aria-label="暂停" onPress={onPause}>
            <Pause size={16} />
          </Button>
        ) : null}
        {item.status === 'paused' ? (
          <Button variant="ghost" size="sm" isIconOnly aria-label="继续" onPress={onResume}>
            <Play size={16} />
          </Button>
        ) : null}
        {item.status === 'failed' || item.status === 'cancelled' ? (
          <Button variant="ghost" size="sm" isIconOnly aria-label="重新下载" onPress={onRetry}>
            <RotateCw size={16} />
          </Button>
        ) : null}
        {item.status === 'completed' ? (
          <>
            <Button variant="ghost" size="sm" onPress={onOpen}>
              打开
            </Button>
            <Button variant="ghost" size="sm" isIconOnly aria-label={isMac ? '在访达中显示' : '在文件夹中显示'} onPress={onReveal}>
              <FolderOpen size={16} />
            </Button>
          </>
        ) : null}
        {unfinished ? (
          <Button variant="ghost" size="sm" isIconOnly aria-label="取消下载" onPress={onCancel}>
            <X size={16} />
          </Button>
        ) : (
          <Button variant="ghost" size="sm" isIconOnly aria-label="从列表移除" onPress={onRemove}>
            <Trash2 size={16} />
          </Button>
        )}
      </div>
    </div>
  );
}
