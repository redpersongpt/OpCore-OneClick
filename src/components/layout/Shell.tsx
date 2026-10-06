import { useEffect, useRef, type ReactNode } from 'react';
import { useWizard } from '../../stores/wizard';
import Header from './Header';
import Sidebar from './Sidebar';
import TaskBar from './TaskBar';

export default function Shell({ children }: { children: ReactNode }) {
  const step = useWizard((s) => s.step);
  const mainRef = useRef<HTMLElement>(null);

  // Every step starts at the top; the scroll container outlives the pages.
  useEffect(() => {
    if (mainRef.current) mainRef.current.scrollTop = 0;
  }, [step]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-fg">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <Header />
        <main ref={mainRef} className="relative flex-1 overflow-y-auto">
          <div className="mx-auto max-w-[760px] px-6 py-6">{children}</div>
        </main>
        <TaskBar />
      </div>
    </div>
  );
}
