import { useEffect, useMemo, useRef } from 'react';
import { Hammer, RefreshCw } from 'lucide-react';
import { BuildOptionsForm } from '../components/efi/BuildOptionsForm';
import { BuildProgressView } from '../components/efi/BuildProgressView';
import { PlanPreview } from '../components/efi/PlanPreview';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { ExportEfiButton } from '../components/review/ExportEfiButton';
import { EmptyState, LoadingState } from '../components/feedback/States';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { PageHeader, Section, StepActions } from '../components/ui/Section';
import { Spinner } from '../components/ui/Spinner';
import { useT } from '../i18n';
import { compatGate } from '../lib/compat';
import { macosLabel } from '../lib/macos';
import { hasIntelWifi } from '../lib/profile';
import { buildKey, canReuseIdentity, toBuildOptions, useBuild } from '../stores/build';
import { compatKey, useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import { selectTarget, setOptions } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { isCancellable, TASK_KINDS, useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

const PLAN_DELAY_MS = 350;

export default function Build() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const compat = useCompat();
  const b = useBuild();
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);
  const runningTask = useTasks((s) => s.running(TASK_KINDS.build));
  const cancelState = useTasks((s) => {
    const id = b.taskId ?? runningTask?.taskId;
    return id ? s.cancels[id] : undefined;
  });
  const progressRef = useRef<HTMLDivElement>(null);

  const target = compat.target;
  // The identity never changes the key, so the plan can be looked up first and
  // then used to decide whether the previous serials still fit its model.
  const key = useMemo(
    () => (target ? buildKey(profile, toBuildOptions(b.draft, target, null)) : null),
    [profile, b.draft, target],
  );
  const plan = key !== null && b.planKey === key ? b.plan : null;
  const plannedModel = plan?.smbios.model ?? null;
  const options = useMemo(
    () => (target ? toBuildOptions(b.draft, target, b.identity, plannedModel) : null),
    [b.draft, target, b.identity, plannedModel],
  );
  const cKey = compatKey(profile, target);
  const { check } = compat;
  const { loadPlan } = b;

  // Keep the compatibility verdict in sync with a target chosen on this page.
  useEffect(() => {
    if (!profile || !target || compat.loading || compat.requestKey === cKey) return;
    void check(profile, target);
  }, [profile, target, cKey, compat.loading, compat.requestKey, check]);

  // Preview the plan for the current options (debounced while typing).
  useEffect(() => {
    if (!profile || !options || !key || b.building || b.planRequestKey === key) return;
    const timer = window.setTimeout(() => void loadPlan(profile, options), PLAN_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [profile, options, key, b.building, b.planRequestKey, loadPlan]);

  // The progress sits below the plan: bring it into view when a build starts.
  useEffect(() => {
    if (b.building) progressRef.current?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' });
  }, [b.building]);

  if (!profile || !target || !options) {
    return (
      <EmptyState
        title={t('bios.noTarget')}
        action={<Button onClick={() => goTo('compatibility')}>{t('bios.goCompat')}</Button>}
      />
    );
  }

  const compatFresh = compat.reportKey === cKey && !compat.loading;
  const gate = compatFresh ? compatGate(compat.report, target) : null;
  const gateOpen = gate === 'ok' || (gate === 'expert' && compat.expertFor === target);
  const resultFresh = b.result !== null && b.resultKey === key;
  // From the save phase on the backend refuses to cancel; do not offer it.
  const task = runningTask && (b.taskId === null || runningTask.taskId === b.taskId) ? runningTask : null;
  // A cancel sent from the task bar counts as well.
  const cancelling = b.cancelled || cancelState === 'requested';
  const cancelRefused = b.building && (b.cancelRefused || cancelState === 'refused');
  const canCancel = b.building && !cancelling && !cancelRefused && task !== null && isCancellable(task);
  const outcome = b.building ? 'running' : b.cancelled ? 'cancelled' : b.error ? 'failed' : 'done';
  const progressView = (
    <BuildProgressView
      detail={b.detail}
      seen={b.phasesSeen}
      items={b.items}
      progress={b.progress}
      message={b.message}
      outcome={outcome}
    />
  );
  // Building downloads everything, so the plan preview must have succeeded first.
  const canBuild = gateOpen && plan !== null && !b.building;

  const startBuild = async () => {
    // A rebuild replaces the current EFI, so later steps must be redone.
    useWizard.getState().invalidateFrom('build');
    useDeploy.getState().resetFlash();
    const result = await b.build(profile, options);
    if (result) complete('build');
  };

  return (
    <>
      <PageHeader title={t('build.title')} subtitle={t('build.subtitle', { version: macosLabel(target) })} />
      <div className="space-y-4">
        {b.previousEfiPath && !b.result && !b.building && (
          <Banner tone="info" title={t('build.previousEfi')}>
            <p className="font-mono break-all">{b.previousEfiPath}</p>
            <div className="mt-2">
              <ExportEfiButton efiPath={b.previousEfiPath} />
            </div>
          </Banner>
        )}
        <Section title={t('build.options')}>
          <BuildOptionsForm
            draft={b.draft}
            onChange={setOptions}
            target={target}
            onTarget={(v) => selectTarget(v, 'build')}
            report={compat.report}
            smbios={plan?.smbios ?? b.plan?.smbios ?? null}
            identity={b.identity}
            identityReusable={canReuseIdentity(b.draft, b.identity, plannedModel) || !b.draft.keepIdentity}
            plannedModel={plannedModel}
            showIntelWifi={hasIntelWifi(profile)}
            disabled={b.building}
          />
        </Section>

        {compat.loading && (
          <div className="flex items-center gap-2 text-sm text-fg-3">
            <Spinner size={14} /> {t('build.checkingTarget')}
          </div>
        )}
        {gate === 'blocked' && (
          <Banner
            tone="danger"
            title={t('build.targetBlocked', { version: macosLabel(target) })}
            actions={<Button size="sm" onClick={() => goTo('compatibility')}>{t('bios.goCompat')}</Button>}
          >
            {compat.report?.summary}
          </Banner>
        )}
        {gate === 'expert' && compat.expertFor !== target && (
          <Banner
            tone="warning"
            title={t('build.targetExpert', { version: macosLabel(target) })}
            actions={<Button size="sm" onClick={() => goTo('compatibility')}>{t('bios.goCompat')}</Button>}
          >
            {t('build.targetExpertBody')}
          </Banner>
        )}

        <Section
          title={t('build.plan')}
          description={t('build.planHint')}
          actions={
            <Button
              size="sm"
              variant="ghost"
              icon={<RefreshCw />}
              onClick={() => void loadPlan(profile, options)}
              loading={b.planLoading}
              disabled={b.building}
            >
              {t('build.refreshPlan')}
            </Button>
          }
        >
          {b.planError && b.planRequestKey === key ? (
            <ErrorPanel error={b.planError} title={t('build.planFailed')} compact />
          ) : plan ? (
            <PlanPreview plan={plan} />
          ) : (
            <LoadingState message={t('build.planning')} />
          )}
        </Section>

        {b.building && (
          <div ref={progressRef}>
            <Section
              title={t('build.running')}
              actions={
                <Button size="sm" variant="ghost" onClick={() => void b.cancel()} disabled={!canCancel}>
                  {cancelling ? t('build.cancelling') : t('common.cancel')}
                </Button>
              }
            >
              {progressView}
              {cancelRefused && <p className="mt-2 text-xs text-fg-3">{t('build.cancelRefused')}</p>}
            </Section>
          </div>
        )}

        {!b.building && b.error && (
          <>
            {b.cancelled ? (
              <Banner tone="info">{t('build.cancelled')}</Banner>
            ) : (
              <ErrorPanel
                error={b.error}
                title={t('build.failed')}
                actions={
                  <Button size="sm" variant="primary" onClick={() => void startBuild()} disabled={!canBuild}>
                    {t('common.retry')}
                  </Button>
                }
              />
            )}
            {b.detail && <Section title={t('build.lastRun')}>{progressView}</Section>}
          </>
        )}

        {!b.building && resultFresh && (
          <Banner
            tone="success"
            title={t('build.done')}
            actions={
              <Button size="sm" variant="primary" onClick={() => goTo('review')}>
                {t('build.toReview')}
              </Button>
            }
          >
            <span className="font-mono">{b.result?.efiPath}</span>
          </Banner>
        )}
      </div>

      <StepActions left={<Button onClick={() => goTo('bios')} disabled={b.building}>{t('common.back')}</Button>}>
        {gateOpen && !plan && !b.building && <span className="text-sm text-fg-3">{t('build.needPlan')}</span>}
        <Button
          variant={resultFresh ? 'secondary' : 'primary'}
          icon={<Hammer />}
          onClick={() => void startBuild()}
          disabled={!canBuild}
          loading={b.building}
        >
          {resultFresh ? t('build.rebuild') : t('build.start')}
        </Button>
      </StepActions>
    </>
  );
}
