import type { ValidationResult } from '../bridge/types';

export type Verdict = 'passed' | 'warnings' | 'failed';

export function validationVerdict(v: ValidationResult): Verdict {
  if (!v.valid || v.issues.some((i) => i.level === 'blocking')) return 'failed';
  if (v.issues.some((i) => i.level === 'warning')) return 'warnings';
  return 'passed';
}

export type ProfileSource = 'scan' | 'manual' | 'imported' | 'demo';

export function profileSource(source: string): ProfileSource {
  return source === 'manual' || source === 'imported' || source === 'demo' ? source : 'scan';
}
