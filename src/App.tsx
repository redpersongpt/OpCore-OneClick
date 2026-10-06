import { useEffect, type ComponentType } from 'react';
import { MotionConfig } from 'motion/react';
import { ErrorBoundary } from './components/feedback/ErrorBoundary';
import CloseConfirmDialog from './components/layout/CloseConfirmDialog';
import Shell from './components/layout/Shell';
import { useBackendEvents } from './hooks/useBackendEvents';
import { useCloseGuard } from './hooks/useCloseGuard';
import { usePersistence } from './hooks/usePersistence';
import Bios from './pages/Bios';
import Build from './pages/Build';
import Compatibility from './pages/Compatibility';
import Complete from './pages/Complete';
import Deploy from './pages/Deploy';
import Hardware from './pages/Hardware';
import Review from './pages/Review';
import Scan from './pages/Scan';
import Settings from './pages/Settings';
import Troubleshoot from './pages/Troubleshoot';
import Welcome from './pages/Welcome';
import { useApp } from './stores/app';
import { useWizard, type Step } from './stores/wizard';

const PAGES: Record<Step, ComponentType> = {
  welcome: Welcome,
  scan: Scan,
  hardware: Hardware,
  compatibility: Compatibility,
  bios: Bios,
  build: Build,
  review: Review,
  deploy: Deploy,
  complete: Complete,
};

export default function App() {
  const step = useWizard((s) => s.step);
  const init = useApp((s) => s.init);

  useBackendEvents();
  usePersistence();
  useCloseGuard();

  useEffect(() => {
    void init();
  }, [init]);

  const Page = PAGES[step];

  return (
    <MotionConfig reducedMotion="user">
      <Shell>
        <ErrorBoundary key={step}>
          <div className="animate-fade-in">
            <Page />
          </div>
        </ErrorBoundary>
      </Shell>
      <Settings />
      <Troubleshoot />
      <CloseConfirmDialog />
    </MotionConfig>
  );
}
