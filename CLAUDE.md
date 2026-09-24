@AGENTS.md

## Claude Code

- Use plan mode for significant or multi-file changes.
- After a change, run the focused test group before the full suite
  (`cargo test --test <group> <name>`).
- If a sim result looks wrong, reproduce it with a minimal SV case before editing.
- Re-verify determinism (`+seed=1` twice, diff the output) for RNG/solver changes.
- When behavior is ambiguous, treat the existing test suite as the source of truth.
