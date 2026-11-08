# xezim — Development Guide for AI Coding Agents

This file is read by coding agents at the start of a session. This repository
holds the bytecode compiler, the simulator and the `xezim` command line. The
preprocessor, parser, elaborator and value model live in the sibling
[xezim-core](https://github.com/aionhw/xezim-core) repository; read its
`AGENTS.md` (`../xezim-core/AGENTS.md`) before changing anything there. If a
fact here conflicts with the code or the test suite, they win; fix this file.

## Where things are

- `docs/building.md`: build, profile-guided build, "Working on xezim-core".
- `CONTRIBUTING.md`: the test suites (`tests/<suite>.rs` with cases in
  `tests/<suite>/`) and what a bug report or pull request needs.
- `docs/DEBUGGING.md`: tracing, hangs, comparing with another simulator.
- `NOTES.md`: release notes; user-visible changes go under `Unreleased`.

## Keeping xezim and xezim-core in sync

The same section is in `../xezim-core/AGENTS.md`; change both together.

- **The pin is the two `rev =` lines** in `Cargo.toml` (`xezim-core` and
  `sv-parser`). Both name the same full 40-character core commit.
  `Cargo.lock` is git-ignored here, so nothing else pins core.
- **xezim CI builds the pinned core, not core `main`.** A core change is
  untested by xezim CI, and invisible to xezim users, until the pin moves to
  it. Core's own CI runs only the core and parser tests.
- **Develop against the local checkout.** Run `scripts/use-local-core.sh`
  once; plain `cargo build`/`cargo test` then uses `../xezim-core` (or
  `./xezim-core`) through a git-ignored `.cargo/config.toml` `[patch]`.
  `cargo tree -p xezim-core` shows a path when the patch is active.
- **Validate the exact commit you will pin.** Commit everything in core first
  (`git -C ../xezim-core status` clean), then run core's tests (`cargo test`
  and `cargo test --manifest-path xezim-parser/Cargo.toml` there) and the
  full suite here (`cargo test --no-fail-fast`) against it. Uncommitted core
  edits make the tested tree differ from the pinned one.
- **Push order:**
  1. Push core to `main`.
  2. Here, add a commit `Sync core revision` that changes only both `rev =`
     lines to the pushed core `HEAD` (`git -C ../xezim-core rev-parse HEAD`).
  3. Check that the pin resolves without the local patch: move
     `.cargo/config.toml` aside, run `cargo fetch`, put it back.
  4. Push xezim, with the commits that need the new core in the same push.
- **Never pin a core commit that is not on core's `origin/main`**
  (`git -C ../xezim-core merge-base --is-ancestor <rev> origin/main`). A
  local-only or force-pushed-away commit breaks every bare clone and CI run.
  Never rewrite pushed core history for the same reason: old xezim pins
  point into it.
- **A core push that xezim does not need yet** (docs, a core-only test)
  still gets a `Sync core revision` in the next xezim push, so the pin does
  not fall behind `main`.
- **Releases:** `xezim`, `xezim-core` and `sv-parser` always carry the same
  `version`. Push the core version bump, then pin xezim to that commit in the
  release push.

A quick check that the pin is current:

```bash
grep -o 'rev = "[0-9a-f]*"' Cargo.toml | sort -u        # exactly one line
git ls-remote https://github.com/aionhw/xezim-core.git refs/heads/main
```
