# Architecture Map

## Renderer / main-process boundary

### Renderer
- `src/App.tsx`
  - top-level wizard orchestration
  - persisted-state restore
  - step progression
  - compatibility / BIOS / validation / deploy gate consumption
  - destructive confirmation modal coordination
  - recovery resume / retry / manual import UX
- `src/components/steps/*`
  - visual step rendering
  - do not own destructive behavior
- `src/lib/releaseFlow.ts`
  - restore / resume / target-change decisions
- `src/lib/stateMachine.ts`
  - typed BIOS/release machines
  - shared build/deploy guard derivation helpers
- `src/lib/installStepGuards.ts`
  - centralized step-transition eligibility rules for the renderer shell

### Main process
- `electron/main.ts`
  - IPC registration
  - application lifecycle wiring
  - service orchestration across hardware scan, BIOS, build, validation, recovery, flashing, diagnostics, tasks
- `electron/configGenerator.ts`
  - OpenCore config generation
  - BIOS recommendations
  - resource selection
- `electron/compatibility.ts`
  - hardware/macOS compatibility evaluation
- `electron/configValidator.ts`
  - EFI integrity and dependency validation
- `electron/bios/*`
  - BIOS orchestrator, session persistence, verification, vendor backends
- `electron/diskOps.ts`
  - disk enumeration and low-level disk operations

## IPC domains in `electron/main.ts`

### State persistence
- `get-persisted-state`
- `save-state`
- `clear-state`

### BIOS / firmware
- `bios:get-state`
- `bios:apply-supported`
- `bios:verify-manual`
- `bios:restart-to-firmware`
- `bios:clear-session`
- `bios:resume-state`
- `bios:restart-capability`
- `probe-bios`
- `probe-firmware`
- `restart-to-bios`

### Flow guards
- `flow:guard-build`
- `flow:guard-deploy`

### Hardware / build / resources
- `scan-hardware`
- `build-efi`
- `fetch-latest-kexts`
- `validate-efi`
- `enable-production-lock`

### Recovery
- `download-recovery`
- `recovery:import`
- `recovery:get-cached-info`
- `recovery:clear-cache`

### Disk / deploy
- `list-usb-devices`
- `get-hard-drives`
- `get-disk-info`
- `shrink-partition`
- `create-boot-partition`
- `flash-usb`

### Preflight / deterministic / failures
- `run-preflight`
- `run-prechecks`
- `preflight:run`
- `preflight:record-failure`
- `preflight:should-skip-retry`
- `preflight:failure-memory`
- `preflight:clear-memory`
- `deterministic:simulate-build`
- `deterministic:dry-run-recovery`
- `deterministic:verify-build-state`
- `deterministic:verify-efi-success`
- `deterministic:verify-recovery-success`

### Diagnostics / tasks / logs
- `get-diagnostics`
- `task:cancel`
- `task:list`
- `open-folder`
- `get-log-path`
- `log:get-tail`
- `log:get-ops-tail`
- `log:get-session-id`
- `log:clear`
- `report-issue`
- `renderer-error` (event)

## Highest-risk files by maintenance cost
- `src/App.tsx`
  - 2700+ lines, mixes step UI, restore logic, destructive guard orchestration, diagnostics, task listeners, and modal control.
- `electron/main.ts`
  - nearly 2900 lines, registers every IPC channel and still owns multiple domain behaviors directly.
- `electron/configGenerator.ts`
  - large branching surface, version-sensitive OpenCore config semantics, high regression risk.
- `electron/configValidator.ts`
  - safety-critical gate before deployment.
- `electron/diskOps.ts`
  - low-level disk identity / destructive disk operation path.

## Refactor seam chosen for this iteration
- Extract renderer step-transition safety rules from `src/App.tsx` into `src/lib/installStepGuards.ts`.
- Rationale:
  - pure logic
  - safety-critical
  - easy to regression test
  - reduces the chance of future UI cleanup silently changing destructive behavior
