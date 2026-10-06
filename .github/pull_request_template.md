## Summary

- What problem does this change solve?
- What changed?

## Validation

- [ ] `npx tsc --noEmit`
- [ ] `npx vitest run`
- [ ] `npm run build`
- [ ] `cd src-tauri && cargo clippy --all-targets -- -D warnings`
- [ ] `cd src-tauri && cargo test` (commit any regenerated files in `src/bridge/generated`)

If you skipped anything, say why.

## Screenshots

Add screenshots for UI changes, if relevant.

## Risk Notes

Call out anything that touches:

- disk listing or flashing
- hardware detection
- the build plan or config.plist generation
- pinned downloads (versions, URLs, SHA-256)
- recovery download/cache
