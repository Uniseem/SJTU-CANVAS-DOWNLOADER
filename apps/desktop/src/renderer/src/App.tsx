import { useEffect } from 'react';

import { Toast } from '@heroui/react';

import { EngineGate } from './components/EngineGate';
import { Sidebar } from './components/Sidebar';
import { CoursePage } from './pages/CoursePage';
import { CoursesPage } from './pages/CoursesPage';
import { DownloadsPage } from './pages/DownloadsPage';
import { LoginPage } from './pages/LoginPage';
import { SettingsPage } from './pages/SettingsPage';
import { useStore } from './store';

function useSystemTheme(): void {
  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = (): void => {
      document.documentElement.classList.toggle('dark', media.matches);
      document.documentElement.classList.toggle('light', !media.matches);
    };
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, []);
}

export default function App() {
  const engineStatus = useStore((state) => state.engine?.status ?? 'starting');
  const authenticated = useStore((state) => state.account?.authenticated ?? false);
  const page = useStore((state) => state.page);
  const courseId = useStore((state) => state.courseId);
  useSystemTheme();

  useEffect(() => {
    void useStore.getState().bootstrap();
  }, []);

  let content;
  if (engineStatus !== 'running') {
    content = <EngineGate />;
  } else if (!authenticated) {
    content = <LoginPage />;
  } else {
    let body;
    if (page === 'downloads') {
      body = <DownloadsPage />;
    } else if (page === 'settings') {
      body = <SettingsPage />;
    } else if (courseId) {
      body = <CoursePage key={courseId} courseId={courseId} />;
    } else {
      body = <CoursesPage />;
    }
    content = (
      <div className="flex h-full">
        <Sidebar />
        <main className="flex min-w-0 flex-1 flex-col">{body}</main>
      </div>
    );
  }

  return (
    <>
      {content}
      <Toast.Provider placement="bottom" />
    </>
  );
}
