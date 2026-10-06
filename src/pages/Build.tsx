import { useEffect, useMemo } from 'react';
import { Hammer, RefreshCw } from 'lucide-react';
import { BuildOptionsForm } from '../components/efi/BuildOptionsForm';
import { PlanPreview } from '../components/efi/PlanPreview';
import { ErrorPanel } from '../components/feedback/ErrorPanel';
import { ExportEfiButton } from '../components/review/ExportEfiButton';
import { EmptyState, LoadingState } from '../components/feedback/States';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Progress } from '../components/ui/Progress';
import { PageHeader, Section, StepActions } from '../components/ui/Section';
import { Spinner } from '../components/ui/Spinner';
import { useT } from '../i18n';
import { compatGate } from '../lib/compat';
import { formatPercent } from '../lib/format';
import { macosLabel } from '../lib/macos';
import { hasIntelWifi } from '../lib/profile';
import { buildKey, canReuseIdentity, toBuildOptions, useBuild } from '../stores/build';
import { compatKey, useCompat } from '../stores/compat';
import { useDeploy } from '../stores/deploy';
import { selectTarget, setOptions } from '../stores/flow';
import { useHardware } from '../stores/hardware';
import { TASK_KINDS, useTasks } from '../stores/tasks';
import { useWizard } from '../stores/wizard';

const PLAN_DELAY_MS = 350;

export default function Build() {
  const t = useT();
  const profile = useHardware((s) => s.profile);
  const compat = useCompat();
  const b = useBuild();
  const complete = useWizard((s) => s.complete);
  const goTo = useWizard((s) => s.goTo);
  const buildTask = useTasks((s) => s.latest(TASK_KINDS.build));

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
  const percent = formatPercent(b.progress);
  const runningTask = buildTask?.status === 'running' ? buildTask : null;
  const canCancel = b.building && !b.cancelled && (b.taskId !== null || runningTask !== null);
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
          <Section title={t('build.running')}>
            <div className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <p className="min-w-0 flex-1 truncate text-base text-fg">{b.message ?? t('build.starting')}</p>
                {percent && <span className="text-sm tabular-nums text-fg-3">{percent}</span>}
              </div>
              <Progress value={b.progress} label={t('build.running')} />
              {b.log.length > 1 && (
                <ul className="space-y-0.5 pt-1 font-mono text-xs text-fg-3">
                  {b.log.slice(0, -1).map((line, i) => (
                    <li key={`${i}-${line}`} className="truncate">
                      {line}
                    </li>
                  ))}
                </ul>
              )}
              <div className="flex justify-end">
                <Button size="sm" variant="ghost" onClick={() => void b.cancel()} disabled={!canCancel}>
                  {b.cancelled ? t('build.cancelling') : t('common.cancel')}
                </Button>
              </div>
            </div>
          </Section>
        )}

        {!b.building && b.error && (
          b.cancelled ? (
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
          )
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
