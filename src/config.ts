// ── Application-wide safety configuration ────────────────────────────────────
//
// BEGINNER_SAFETY_MODE controls the default protective posture of the app.
// When true:
//  - Destructive actions are blocked unless ALL safety checks pass
//  - Advanced override paths require explicit typed confirmation
//  - Low-confidence states block rather than guess
//  - Advanced options are collapsed by default
//
// This is a compile-time constant. Setting it to false downgrades safety gates
// for advanced/developer use only.

export const BEGINNER_SAFETY_MODE = true;

// Minimum typed confirmation required for destructive disk operations.
// Users must type the disk identifier exactly as displayed.
export const DESTRUCTIVE_CONFIRM_REQUIRED = true;

// Confidence threshold below which a device is hard-blocked from selection.
// Devices that cannot be positively identified as removable are blocked.
export const MIN_DEVICE_CONFIDENCE: 'high' | 'medium' | 'low' = 'medium';

// Timeout in milliseconds for IPC operations that load data (firmware probe,
// hardware scan, USB list refresh, download resume check).
export const STEP_LOAD_TIMEOUT_MS = 30_000;
