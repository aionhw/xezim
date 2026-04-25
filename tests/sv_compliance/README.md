# SystemVerilog Compliance Test Suite

This is a **starter compliance suite** organized into **separate files by major language/LRM topic**.
Each test is intended to be small, self-checking, and easy to isolate while bringing up a parser,
elaborator, simulator, or compiler.

## Structure

- `common/svtest_defs.svh` — tiny common macros and helpers
- `tests/` — one test file per language area / LRM-style section
- `manifest.csv` — file-to-topic mapping

## Test philosophy

- Prefer **small, deterministic, self-checking** tests
- Each file should compile and run independently
- A passing test prints `TEST_PASS`
- A failing test prints `TEST_FAIL` and calls `$fatal`

## Included topics

1. lexical elements and identifiers
2. preprocessor macros and conditional compilation
3. literal values
4. scalar/integer data types
5. enums, structs, and unions
6. arrays, queues, dynamic arrays, associative arrays
7. operators and expressions
8. continuous / blocking / nonblocking assignments
9. procedural control flow
10. tasks and functions
11. modules, parameters, ports
12. generate blocks
13. packages and imports
14. interfaces and modports
15. processes and events
16. classes and inheritance
17. constrained randomization
18. assertions
19. covergroups and coverage
20. clocking blocks

## Notes

- This suite intentionally focuses on **core user-visible SystemVerilog language features**.
- DPI, PLI/VPI, checker blocks, program blocks, and advanced SVA sequences are not included here.
- Some tools may support only a subset of the advanced tests (`class`, `randomize`, `covergroup`, `assert property`, `clocking`).

## Suggested bring-up order

Start with:

1. lexical
2. preprocessor
3. literals
4. data types
5. operators
6. assignments
7. control flow
8. modules/parameters
9. tasks/functions
10. arrays

Then enable advanced features:

11. packages
12. generate
13. interfaces
14. processes/events
15. classes
16. randomization
17. assertions
18. covergroups
19. clocking


## Added in the next expansion

### Advanced positive tests

These were added as a second wave of compliance coverage:

21. strings, typedefs, and casts
22. array methods
23. events, mailboxes, and semaphores
24. fork/join variants and wait fork
25. checker blocks
26. specify blocks
27. advanced constraints
28. advanced SVA sequences/properties
29. cross coverage bins
30. let construct
31. user-defined nettypes

Files are in `tests_advanced/`.

### Negative compile-fail tests

Files in `tests_negative/` are intended to be rejected by a compliant compiler or elaborator.
Each file begins with an `EXPECT: compile_fail` marker.

Current negative tests cover:

- duplicate declarations
- undeclared identifiers
- writes to `const`
- non-constant generate conditions
- bad package imports
- illegal modport driving
- illegal clocking block driving
- bad constraint references

## Practical usage notes

- `tests/` and `tests_advanced/` are mainly **compile-and-run** style self-checking tests.
- `tests_negative/` is **compile-fail** oriented.
- Some advanced features (`checker`, `specify`, `nettype`) are not implemented uniformly across all tools.
- This package was organized and reviewed here, but it was **not simulator-validated in-container** because no SystemVerilog simulator is installed.


## xxxyyy / ModelSim
A `Makefile` is included for Mentor xxxyyy / ModelSim style flows.

Examples:
```bash
make basic
make advanced
make positive
make negative
make all
make run-01_lexical_identifiers
make compilefail-neg01_duplicate_declaration
```

Notes:
- Positive tests compile into isolated libraries under `sim/libs/` and run one-by-one.
- Negative tests are expected to fail during `COMPILE` compile.
- Because some tests have free-running clocks and do not call `$finish`, the Makefile runs each simulation for a bounded `RUN_TIME` and then checks the transcript for `TEST_PASS`.
- Override runtime if needed, for example `make positive RUN_TIME=100us`.
