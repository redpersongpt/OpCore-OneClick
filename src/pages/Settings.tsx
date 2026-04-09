import { useCallback, useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { open as openUrl } from '@tauri-apps/plugin-shell';
import { save } from '@tauri-apps/plugin-dialog';
import {
  ExternalLink, FileDown, Loader2, RefreshCw, Settings2, Trash2,
  Bug, Download, CheckCircle, AlertCircle, ArrowUpCircle,
} from 'lucide-react';
import { Modal } from '../components/ui/Modal';
import { Button } from '../components/ui/Button';
import { Badge } from '../components/ui/Badge';
import { api } from '../bridge/invoke';
import { parseError } from '../lib/parseError';

interface SettingsProps {
  open: boolean;
  onClose: () => void;
  onOpenTroubleshoot?: () => void;
}

const GITHUB_URL = 'https://github.com/redpersongpt/OpCore-OneClick';
const GITHUB_API_LATEST = 'https://api.github.com/repos/redpersongpt/OpCore-OneClick/releases/latest';

interface UpdateInfo {
  available: boolean;
  latestVersion: string;
  currentVersion: string;
  releaseUrl: string;
  releaseNotes: string;
  publishedAt: string;
}

export default function Settings({ open, onClose, onOpenTroubleshoot }: SettingsProps) {
  const [appVersion, setAppVersion] = useState('5.0.0');
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [logTail, setLogTail] = useState('');
  const [loadingLog, setLoadingLog] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [clearingCache, setClearingCache] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  // Update checker
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);

  const loadDiagnostics = useCallback(async () => {
    setLoadingLog(true);
    setError(null);

    try {
      const [version, nextSessionId, tail] = await Promise.all([
        getVersion(),
        api.logGetSessionId(),
        api.logGetTail(200),
      ]);
      setAppVersion(version);
      setSessionId(nextSessionId);
      setLogTail(tail);
    } catch (err) {
      setError(parseError(err));
    } finally {
      setLoadingLog(false);
    }
  }, []);

  const checkForUpdates = useCallback(async () => {
    setCheckingUpdate(true);
    setUpdateError(null);

    try {
      const currentVersion = await getVersion();
      const response = await fetch(GITHUB_API_LATEST);

      if (!response.ok) {
        throw new Error(`GitHub API returned ${response.status}`);
      }

      const release = await response.json();
      const latestTag: string = release.tag_name?.replace(/^v/, '') ?? '0.0.0';

      const isNewer = compareVersions(latestTag, currentVersion) > 0;

      setUpdateInfo({
        available: isNewer,
        latestVersion: latestTag,
        currentVersion,
        releaseUrl: release.html_url ?? `${GITHUB_URL}/releases/latest`,
        releaseNotes: release.body?.slice(0, 500) ?? '',
        publishedAt: release.published_at ?? '',
      });
    } catch (err) {
      setUpdateError(parseError(err));
    } finally {
      setCheckingUpdate(false);
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    void loadDiagnostics();
    void checkForUpdates();
  }, [open, loadDiagnostics, checkForUpdates]);

  const handleExportDiagnostics = async () => {
    setExporting(true);
    setError(null);
    setStatus(null);

    try {
      const targetPath = await save({
        defaultPath: `opcore-support-${new Date().toISOString().slice(0, 10)}.log`,
      });

      if (!targetPath) return;

      await api.saveSupportLog(targetPath);
      setStatus(`Saved diagnostics to ${targetPath}`);
    } catch (err) {
      setError(parseError(err));
    } finally {
      setExporting(false);
    }
  };

  const handleClearCache = async () => {
    setClearingCache(true);
    setError(null);
    setStatus(null);

    try {
      await api.clearAppCache();
      setStatus('Cleared generated builds, cached recovery assets, and local download cache.');
    } catch (err) {
      setError(parseError(err));
    } finally {
      setClearingCache(false);
    }
  };

  const handleSendReport = async () => {
    let diagData = '';
    try {
      const [version, sid, tail] = await Promise.all([
        getVersion(),
        api.logGetSessionId(),
        api.logGetTail(80),
      ]);
      diagData = [
        `OpCore-OneClick v${version}`,
        `Session: ${sid}`,
        `Platform: ${navigator.platform}`,
        `Date: ${new Date().toISOString()}`,
        '',
        tail,
      ].join('\n');
    } catch {
      diagData = 'Failed to collect diagnostics';
    }

    const title = encodeURIComponent('Bug Report: [describe your issue]');
    const body = encodeURIComponent(
      [
        '## Description',
        '<!-- Describe the issue in detail -->',
        '',
        '## Steps to Reproduce',
        '1. ',
        '2. ',
        '3. ',
        '',
        '## Expected Behavior',
        '',
        '## Diagnostics (auto-generated)',
        '```',
        diagData,
        '```',
      ].join('\n'),
    );

    const url = `${GITHUB_URL}/issues/new?title=${title}&body=${body}&labels=bug`;
    try {
      await openUrl(url);
    } catch {
      window.open(url, '_blank');
    }
  };

  const handleOpenRelease = async () => {
    if (!updateInfo) return;
    try {
      await openUrl(updateInfo.releaseUrl);
    } catch {
      window.open(updateInfo.releaseUrl, '_blank');
    }
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Settings"
      width="max-w-3xl"
      footer={
        <Button variant="secondary" onClick={onClose}>
          Close
        </Button>
      }
    >
      <div className="space-y-5">
        {/* Application info */}
        <section className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="text-[0.875rem] font-medium text-[--text-primary]">Application</p>
              <p className="text-[0.75rem] text-[--text-tertiary] mt-1">
                Inspect diagnostics, export support logs, and clear cached build artifacts.
              </p>
            </div>
            <Badge variant="info" size="sm" dot>
              v{appVersion}
            </Badge>
          </div>
          {sessionId && (
            <p className="mt-3 text-[0.6875rem] font-mono text-[--text-tertiary]">
              Session {sessionId}
            </p>
          )}
        </section>

        {/* Update checker */}
        <section className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
          <div className="flex items-start justify-between gap-4">
            <div className="flex-1">
              <p className="text-[0.8125rem] font-medium text-[--text-primary] flex items-center gap-2">
                <ArrowUpCircle size={14} className="text-[--text-tertiary]" />
                Updates
              </p>
              {checkingUpdate && (
                <p className="text-[0.6875rem] text-[--text-tertiary] mt-1.5 flex items-center gap-1.5">
                  <Loader2 size={12} className="animate-spin" />
                  Checking for updates...
                </p>
              )}
              {updateError && (
                <p className="text-[0.6875rem] text-[--color-red-5] mt-1.5">
                  Could not check for updates: {updateError}
                </p>
              )}
              {updateInfo && !checkingUpdate && (
                updateInfo.available ? (
                  <div className="mt-2">
                    <div className="flex items-center gap-2 mb-1.5">
                      <Badge variant="warning" size="sm" dot>
                        v{updateInfo.latestVersion} available
                      </Badge>
                      <span className="text-[0.625rem] text-[--text-tertiary]">
                        (you have v{updateInfo.currentVersion})
                      </span>
                    </div>
                    {updateInfo.releaseNotes && (
                      <p className="text-[0.6875rem] text-[--text-tertiary] leading-snug line-clamp-3 mb-2">
                        {updateInfo.releaseNotes.split('\n').slice(0, 3).join(' ')}
                      </p>
                    )}
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={() => void handleOpenRelease()}
                      leadingIcon={<Download size={13} />}
                    >
                      Download Update
                    </Button>
                  </div>
                ) : (
                  <p className="text-[0.6875rem] text-[#22c55e] mt-1.5 flex items-center gap-1.5">
                    <CheckCircle size={12} />
                    You're on the latest version (v{updateInfo.currentVersion})
                  </p>
                )
              )}
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void checkForUpdates()}
              loading={checkingUpdate}
              leadingIcon={!checkingUpdate ? <RefreshCw size={13} /> : undefined}
            >
              Check
            </Button>
          </div>
        </section>

        {/* Diagnostics + Cache row */}
        <section className="grid grid-cols-2 gap-3">
          <div className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
            <p className="text-[0.8125rem] font-medium text-[--text-primary]">Diagnostics</p>
            <p className="text-[0.6875rem] text-[--text-tertiary] mt-1 leading-snug">
              Export the current session logs and environment details for debugging.
            </p>
            <div className="mt-4 flex gap-2">
              <Button
                variant="primary"
                onClick={handleExportDiagnostics}
                loading={exporting}
                leadingIcon={!exporting ? <FileDown size={14} /> : undefined}
              >
                Export Diagnostics
              </Button>
            </div>
          </div>

          <div className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
            <p className="text-[0.8125rem] font-medium text-[--text-primary]">Cache</p>
            <p className="text-[0.6875rem] text-[--text-tertiary] mt-1 leading-snug">
              Remove generated EFIs, cached recovery downloads, and local resource downloads.
            </p>
            <div className="mt-4 flex gap-2">
              <Button
                variant="secondary"
                onClick={handleClearCache}
                loading={clearingCache}
                leadingIcon={!clearingCache ? <Trash2 size={14} /> : undefined}
              >
                Clear Cache
              </Button>
            </div>
          </div>
        </section>

        {/* Send Report + Troubleshoot */}
        <section className="grid grid-cols-2 gap-3">
          <div className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
            <p className="text-[0.8125rem] font-medium text-[--text-primary] flex items-center gap-2">
              <Bug size={14} className="text-[--color-red-5]" />
              Report Issue
            </p>
            <p className="text-[0.6875rem] text-[--text-tertiary] mt-1 leading-snug">
              Open a GitHub issue with your session logs automatically attached.
            </p>
            <div className="mt-4">
              <Button
                variant="secondary"
                onClick={() => void handleSendReport()}
                leadingIcon={<AlertCircle size={14} />}
              >
                Send Bug Report
              </Button>
            </div>
          </div>

          <div className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
            <p className="text-[0.8125rem] font-medium text-[--text-primary] flex items-center gap-2">
              <Bug size={14} className="text-[--text-tertiary]" />
              Troubleshoot
            </p>
            <p className="text-[0.6875rem] text-[--text-tertiary] mt-1 leading-snug">
              Common Hackintosh issues, symptoms, and step-by-step fixes.
            </p>
            <div className="mt-4">
              <Button
                variant="secondary"
                onClick={() => {
                  onClose();
                  onOpenTroubleshoot?.();
                }}
                leadingIcon={<ExternalLink size={14} />}
              >
                Open Troubleshoot
              </Button>
            </div>
          </div>
        </section>

        {/* Log Viewer */}
        <section className="rounded-lg border border-[--border-subtle] bg-[--surface-1] overflow-hidden">
          <div className="flex items-center justify-between border-b border-[--border-subtle] px-4 py-3">
            <div>
              <p className="text-[0.8125rem] font-medium text-[--text-primary]">Log Viewer</p>
              <p className="text-[0.6875rem] text-[--text-tertiary] mt-0.5">
                Last 200 lines from the active log file.
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void loadDiagnostics()}
              leadingIcon={loadingLog ? <Loader2 size={14} className="animate-spin" /> : <RefreshCw size={14} />}
            >
              Refresh
            </Button>
          </div>
          <div className="max-h-[320px] overflow-auto px-4 py-3">
            <pre className="whitespace-pre-wrap break-words font-mono text-[0.6875rem] leading-5 text-[--text-secondary]">
              {loadingLog ? 'Loading logs...' : logTail || 'No log output available.'}
            </pre>
          </div>
        </section>

        {/* About */}
        <section className="rounded-lg border border-[--border-subtle] bg-[--surface-1] px-4 py-3">
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="text-[0.8125rem] font-medium text-[--text-primary]">About</p>
              <p className="text-[0.6875rem] text-[--text-tertiary] mt-1 leading-snug">
                OpCore-OneClick is a Tauri-based Hackintosh EFI generator built around
                OpenCore.
              </p>
            </div>
            <Settings2 size={16} className="text-[--text-tertiary] shrink-0" />
          </div>
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noreferrer"
            className="mt-4 inline-flex items-center gap-2 text-[0.75rem] text-[--color-blue-6] hover:text-[--color-blue-5]"
          >
            Project on GitHub
            <ExternalLink size={13} />
          </a>
        </section>

        {status && (
          <div className="rounded-lg border border-[--color-green-3] bg-[--color-green-1] px-4 py-3 text-[0.75rem] text-[--color-green-7]">
            {status}
          </div>
        )}

        {error && (
          <div className="rounded-lg border border-[--color-red-3] bg-[--color-red-1] px-4 py-3 text-[0.75rem] text-[--color-red-7]">
            {error}
          </div>
        )}
      </div>
    </Modal>
  );
}

/** Compare two semver strings. Returns >0 if a > b, <0 if a < b, 0 if equal. */
function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const na = pa[i] ?? 0;
    const nb = pb[i] ?? 0;
    if (na !== nb) return na - nb;
  }
  return 0;
}
