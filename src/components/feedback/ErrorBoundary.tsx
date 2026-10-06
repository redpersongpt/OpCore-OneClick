import { Component, type ErrorInfo, type ReactNode } from 'react';
import { AlertOctagon } from 'lucide-react';
import { useT } from '../../i18n';
import { Button } from '../ui/Button';

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    if (import.meta.env.DEV) console.error(error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return <Fallback error={this.state.error} onReset={() => this.setState({ error: null })} />;
  }
}

function Fallback({ error, onReset }: { error: Error; onReset: () => void }) {
  const t = useT();
  return (
    <div className="flex flex-col items-center gap-3 px-6 py-16 text-center">
      <AlertOctagon size={22} className="text-err" aria-hidden />
      <p className="text-md font-semibold text-fg">{t('error.boundaryTitle')}</p>
      <p className="max-w-md break-words font-mono text-sm text-fg-3">{error.message}</p>
      <Button size="sm" onClick={onReset}>
        {t('common.retry')}
      </Button>
    </div>
  );
}
