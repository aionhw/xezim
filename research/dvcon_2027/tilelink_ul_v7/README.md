# DVCon 2027 TileLink-UL showcases

Completed **56 real showcase subjects** using ChatGPT-authenticated local Codex app-server, `gpt-6.1-sol` with `medium` reasoning: **48 runtime-verified fault repairs and eight no-edit clean confirmations**. Each mode has twelve fault cases and two clean controls, one repetition. The user explicitly waived human review for showcase scope. No expert score, independent novelty certification or comparative-effectiveness benefit is claimed.

Start with the [paper-ready report](paper/RESULTS_FOR_DVCON.md), [completed showcase details and costs](paper/SHOWCASES_REVIEW_WAIVED.md), [three earlier pilot narratives](paper/SHOWCASE_CATALOG.md), and [publication scope](PUBLISHING_SCOPE.md). The original 52 calibration subjects (39 repairs, 13 quota failures) and 20 excluded stock-runner development subjects are separate cohorts; their failures remain visible. The unused 168-subject prospective comparison was retired, with zero primary subjects.

All subjects used fresh Python/Codex processes and threads, source-hidden Stage I seals with valid issued evidence IDs before source disclosure, and case-only tools. Every D subject selected an actual timed force/release experiment after submitting its prediction; all physical requests ran two separate negative controls. The three selected D patches passed nine reference-simulator schedules; twelve registered faulty intervention/control traces reproduced across simulators. One repaired monitor value trace matches an independent accepted-pin ledger. Native sampled records and external ledger/value processing are explicitly distinguished.

## Tested source pair

- Xezim baseline: `80f00d354fc68c436569b12b6d4e175ef105a652`, plus the opt-in native observer source published on this branch.
- xezim-core and sv-parser: `837725742ba9aaa3b3408f770b288cf85f020ef9`, unchanged on the paired [`dvcon_2027` branch](https://github.com/aionhw/xezim-core/tree/dvcon_2027).
- Observer source and qualification identities: [implementation pair](IMPLEMENTATION_PAIR.json).
- Published observer precursor: `a2e29b76262ce3862759a93e1fe9a8341275c397`; this port does not change default scheduler behavior. Writer/source/scheduler-wide provenance remains unavailable.

The experiment's exact research binary hash is recorded, but binaries are not redistributed and a source publication is not a claim that this commit was rebuilt into that exact binary. The original resolved dependency lock is [included](repro/locks/xezim-observer.lock). From the repository root, copy it to the ignored root `Cargo.lock`, then use `cargo build --release --locked` with Rust 1.95 or the compatible declared toolchain. xezim-core requires no new functional commit or repin for these showcases.

## Public clean replay

From this package directory, the following uses no model service and creates a fresh output directory:

```sh
cd harness
make test
python3 tools/generate_stimulus.py --seed 123 --count 96 --out results/public_clean_seed_123
python3 tools/run_sim.py \
  --xezim /absolute/path/to/xezim/target/release/xezim \
  --source rtl/tlul_mem_endpoint.sv --stimuli results/public_clean_seed_123 \
  --out results/public_clean_123 --mode none
```

`make test` runs offline oracle/runner tests; mocked transport tests are not model inference. The recorded study had 41 base tests and four additional preexecution showcase-scope checks. The accepted-pin oracle, not exit code alone, determines clean correctness. The included RTL is a synthetic 32-bit single-beat TL-UL profile, not full TileLink verification IP or coherence support.

The [host harness](harness/) is a source snapshot. Its protected primary paths intentionally fail closed without the private catalog, original qualified dependencies and actual independent review. The [showcase overlay](repro/showcase/README.md) preserves its historical import layout as provenance. The private 14-case instance set, seeds, answer mapping and raw model conversations are excluded, so this public package does not by itself rerun the full model campaign. Selected final case-study submissions and predicted/observed contrasts are curated publication excerpts, not full conversations or a complete answer-key catalog.

For any new experimental model subject, use local **Sign in with ChatGPT**, require `account/read=chatgpt` and exact advertised Sol/medium, and requalify filesystem/network/tool boundaries. Do not use API-key authentication, a separately billed API client or a different model. The exact opt-in [Codex 0.166.0 source patch](repro/codex/case_tools_only.patch), upstream commit, normalized lock and qualification receipts are supplied; the 638 MB runtime and build dependencies are not. Preserve nonzero provider-cached input as telemetry; missing values are unknown. Fresh contexts do not eliminate caching or pretraining.

## Reproducibility and scope

[PUBLICATION_MANIFEST.json](PUBLICATION_MANIFEST.json) hashes every exported file except itself. [PUBLICATION_SOURCE_EXPORT.json](PUBLICATION_SOURCE_EXPORT.json) records original-study source hashes and transformations. Original receipt hashes remain provenance references to local artifacts; exported hashes can differ after redaction and editorial clarification. [Aggregate CSV tables](tables/) retain separate mode/cohort denominators and known/unknown cache counts.

The accepted abstract was not rewritten. This is an executable clean harness and results/figure package, not a camera-ready DVCon manuscript. No outside publication, old raw archive or prior experimental answer corpus is bundled. One repetition, synthetic common architecture, source-revealed repair ceilings, unresolved novelty/identifiability and the early native capture window limit generalization. All measurements are scoped to the described study; no simulator speedup, native diagnosis advantage or independent human review is implied.
