import React, { useEffect, useState } from 'react';
import { motion } from 'motion/react';
import { useWizard } from './stores/wizard';
import { useTasks } from './stores/tasks';
import { onTaskUpdate } from './bridge/events';
import Shell from './components/layout/Shell';
import { ErrorBoundary } from './components/feedback';
import Welcome from './pages/Welcome';
import Scan from './pages/Scan';
import Compatibility from './pages/Compatibility';
import Prerequisites from './pages/Prerequisites';
import BiosSetup from './pages/BiosSetup';
import Build from './pages/Build';
import Review from './pages/Review';
import Deploy from './pages/Deploy';
import Complete from './pages/Complete';
import Settings from './pages/Settings';
import Troubleshoot from './pages/Troubleshoot';

const pages: Record<string, React.ComponentType> = {
  welcome: Welcome,
  scan: Scan,
  compatibility: Compatibility,
  prerequisites: Prerequisites,
  bios: BiosSetup,
  build: Build,
  review: Review,
  deploy: Deploy,
  complete: Complete,
};

export default function App() {
  const step = useWizard((s) => s.step);
  const handleTaskUpdate = useTasks((s) => s.handleUpdate);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [troubleshootOpen, setTroubleshootOpen] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onTaskUpdate(handleTaskUpdate).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [handleTaskUpdate]);

  const Page = pages[step] ?? Welcome;

  return (
    <>
      <Shell onOpenSettings={() => setSettingsOpen(true)}>
        <motion.div
          key={step}
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.2, ease: 'easeInOut' }}
        >
          <ErrorBoundary key={step}>
            <Page />
          </ErrorBoundary>
        </motion.div>
      </Shell>
      <Settings
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onOpenTroubleshoot={() => setTroubleshootOpen(true)}
      />
      <Troubleshoot
        open={troubleshootOpen}
        onClose={() => setTroubleshootOpen(false)}
      />
    </>
  );
}
