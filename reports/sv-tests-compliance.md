# sv-tests compliance run (runner = xezim)

- **Date:** 2026-07-08
- **Simulator:** xezim 0.8.1 (release build)
- **Suite:** [chipsalliance/sv-tests](https://github.com/chipsalliance/sv-tests) @ `4d3772a6`
- **Command:** `make report RUNNERS=Xezim -j5` (with `XEZIM_BIN` pointing at the
  release binary)
- **Full HTML report:** `svtests_index.html` (+ `svtests_report.csv`)

## Headline

| Category | Pass / Total | Rate |
|---|---|---|
| **All tests** | **4354 / 4768** | **91.3 %** |
| UVM (1800.2-2017) | 484 / 487 | 99.4 % |
| non-`ivtest` | 2153 / 2237 | 96.2 % |
| Icarus `ivtest` suite | 2201 / 2531 | 87.0 % |

## History: the 52 % → 91 % jump

The first run of this suite scored only **52 %**. Investigation showed that was
not a capability gap but a library-scan artifact:

- sv-tests runs each `ivtest` case as a single file but adds its directory,
  `third_party/tools/icarus/ivtest/ivltests/` (~1000 unrelated single-file
  tests), as an `-I` include dir.
- xezim honors IEEE §23.3.2 library-directory semantics — an `-I` dir supplies
  `module`/`interface`/`program` definitions to satisfy unresolved
  instantiations — but `resolve_library_modules` was adopting **every**
  definition found in the directory. Typedefs/enums from unrelated sibling
  files (e.g. `typedef word word_darray[];` in `unp_array_typedef.v`) leaked
  into the primary design's scope and failed a spurious §6.18 "base type not
  declared" check in tests that never mention them. ~2100 `ivtest` cases failed
  this way.

Reproduction (before the fix):

```sh
IV=third_party/tools/icarus/ivtest
xezim --simulate --sv2017 $IV/ivltests/casesynth7.v            # PASSED
xezim --simulate --sv2017 -I $IV/ivltests $IV/ivltests/casesynth7.v
# → Simulation error: typedef 'word_darray': base type 'word' is not declared
#   (word_darray appears nowhere in casesynth7.v — a trivial mux)
```

**Fix** (`xezim-core/src/lib.rs::resolve_library_modules`): index the library
directory's module/interface/program definitions without adopting them, then
pull in only those reachable from the explicitly-compiled design's
instantiations, transitively. Classes, packages and forward typedefs are never
imported from a library dir; a non-forward library typedef is adopted only to
fill a forward typedef the primary design actually declared.

Result: `ivtest` 13.9 % → 87.0 %, overall 52.0 % → 91.3 %, with the native
LRM/UVM tests unchanged (no regressions).

## How it was run

sv-tests already ships a `Xezim.py` runner (registers as `xezim`, resolves the
binary from `$XEZIM_BIN` first). Modes map to `--preprocess` / `--parse` /
`--compile` / `--simulate`; all invocations pass `--sv2017`.

```sh
export XEZIM_BIN=/home/bondan/repo/fix/jul7/xezim/target/release/xezim
cd sv-tests
make report RUNNERS=Xezim -j5   # → out/report/index.html
```
