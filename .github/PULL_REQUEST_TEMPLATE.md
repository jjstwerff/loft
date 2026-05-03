## Summary

<!-- 1–3 sentences on what changed and why.  The "why" matters more than the "what" — the diff already shows what. -->

## Test plan

<!-- Tick what applies; add specifics. -->

- [ ] `make ci` (fmt + clippy + test) passes locally
- [ ] New behaviour has a regression test under `tests/`
- [ ] Doc tests under `tests/docs/` updated if user-facing
- [ ] `make wasm-html-test` passes if browser/WASM is touched
- [ ] `make wrap` passes if heap / lifetime is touched

## Notes for the reviewer

<!-- Anything non-obvious about the approach, trade-offs you considered, follow-ups you deliberately deferred. -->
