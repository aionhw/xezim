# Prospective cohort preparation and admission

This workflow is prospective infrastructure. It has not generated or admitted any real primary cases. The52 strict calibration subjects remain an earlier uniform code snapshot; these changes never regrade or retry them.

## Design before freeze

The trusted constructor must first author a **new private** taxonomy with twelve distinct fault mechanisms and two clean controls, using at least three component categories. Exposed named and temporal pilot families are excluded. A parameter change or a renamed source does not prove independence. A reviewer must assess semantic overlap against earlier cohorts; no old case material is supplied to debugging subjects. The construction agent must not invent reviewer identity, expertise, independence or approval.

`tools/primary_contract.py` defines schema `TLUL_PRIMARY_V1`. The private taxonomy has:

- `status: FINAL_DESIGN`, a real construction `author_id`, `identifiability_policy`, `transactions_per_schedule` in96..512, and exactly14 `cases`.
- Each case declares `spec_id`, `kind` (`fault` or `clean`), `component` (`endpoint`, `traffic`, `monitor`, `scoreboard`), `patch_target` (`rtl/tlul_mem_endpoint.sv` or `rtl/tlul_components.sv`), `mechanism_private`, `expected_symptom`, `identifiability`, and `novelty_review_questions`.
- `clean_template`, `testbench` and every `extra_sources` item are `{path, sha256}` references within this isolated project. The full clean component template can contain the other protected components. The monitor-capable testbench must emit the public PIN/MON contracts, and the fixed native recipe must resolve its watches.
- A fault has a unique `mechanism_family` and an `edit` with one exact `old` span and distinct `new` span. A clean control has `edit: null`. Optional integer `parameters` map uppercase names to inclusive `{min,max}` bounds; references in text use `{{NAME}}`. No parameterized fault instance is emitted before freeze.

These fields are a design contract, not proof that a fault triggers or is novel. The generator is implemented. Twelve concrete candidate edit designs and two clean controls are drafted under `private/prospective_design_001`, across four component categories. The taxonomy is **not finalized or independently reviewed**; several semantic-overlap and identifiability risks are explicit. Only clean templates have been instantiated and executed. No dummy taxonomy or placeholder hash is accepted as preregistration.

After completing the design, run from the active attempt:

```sh
python3 tools/prepare_preregistration.py \
  --taxonomy private/NEW_DESIGN/taxonomy.json \
  --review-plan 'Describe actual independent admission and blinded adjudication arrangements'
```

This prepares `config/preregistration.json` once. It binds the actual rubric, generator, taxonomy, all source templates/testbenches/dependencies, Sol/medium, allocation168, tool/evidence/deadline limits and implemented bootstrap seed/resamples. It neither freezes nor supplies review approval. Incorrect hashes, escaping/symlinked paths, repeated mechanism families and insufficient component diversity fail validation.

## Freeze, then generate

After protocol, prompts, grader, checker, input templates and independent review arrangements are finalized:

```sh
python3 tools/freeze.py --create
python3 tools/freeze.py --verify
python3 tools/generate_holdouts.py --out private/holdouts_primary_001
```

The freeze hashes qualified binaries, locks and qualification receipts as well as code and private design inputs. Every verification rereads those private inputs. A post-freeze change invalidates the cohort. The local timestamp and readonly files prevent accidental drift; they are not an external timestamp attestation or protection against a malicious host.

The generator obtains a private256-bit seed **after** freeze, shuffles case order, samples declared parameters, assigns opaque IDs and creates original/unseen/adversarial schedules. Seed, clean counterpart, private mechanism and expected repair stay in the trusted private tree. The output catalog is `GENERATED_NOT_ADMITTED`. A persistent generation claim prevents selecting another sample after observing an unsuccessful generation. Partial generation failures are retained; they cannot become primary subjects.

## Automated admission, then independent review

```sh
python3 tools/admit_holdouts.py --catalog private/holdouts_primary_001/catalog.json
```

For every case, this runs clean counterpart and case on three schedules with stock Xezim, research Xezim and the reference simulator:18 actual runs per case,252 scheduled runs overall. It checks actual return/timeout, accepted-pin oracle, public trace equality and concrete functional/monitor symptoms. A timeout, insufficient traffic, missing monitor records or invalid PIN formatting alone is not a functional fault. A fault may fail drain in addition to a concrete symptom; clean controls must drain and pass. No model inference occurs during admission.

An automated PASS is `PASS_AWAITING_REVIEW`, never `APPROVED`. The independent person receives the catalog, frozen design and exact execution evidence. Their supplied admission JSON requires:

- `schema: TLUL_PRIMARY_V1`, `status: APPROVED`, exact `catalog_sha256`.
- `review` with `origin: human`, actual `reviewer_id`, `expertise`, UTC date, and `independent_of_construction: true`; reviewer must differ from the constructor.
- `cases` keyed by all14 opaque IDs, each with `admitted: true`, `novelty_review_passed: true`, a nonempty `identifiability_class` and reason, plus `execution_evidence: {path,sha256}` for that case's actual `case_admission.json`.

