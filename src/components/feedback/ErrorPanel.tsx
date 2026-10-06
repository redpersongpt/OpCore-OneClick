import { useState, type ReactNode } from 'react';
import { AlertOctagon, Check, Copy } from 'lucide-react';
import { describeError, UNKNOWN_MESSAGE, type AppError } from '../../bridge/errors';
import { useI18n } from '../../i18n';
import { copyText } from '../../lib/external';
import { errorHint } from '../../lib/labels';

/** Shows a backend error with its suggestion and code, plus caller-provided actions. */
export function ErrorPanel({
  error,
  title,
  actions,
  compact = false,
}: {
  error: AppError;
  title?: ReactNode;
  actions?: ReactNode;
  compact?: boolean;
}) {
  const { t, lang } = useI18n();
  const [copied, setCopied] = useState(false);
  // Errors raised by the frontend itself (not by a backend command) are translated here.
  const local = error.code === 'IPC_UNAVAILABLE';
  const message = local ? t('error.ipcUnavailable') : error.message === UNKNOWN_MESSAGE ? t('error.unknown') : error.message;
  // Backend text is English. For well-known codes other languages get translated
  // advice first; the backend's own (often more specific) suggestion follows.
  const hint = local ? null : errorHint(error.code);
  let suggestion = local ? t('error.ipcUnavailableHint') : error.suggestion;
  let detail: string | null = null;
  if (hint && (lang !== 'en' || !suggestion)) {
    detail = lang !== 'en' ? suggestion : null;
    suggestion = t(hint);
  }
  const tone = error.severity === 'warning' ? 'border-warn-line bg-warn-soft' : 'border-err-line bg-err-soft';
  const iconTone = error.severity === 'warning' ? 'text-warn' : 'text-err';

  return (
    <div role="alert" className={`rounded-md border ${tone} ${compact ? 'px-3 py-2.5' : 'px-4 py-3.5'}`}>
      <div className="flex gap-3">
        <AlertOctagon size={compact ? 14 : 16} className={`mt-px shrink-0 ${iconTone}`} aria-hidden />
        <div className="min-w-0 flex-1">
          {title && <p className="text-base font-semibold text-fg">{title}</p>}
          <p className={`break-words text-fg-2 ${title ? 'mt-0.5 text-sm' : 'text-base'}`}>{message}</p>
          {suggestion && (
            <p className="mt-1.5 text-sm text-fg">
              <span className="font-medium">{t('error.suggestion')}: </span>
              {suggestion}
            </p>
          )}
          {detail && <p className="mt-0.5 text-xs text-fg-3">{detail}</p>}
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {actions}
            <button
              type="button"
              onClick={async () => {
                if (await copyText(describeError(error))) {
                  setCopied(true);
                  window.setTimeout(() => setCopied(false), 1500);
                }
              }}
              className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-2xs text-fg-3 hover:bg-panel-2 hover:text-fg-2"
              title={t('error.copy')}
            >
              {copied ? <Check size={11} aria-hidden /> : <Copy size={11} aria-hidden />}
              {error.code}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
