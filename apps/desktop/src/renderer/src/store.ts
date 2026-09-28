import { create } from 'zustand';

import type {
  AccountInfo,
  Course,
  DownloadCounts,
  DownloadInfo,
  DownloadProgress,
  EngineNotification,
  EngineState,
  LoginStatus,
  Preferences,
  SettingsInfo,
} from '@shared/protocol';

import { engine, errorMessage } from './api';
import { isUnfinished } from './format';

export type Page = 'courses' | 'downloads' | 'settings';

interface AppStore {
  engine: EngineState | null;
  account: AccountInfo | null;
  settings: SettingsInfo | null;
  login: LoginStatus | null;
  page: Page;
  courseId: string | null;
  courses: Course[] | null;
  coursesError: string | null;
  coursesLoading: boolean;
  downloads: Record<string, DownloadInfo>;
  downloadsLoaded: boolean;
  downloadsError: string | null;

  bootstrap(): Promise<void>;
  navigate(page: Page): void;
  openCourse(courseId: string): void;
  closeCourse(): void;
  loadCourses(force?: boolean): Promise<void>;
  loadDownloads(): Promise<void>;
  savePreferences(preferences: Preferences): Promise<void>;
  logout(): Promise<void>;
  restartEngine(): Promise<void>;
}

let subscribed = false;

export const useStore = create<AppStore>((set, get) => ({
  engine: null,
  account: null,
  settings: null,
  login: null,
  page: 'courses',
  courseId: null,
  courses: null,
  coursesError: null,
  coursesLoading: false,
  downloads: {},
  downloadsLoaded: false,
  downloadsError: null,

  async bootstrap() {
    if (!subscribed) {
      subscribed = true;
      window.canvas.onEngineState((state) => applyEngineState(state));
      window.canvas.onNotification((notification) => applyNotification(notification));
    }
    applyEngineState(await window.canvas.engineState());
  },

  navigate(page) {
    set({ page });
  },

  openCourse(courseId) {
    set({ page: 'courses', courseId });
  },

  closeCourse() {
    set({ courseId: null });
  },

  async loadCourses(force = false) {
    if (get().coursesLoading || (get().courses && !force)) {
      return;
    }
    set({ coursesLoading: true, coursesError: null });
    try {
      const courses = await engine.courses();
      set({ courses, coursesLoading: false });
    } catch (error) {
      set({ coursesError: errorMessage(error), coursesLoading: false });
    }
  },

  async loadDownloads() {
    try {
      const list = await engine.listDownloads('all');
      const downloads: Record<string, DownloadInfo> = {};
      for (const item of list.items) {
        downloads[item.id] = item;
      }
      set({ downloads, downloadsLoaded: true, downloadsError: null });
    } catch (error) {
      set({ downloadsError: errorMessage(error), downloadsLoaded: true });
    }
  },

  async savePreferences(preferences) {
    const settings = await engine.updateSettings(preferences);
    set({ settings });
  },

  async logout() {
    const account = await engine.logout();
    set({ account });
  },

  async restartEngine() {
    set({ engine: { ...(get().engine ?? emptyState()), status: 'starting', error: null } });
    applyEngineState(await window.canvas.restartEngine());
  },
}));

function emptyState(): EngineState {
  return { status: 'starting', error: null, version: null, dataDir: '', settings: null, account: null };
}

function applyEngineState(state: EngineState): void {
  const previous = useStore.getState();
  const becameRunning = state.status === 'running' && previous.engine?.status !== 'running';
  useStore.setState({
    engine: state,
    settings: state.settings ?? previous.settings,
    account: state.account ?? (state.status === 'running' ? previous.account : previous.account),
  });
  if (becameRunning) {
    // A restarted engine has a fresh download list and may have lost the login.
    useStore.setState({ downloadsLoaded: false, courses: null });
    void useStore.getState().loadDownloads();
    if (state.account?.authenticated) {
      void verifyAccount();
    }
  }
}

/** Confirms a restored login with Canvas; an expired one ends here. */
async function verifyAccount(): Promise<void> {
  try {
    const account = await engine.account(true);
    useStore.setState({ account });
  } catch {
    // Offline: keep the saved login; requests report their own errors.
  }
}

function applyNotification(notification: EngineNotification): void {
  const state = useStore.getState();
  switch (notification.method) {
    case 'login.status':
      useStore.setState({ login: notification.params as LoginStatus });
      break;
    case 'account.changed': {
      const account = notification.params as AccountInfo;
      const wasSignedIn = state.account?.authenticated ?? false;
      useStore.setState({ account });
      if (!account.authenticated && wasSignedIn) {
        useStore.setState({ courses: null, courseId: null, page: 'courses' });
      }
      if (account.authenticated && !wasSignedIn) {
        useStore.setState({ courses: null, login: null });
        void state.loadDownloads();
      }
      break;
    }
    case 'download.changed': {
      const info = notification.params as DownloadInfo;
      useStore.setState({ downloads: { ...state.downloads, [info.id]: info } });
      break;
    }
    case 'download.progress': {
      const progress = notification.params as DownloadProgress;
      const current = state.downloads[progress.id];
      if (current) {
        useStore.setState({
          downloads: {
            ...state.downloads,
            [progress.id]: { ...current, received: progress.received, total: progress.total, speed: progress.speed },
          },
        });
      }
      break;
    }
    case 'download.removed': {
      const { id } = notification.params as { id: string };
      if (state.downloads[id]) {
        const downloads = { ...state.downloads };
        delete downloads[id];
        useStore.setState({ downloads });
      }
      break;
    }
    default:
      break;
  }
}

export function countDownloads(downloads: Record<string, DownloadInfo>): DownloadCounts {
  const counts: DownloadCounts = { all: 0, active: 0, running: 0, completed: 0, failed: 0 };
  for (const item of Object.values(downloads)) {
    counts.all += 1;
    if (isUnfinished(item.status)) {
      counts.active += 1;
    }
    if (item.status === 'queued' || item.status === 'downloading') {
      counts.running += 1;
    }
    if (item.status === 'completed') {
      counts.completed += 1;
    }
    if (item.status === 'failed' || item.status === 'cancelled') {
      counts.failed += 1;
    }
  }
  return counts;
}