Those are reviewer-authored declarations. Host validation binds them to evidence and recomputes saved independent oracles; it does not authenticate identity or turn a model-written statement into human review. Invalid or absent review denies primary inference. A failed frozen all-or-nothing admission is preserved as a failed prospective attempt rather than repaired or resampled in place.

## Primary entrypoints

`agents/run_trial.py --cohort primary` now requires `--primary-catalog`, `--primary-admission`, `--case-id` and `--repeat 1|2|3`. It requires the reviewed source/config/original schedule, exact qualified research simulator,1200-second deadline and native C/D tier. Trial IDs are `CASEID_MODE_REPEAT`. An exclusive per-cell claim prevents replacing a failed subject using a different output path. Claims are outside model-facing workspaces. After prospective freeze or generation, pilot inference is closed; labeling a holdout trial as a pilot cannot bypass admission. Baseline evidence must separately pass the existing source/binary/stimulus/TB/native/value/VCD identity checks.

`tools/grade_patch.py` rechecks original admission and source identity, and uses the **same admitted three schedules** for primary grading rather than generating unrelated fixed-seed schedules. Protected target identity and simulator identity are checked. StageI seal, issued-ID membership, case-only APIs, fresh process/thread and subscription preflight remain required. `tools/run_primary_schedule.py` now supplies the deterministic168-cell plan and gated orchestration. Each case receives all modes and three repeats, with mode order rotated by case/repeat. Capture preparation requires actual reviewed admission; subject and grade identities are rechecked. Only an exact quota-only capability error can continue; mixed quota/forbidden-capability errors halt. Completed and failed subject cells retain exclusive claims and append-only outcomes. A controller interruption never authorizes a replacement inference for an attempted cell. Blinded human grading remains separate; no sessions are implied by this implementation.

Current evidence:38 offline tests pass. The new reference-admission helper also executed an already-exposed clean pilot fixture and matched the existing research-simulator public trace exactly. This probe is not a holdout or model subject. See `results/primary_reference_runner_probe_001/probe.json` and `results/tests_primary_schedule_003.log`.

After actual freeze, generated-case admission and independent approval:

```sh
python3 tools/run_primary_schedule.py --prepare \
  --catalog private/holdouts_primary_001/catalog.json \
  --admission private/holdouts_primary_001/admission.json \
  --out results/primary_001
python3 tools/run_primary_schedule.py --execute --out results/primary_001
```

The preparation phase makes shared native/value/VCD bundles and checks their public traces against admitted research runs. It performs no model inference. Execution records host subject, model/tool-reported, grading and total cell wall separately; token/cache unknowns remain null. If all168 cells finish, state is `FINISHED_AWAITING_BLINDED_REVIEW`, not a primary efficacy conclusion. A hard controller interruption preserves its execution claim for inspection. Never delete a subject claim to retry a failed outcome.

The draft design review request is `private/prospective_design_001/REVIEW_REQUEST.md`, with private file hashes in `results/primary_design_review_packet_001/manifest_private.json`. It explicitly flags adapter/endpoint observational equivalence under the present evidence aperture, synthetic-codec construct validity and semantic-overlap risks. An independent reviewer may request replacement/exclusion or a separately qualified generic observation-profile change before freeze; no strong source-hidden identifiability claim is made from clean-template results. The actual draft status is denied by the preregistration validator.

## Replacement constructor checkpoint

Before prospective freeze, eight draft001 candidates were replaced in versioned private design002. Artificial codec, width/endian and source-index changes were removed. The final constructor taxonomy is `private/prospective_design_002/taxonomy_final.json`:6 endpoint,4 monitor and2 scoreboard fault designs plus2 clean controls. It registers exact per-case admissible causal classes; independent review must accept or reject those classes without widening them. No independent novelty finding is implied. The common `temporal_reuse_v2` traffic recipe is case-independent and covers repeated payloads at different addresses, write/read/write/read lifetimes and noncontiguous masks.

The new clean prototype passes9 simulator runs and1 native/value/VCD capture. Both input targets pass24 control executions across the three simulators. Two exposed anti-silencing canaries still trigger the new clean checking components.41 offline tests pass. These are clean/engineering qualifications, not mutation admission or model outcomes.

The constructor may finalize and freeze its design before post-generation review. The mandatory human review occurs after all252 actual admission executions and BEFORE primary preparation/inference. No reviewer identity or favorable result is invented. `tools/prepare_admission_review.py` creates only a `DRAFT_NOT_APPROVED` form with human fields and decisions null. The exact arrangement and residual blinding limitations are frozen in `INDEPENDENT_REVIEW_PLAN.md`. If any admission fails or review is unavailable, preserve the frozen catalog and report primary NOT_RUN.
