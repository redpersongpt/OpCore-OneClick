import { LogLevel } from './logger.js';

/**
 * Internal Failure Simulation Layer
 * Controlled via SIM_MODE environment variable.
 */

export type SimFailureKind = 
  | 'disk:write-fail'
  | 'usb:disconnect'
  | 'download:interrupted'
  | 'download:corrupt'
  | 'hw:invalid-data'
  | 'hw:timeout';

class SimulationManager {
  private activeSims: Set<SimFailureKind> = new Set();

  constructor() {
    const envSims = process.env.SIM_MODE?.split(',') || [];
    for (const s of envSims) {
      if (s.trim()) this.activeSims.add(s.trim() as SimFailureKind);
    }
  }

  isEnabled(kind: SimFailureKind): boolean {
    return this.activeSims.has(kind);
  }

  /**
   * Helper to log a simulated event.
   */
  logSim(log: (level: LogLevel, ctx: string, msg: string, data?: any) => void, kind: SimFailureKind, message: string) {
    log('WARN', 'SIMULATION', `[${kind}] ${message}`, { simMode: true });
  }

  /**
   * Simulate a random delay or immediate error.
   */
  async failIf(kind: SimFailureKind, message: string): Promise<void> {
    if (this.isEnabled(kind)) {
      // Small jitter to make it feel "real"
      await new Promise(r => setTimeout(r, Math.random() * 500 + 200));
      throw new Error(`[SIMULATED_FAILURE] ${message}`);
    }
  }
}

export const sim = new SimulationManager();
