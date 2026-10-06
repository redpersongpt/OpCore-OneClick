import type { ArtifactStatus, NoteLevel, SupportLevel } from '../bridge/types';
import type { Tone } from '../components/ui/Badge';
import type { CheckState } from './firmware';

export const SUPPORT_TONE: Record<SupportLevel, Tone> = {
  supported: 'success',
  partial: 'warning',
  unsupported: 'danger',
  unknown: 'neutral',
};

export const NOTE_BADGE_TONE: Record<NoteLevel, Tone> = {
  info: 'info',
  warning: 'warning',
  blocking: 'danger',
};

export const ARTIFACT_TONE: Record<ArtifactStatus, Tone> = {
  downloaded: 'success',
  cached: 'success',
  bundled: 'info',
  generated: 'info',
  skipped: 'warning',
  failed: 'danger',
};

export const CHECK_TONE: Record<CheckState, Tone> = {
  ok: 'success',
  action: 'danger',
  inferred: 'info',
  unknown: 'neutral',
  na: 'neutral',
};
