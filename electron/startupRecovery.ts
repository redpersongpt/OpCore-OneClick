import { redactSensitiveText } from '../src/lib/diagnosticRedaction.js';
import type { IssueReportDraft, PublicDiagnosticsSnapshot } from './releaseDiagnostics.js';

export const DID_FAIL_LOAD_ERR_ABORTED = -3;
export const RENDERER_READY_TIMEOUT_MS = 8_000;
export const MAX_MAIN_FRAME_LOAD_RETRIES = 1;

export type StartupFailureKind =
  | 'missing_assets'
  | 'load_rejected'
  | 'did_fail_load'
  | 'renderer_process_gone'
  | 'renderer_boot_timeout';

export interface DidFailLoadContext {
  errorCode: number;
  errorDescription: string;
  validatedURL: string;
  isMainFrame: boolean;
}

export type DidFailLoadAction = 'ignore' | 'retry' | 'recover';

export interface StartupFailurePageInput {
  kind: StartupFailureKind;
  diagnostics: PublicDiagnosticsSnapshot;
  issueDraft: IssueReportDraft;
  retryTargetUrl?: string | null;
  safeTargetUrl?: string | null;
  preloadExists?: boolean;
  indexExists?: boolean;
  errorCode?: number;
  errorDescription?: string;
  validatedURL?: string;
  reason?: string;
  exitCode?: number;
  detail?: string | null;
}

export type StartupFailureEventInput = Omit<StartupFailurePageInput, 'diagnostics' | 'issueDraft'>;

export interface StartupFailureDescriptor {
  title: string;
  summary: string;
  likelyCause: string;
  nextActions: string[];
  technicalSummary: string;
  failureMessage: string;
}

export function shouldIgnoreDidFailLoad(context: DidFailLoadContext): boolean {
  if (!context.isMainFrame) return true;
  if (context.errorCode === DID_FAIL_LOAD_ERR_ABORTED) return true;
  if (context.validatedURL.startsWith('devtools://')) return true;
  if (context.validatedURL.startsWith('chrome-error://')) return false;
  if (context.validatedURL.startsWith('data:text/html')) return true;
  return false;
}

