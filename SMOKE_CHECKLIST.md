# Destructive Path Smoke Checklist

- Verify EFI target contains `EFI/OC/OpenCore.efi` after flash.
- Verify recovery target contains `com.apple.recovery.boot/BaseSystem.dmg` after flash.
- Verify flash aborts when the selected disk identity no longer matches the captured identity.
- Verify recovery download resumes after interruption and app restart.
- Verify partition path resolution uses the correct target partition on macOS, Windows, and Linux before any write step.
