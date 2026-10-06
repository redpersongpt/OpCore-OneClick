/** Phases emitted in `flash:progress`, in order (see contracts.rs `FlashProgress`). */
export function flashMilestones(withRecovery: boolean): string[] {
  return ['prepare', 'partition', 'format', 'copy-efi', ...(withRecovery ? ['copy-recovery'] : []), 'verify'];
}

export type MilestoneState = 'done' | 'active' | 'failed' | 'pending';
export type FlashRunState = 'idle' | 'running' | 'done' | 'failed';

/** State of each milestone given the phases seen so far. Unknown phases never break the list. */
export function milestoneStates(
  milestones: readonly string[],
  seen: readonly string[],
  status: FlashRunState,
): MilestoneState[] {
  if (status === 'done') return milestones.map(() => 'done');
  let current = -1;
  for (const phase of seen) {
    const idx = milestones.indexOf(phase);
    if (idx > current) current = idx;
  }
  return milestones.map((_, i) => {
    if (i < current) return 'done';
    if (i === current) return status === 'failed' ? 'failed' : 'active';
    if (current === -1 && i === 0 && status === 'running') return 'active';
    return 'pending';
  });
}