export function determineDidFailLoadAction(
  context: DidFailLoadContext,
  retryCount: number,
): DidFailLoadAction {
  if (shouldIgnoreDidFailLoad(context)) return 'ignore';
  if (retryCount < MAX_MAIN_FRAME_LOAD_RETRIES) return 'retry';
  return 'recover';
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function scriptString(value: string): string {
  return JSON.stringify(value);
}

function encodeInlineValue(value: string): string {
  return Buffer.from(value, 'utf8').toString('base64');
}

function formatSnapshotSummary(snapshot: PublicDiagnosticsSnapshot): string[] {
  const lines = [
    `Trigger: ${snapshot.trigger}`,
    `Session: ${snapshot.sessionFingerprint}`,
    `Platform: ${snapshot.platform} ${snapshot.arch}`,
  ];

  if (snapshot.compatibilityState) {
    lines.push(`Compatibility: ${snapshot.compatibilityState.level} -> ${snapshot.compatibilityState.recommendedVersion}`);
  }

  if (snapshot.lastTaskKind || snapshot.lastTaskStatus) {
    lines.push(`Last task: ${snapshot.lastTaskKind ?? 'unknown'} (${snapshot.lastTaskStatus ?? 'unknown'})`);
  }

  if (snapshot.lastError) {
    lines.push(`Last error: ${snapshot.lastError}`);
  }

  return lines;
}

export function describeStartupFailure(input: StartupFailurePageInput): StartupFailureDescriptor {
  switch (input.kind) {
    case 'missing_assets': {
      const missing: string[] = [];
      if (input.preloadExists === false) missing.push('preload bridge');
      if (input.indexExists === false) missing.push('renderer bundle');
      const missingText = missing.length > 0 ? missing.join(' and ') : 'required startup assets';
      return {
        title: 'Startup failed',
        summary: 'The packaged interface could not start because required app files were missing.',
        likelyCause: `The installed bundle is incomplete or damaged. The app could not find the ${missingText}.`,
        nextActions: [
          'Retry the app once in case the install was interrupted.',
          'If the problem repeats, reinstall the app from a fresh release download.',
          'Copy the report and open an issue if the new install shows the same error.',
        ],
        technicalSummary: redactSensitiveText(`Missing assets: preload=${String(input.preloadExists)} renderer=${String(input.indexExists)}`),
        failureMessage: `Startup assets missing: ${missingText}.`,
      };
    }
    case 'load_rejected':
      return {
        title: 'Startup failed',
        summary: 'The app found its packaged interface but Electron rejected the initial load.',
        likelyCause: 'The renderer entry was present, but the startup navigation failed before the interface became usable.',
        nextActions: [
          'Retry the app once.',
          'If this keeps happening, reinstall the app from a clean build.',
          'Use Copy Report and Open Issue so the startup diagnostics can be reviewed.',
        ],
        technicalSummary: redactSensitiveText(input.detail ?? 'Renderer entry rejected during loadFile/loadURL.'),
        failureMessage: redactSensitiveText(input.detail ?? 'Renderer entry rejected during startup.'),
      };
    case 'did_fail_load':
      return {
        title: 'Startup failed',
        summary: 'Electron could not finish loading the main interface.',
        likelyCause: 'The main renderer navigation failed before the UI could paint. This is usually caused by a bad packaged asset path, a missing renderer file, or a startup-time browser load error.',
        nextActions: [
          'Retry the app once.',
          'If the failure repeats, use Back to Safety to start with cleared saved state.',
          'If that still fails, reinstall the app and attach the copied report to an issue.',
        ],
        technicalSummary: redactSensitiveText(`did-fail-load (${input.errorCode ?? 'unknown'}): ${input.errorDescription ?? 'unknown'} @ ${input.validatedURL ?? 'unknown'}`),
        failureMessage: redactSensitiveText(`Renderer navigation failed: ${input.errorDescription ?? 'unknown error'}.`),
      };
    case 'renderer_process_gone':
      return {
        title: 'Interface stopped unexpectedly',
        summary: 'The renderer process exited before the app could recover normally.',
        likelyCause: 'The UI process crashed or exited during startup, so the app fell back to a recovery screen instead of leaving a blank or black window.',
        nextActions: [
          'Retry the app once.',
          'If the app opens but returns here again, use Back to Safety to clear saved UI state.',
          'Copy the report and open an issue if the renderer keeps exiting.',
        ],
        technicalSummary: redactSensitiveText(`renderer-process-gone: reason=${input.reason ?? 'unknown'} exitCode=${String(input.exitCode ?? 'unknown')}`),
        failureMessage: redactSensitiveText(`Renderer process exited during startup (${input.reason ?? 'unknown'}).`),
      };
    case 'renderer_boot_timeout':
    default:
      return {
        title: 'Interface did not finish starting',
        summary: 'The window loaded but the app UI never reported itself ready.',
        likelyCause: 'The HTML shell loaded, but the preload bridge or renderer bootstrap failed before React could finish mounting.',
        nextActions: [
          'Retry the app once.',
          'Use Back to Safety to clear saved state and start from the welcome screen.',
          'If the timeout repeats, copy the report and open an issue.',
        ],
        technicalSummary: redactSensitiveText(input.detail ?? 'Renderer ready handshake timed out.'),
        failureMessage: redactSensitiveText(input.detail ?? 'Renderer startup handshake timed out.'),
      };
  }
}

export function buildStartupFailurePageUrl(input: StartupFailurePageInput): string {
  const descriptor = describeStartupFailure(input);
  const snapshotSummary = formatSnapshotSummary(input.diagnostics);
  const issueUrl = `https://github.com/redpersongpt/macOS-One-Click/issues/new?title=${encodeURIComponent(input.issueDraft.title)}&labels=bug`;
  const reportBody = input.issueDraft.body;
  const retryTarget = input.retryTargetUrl ?? '';
  const safeTarget = input.safeTargetUrl ?? retryTarget;
  const retryTargetEncoded = encodeInlineValue(retryTarget);
  const safeTargetEncoded = encodeInlineValue(safeTarget);

  const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>${escapeHtml(descriptor.title)}</title>
    <style>
      :root {
        color-scheme: dark;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        --bg: #050505;
        --panel: rgba(255, 255, 255, 0.05);
        --border: rgba(255, 255, 255, 0.12);
        --muted: rgba(245, 245, 245, 0.65);
        --soft: rgba(245, 245, 245, 0.42);
        --accent: #6ee7b7;
        --warn: #fbbf24;
        --danger: #fb7185;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(110, 231, 183, 0.08), transparent 28%),
          radial-gradient(circle at bottom right, rgba(251, 113, 133, 0.08), transparent 30%),
          var(--bg);
        color: #f5f5f5;
        display: grid;
        place-items: center;
        padding: 28px;
      }
      main {
        width: min(880px, 100%);
        display: grid;
        gap: 18px;
      }
      .hero, .panel {
        border: 1px solid var(--border);
        background: var(--panel);
        border-radius: 24px;
        box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
        backdrop-filter: blur(18px);
      }
      .hero {
        padding: 28px;
      }
      .hero h1 {
        margin: 0 0 10px;
        font-size: 30px;
        line-height: 1.1;
      }
      .hero p {
        margin: 0;
        line-height: 1.7;
        color: var(--muted);
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
        gap: 16px;
      }
      .panel {
        padding: 18px 20px;
      }
      .label {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        margin: 0 0 10px;
        font-size: 11px;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        color: var(--soft);
      }
      .panel h2 {
        margin: 0 0 8px;
        font-size: 18px;
      }
      .panel p, .panel li {
        color: var(--muted);
        line-height: 1.65;
      }
      ul {
        margin: 0;
        padding-left: 18px;
      }
      .actions {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
        gap: 10px;
      }
      button, a.button {
        appearance: none;
        border: 1px solid var(--border);
        border-radius: 16px;
        padding: 13px 14px;
        background: rgba(255, 255, 255, 0.05);
        color: #f5f5f5;
        font-weight: 700;
        text-decoration: none;
        cursor: pointer;
        transition: transform 120ms ease, background 120ms ease, border-color 120ms ease;
        text-align: center;
      }
      button:hover, a.button:hover {
        transform: translateY(-1px);
        background: rgba(255, 255, 255, 0.09);
      }
      button.primary {
        border-color: rgba(110, 231, 183, 0.28);
        background: rgba(110, 231, 183, 0.12);
      }
      button.warning {
        border-color: rgba(251, 191, 36, 0.28);
        background: rgba(251, 191, 36, 0.12);
      }
      button.secondary {
        border-color: rgba(251, 113, 133, 0.24);
        background: rgba(251, 113, 133, 0.1);
      }
      button:disabled {
        opacity: 0.45;
        cursor: not-allowed;
        transform: none;
      }
      details {
        border-top: 1px solid rgba(255, 255, 255, 0.08);
        padding-top: 12px;
      }
      summary {
        cursor: pointer;
        color: #f5f5f5;
        font-weight: 700;
      }
      pre {
        white-space: pre-wrap;
        word-break: break-word;
        background: rgba(0, 0, 0, 0.25);
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 16px;
        padding: 14px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.6;
      }
      .status {
        min-height: 20px;
        color: var(--accent);
        font-size: 12px;
      }
      .foot {
        color: var(--soft);
        font-size: 12px;
      }
      @media (max-width: 700px) {
        body { padding: 16px; }
        .hero, .panel { border-radius: 20px; }
      }
    </style>
  </head>
  <body>
    <main>
      <section class="hero">
        <div class="label">Startup Recovery</div>
        <h1>${escapeHtml(descriptor.title)}</h1>
        <p>${escapeHtml(descriptor.summary)}</p>
      </section>

      <section class="grid">
        <article class="panel">
          <div class="label">Why It Likely Failed</div>
          <h2>${escapeHtml(descriptor.failureMessage)}</h2>
          <p>${escapeHtml(descriptor.likelyCause)}</p>
        </article>
        <article class="panel">
          <div class="label">What To Do Next</div>
          <ul>
            ${descriptor.nextActions.map((action) => `<li>${escapeHtml(action)}</li>`).join('')}
          </ul>
        </article>
      </section>

      <section class="panel">
        <div class="label">Recovery Actions</div>
        <div class="actions">
          <button class="primary" id="retry-button"${retryTarget ? '' : ' disabled'}>Retry</button>
          <button id="copy-button">Copy Report</button>
          <a class="button secondary" id="issue-link" href="${escapeHtml(issueUrl)}" target="_blank" rel="noopener noreferrer">Open Issue</a>
          <button class="warning" id="safe-button"${safeTarget ? '' : ' disabled'}>Back to Safety</button>
        </div>
        <p class="foot">Destructive operations remain locked. This screen only helps you recover or report the failure.</p>
        <div class="status" id="status-text" aria-live="polite"></div>
      </section>

      <section class="panel">
        <div class="label">Clean Diagnostics</div>
        <ul>
          ${snapshotSummary.map((line) => `<li>${escapeHtml(line)}</li>`).join('')}
        </ul>
        <details>
          <summary>Technical detail</summary>
          <pre>${escapeHtml(descriptor.technicalSummary)}</pre>
        </details>
      </section>
    </main>

    <script>
      const reportBody = ${scriptString(reportBody)};
      const retryTarget = atob(${scriptString(retryTargetEncoded)});
      const safeTarget = atob(${scriptString(safeTargetEncoded)});
      const statusNode = document.getElementById('status-text');

      async function copyReport() {
        try {
          if (navigator.clipboard?.writeText) {
            await navigator.clipboard.writeText(reportBody);
          } else {
            const el = document.createElement('textarea');
            el.value = reportBody;
            document.body.appendChild(el);
            el.select();
            document.execCommand('copy');
            document.body.removeChild(el);
          }
          statusNode.textContent = 'Report copied. Paste it into the GitHub issue body.';
        } catch (error) {
          statusNode.textContent = 'Could not copy automatically. Open Issue and paste the diagnostics manually.';
        }
      }

      document.getElementById('copy-button')?.addEventListener('click', copyReport);
      document.getElementById('retry-button')?.addEventListener('click', () => {
        if (retryTarget) window.location.replace(retryTarget);
      });
      document.getElementById('safe-button')?.addEventListener('click', () => {
        if (safeTarget) window.location.replace(safeTarget);
      });
      document.getElementById('issue-link')?.addEventListener('click', () => {
        copyReport();
      });
    </script>
  </body>
</html>`;

  return `data:text/html;charset=UTF-8,${encodeURIComponent(html)}`;
}
