# Real Accellera `uvm-tests` vs xezim — independent measurement

Date: 2026-07-14
Harness: `reports/uvmtests_harness.py` · Raw results: `reports/uvmtests_results.json`
UVM library: `1800.2-2020.3.1` (paired with `uvm-tests`, per the 2020 series)

## TL;DR

The README/compliance headline **"UVM (1800.2-2017) 484 / 487 = 99.4 %"** is **not**
measured against the real Accellera `uvm-tests`. It is the score on the
**`sv-tests` `uvm-req` meta-tag** — 487 *generic SystemVerilog feature tests*
(sections 6.18, 8.3, …) labelled "UVM-relevant" because UVM happens to use those
LRM features. The label "UVM (1800.2-2017)" is misleading.

Running the **actual** Accellera `uvm-tests` (622 simulation tests) against the
current xezim 0.9.2 binary:

| Configuration | Pass | Rate |
|---|---|---|
| Current default (`PURE_SV_LRM` on, 0.9.2) | 39 / 622 | **6.3 %** |
| `PURE_SV_LRM=0` (pre-regression default) | 149 / 622 | **24.0 %** |

Either way, **99.4 % is not retrievable.**

## Finding 1 — a run-phase regression in 0.9.2

Commit `7fc8187` (2026-07-13, "PURE_SV_LRM default on") flipped
`pure_sv_lrm` from **off** to **on by default**. With the new default the
spawned `run_phase` task bodies never execute, so simulation finishes at
**time 0** with no stimulus, no objections drained, and no `report_phase` —
hence no `** UVM TEST PASSED **`.

The flagship `GettingVerilatorStartedWithUVM` example (README claim
"COLLECTED PACKETS = 77, 4/4 pass") **does not reproduce** on 0.9.2. It only
works with `PURE_SV_LRM=0`, where it again reaches time 1575 with
`COLLECTED PACKETS = 77` — exactly matching the documented result. So the
0.8.1-era UVM numbers require reverting that default (`PURE_SV_LRM=0`).

## Finding 2 — the real `uvm-tests` pass rate (PURE_SV_LRM=0)

Even with the regression reverted, the real suite scores **24.0 % (149/622)**.
NOVERDICT root causes (354 unfinished tests):

| Count | Root cause |
|---|---|
| 144 | explicit `UVM_FATAL`/`UVM_ERROR` (real xezim capability gaps) |
| 90  | phases never run — sim finishes at **time 0** (no objection/clock → phase never drains) |
| 48  | multi-file tests needing a filelist (harness gap) |
| 28  | parse errors |
| 11  | sim errors / misc |

The 90-test "time 0" bucket is a **pre-existing** limitation, independent of
the regression: tests whose `run_phase` raises no objection and has no clock
never drain, so `report_phase` never fires. (Even `00basic/00hello`, the
simplest test in the suite, is affected — adding an objection + clock still
finishes at time 0 because the spawned task body doesn't run.)

## Finding 3 — what the 99.4 % actually is

`reports/sv-tests-compliance.md` → `svtests_report.csv`: 487 rows carry the
`uvm`/`uvm-req` tag. `sv-tests/conf/meta-tags.conf` defines `uvm-req` as a
flat list of LRM sections (5.4, 6.18, 8.3, …). The 487 tests are SV-language
feature tests (typedefs, `ifdef`, packed structs, …) tagged because UVM uses
those features — **not** UVM methodology tests. That score is legitimate as a
*SystemVerilog-language* compliance figure, but it is not "UVM 99.4 %".

## Reproduce

```sh
XEZIM=/path/to/xezim  UVM=/path/to/1800.2-2020.3.1  python3 reports/uvmtests_harness.py
```
