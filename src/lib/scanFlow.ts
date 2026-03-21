export type ScanSuccessStep = 'version-select' | 'report';

export function resolveScanSuccessStep(input: unknown): ScanSuccessStep {
  return input === 'report' || input === 'version-select'
    ? input
    : 'version-select';
}
