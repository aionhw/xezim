# Developer Documentation

How an agent or engineer uses these docs. The top-level `AGENTS.md` is the
canonical guide — this set is the deep dive it points to.

Pick a doc by what you are doing:

| Doc | When to read it |
|---|---|
| `architecture.md` | First time in this repo; before adding a feature |
| `debugging.md` | A simulation gives a wrong result or hangs |
| `testing.md` | Before writing or running any test |
| `gotchas.md` | Before editing `src/` or `xezim-core` |

The companion `scripts/dev/` toolbox wraps the common actions:

- `quick-repro.sh '<sv-snippet>' [args…]` — simulate a one-off snippet without
  writing a repo file.
- `check-determinism.sh design.sv [args…]` — run twice with the same seed and
  fail on any drift.
- `new-test.sh <group> <name>` — scaffold a test and wire its `#[path]` mod line
  into the group root.

These docs relate to the guides as follows: `AGENTS.md` (top level) is the
general orientation — what to build and what must never break; `docs/dev/*` is
the working detail for a specific activity; `../xezim-core/AGENTS.md` covers the
sibling library (parser/elaboration) that this repo depends on.
