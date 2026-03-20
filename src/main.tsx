import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import App from './App.tsx';
import './index.css';

// Belt-and-suspenders renderer crash capture (preload sets window.onerror first;
// these catch anything that slips through after React mounts).
function emitRuntimeError(type: string, message: string) {
  window.dispatchEvent(new CustomEvent('moc:runtime-error', { detail: { type, message } }));
}

window.addEventListener('error', (e) => {
  emitRuntimeError('error', e.message);
  if (import.meta.env.DEV) {
    console.error('[renderer] uncaught error:', e.message, e.error?.stack);
  }
});
window.addEventListener('unhandledrejection', (e) => {
  const message = e.reason instanceof Error ? e.reason.message : String(e.reason);
  emitRuntimeError('unhandledrejection', message);
  if (import.meta.env.DEV) {
    console.error('[renderer] unhandled rejection:', e.reason);
  }
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
