import { Avatar, Chip, cn } from '@heroui/react';
import { BookOpen, Download, Settings } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';

import { countDownloads, useStore, type Page } from '../store';

const ITEMS: { page: Page; label: string; icon: typeof BookOpen }[] = [
  { page: 'courses', label: '课程', icon: BookOpen },
  { page: 'downloads', label: '下载', icon: Download },
  { page: 'settings', label: '设置', icon: Settings },
];

export function Sidebar() {
  const { page, account, downloads, navigate, closeCourse } = useStore(
    useShallow((state) => ({
      page: state.page,
      account: state.account,
      downloads: state.downloads,
      navigate: state.navigate,
      closeCourse: state.closeCourse,
    })),
  );
  const counts = countDownloads(downloads);
  const profile = account?.profile;

  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-border bg-surface-secondary pt-2">
      <nav className="flex flex-1 flex-col gap-1 px-3" aria-label="主导航">
        {ITEMS.map(({ page: target, label, icon: Icon }) => {
          const active = page === target;
          return (
            <button
              key={target}
              type="button"
              aria-current={active ? 'page' : undefined}
              onClick={() => {
                if (target === 'courses') {
                  closeCourse();
                }
                navigate(target);
              }}
              className={cn(
                'flex h-9 items-center gap-3 rounded-xl px-3 text-sm transition-colors outline-none focus-visible:ring-2 focus-visible:ring-focus',
                active
                  ? 'bg-accent-soft font-medium text-accent-soft-foreground'
                  : 'text-foreground/80 hover:bg-surface-tertiary',
              )}
            >
              <Icon size={17} className={active ? '' : 'text-muted'} />
              <span className="flex-1 text-left">{label}</span>
              {target === 'downloads' && counts.active > 0 ? (
                <Chip size="sm" color="accent" variant="soft">
                  {counts.active}
                </Chip>
              ) : null}
            </button>
          );
        })}
      </nav>
      {profile ? (
        <button
          type="button"
          onClick={() => navigate('settings')}
          className="m-3 flex items-center gap-3 rounded-xl px-3 py-2 text-left outline-none hover:bg-surface-tertiary focus-visible:ring-2 focus-visible:ring-focus"
          title="账户与设置"
        >
          <Avatar size="sm">
            {profile.avatar_url ? <Avatar.Image src={profile.avatar_url} alt="" /> : null}
            <Avatar.Fallback>{profile.name.slice(0, 1)}</Avatar.Fallback>
          </Avatar>
          <div className="min-w-0">
            <div className="truncate text-sm font-medium">{profile.name}</div>
            <div className="truncate text-[11px] text-muted">已登录 Canvas</div>
          </div>
        </button>
      ) : null}
    </aside>
  );
}
