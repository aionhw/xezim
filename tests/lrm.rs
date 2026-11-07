//! Integration-test group: IEEE 1800-2023 conformance.
//!
//! One module per LRM clause. Each test runs one audit probe or finding
//! (`tests/lrm/sv/`) and compares the `T|` lines it prints with the lines
//! the reference simulator printed for the same file, embedded as the
//! expected value. Where xezim still differs, the differing lines are left
//! out of the comparison and listed in a `Known gap` comment at the test;
//! lines whose value is implementation-defined, or where the reference
//! departs from the LRM, are marked `Not compared`.
//!
//! To add a case: put the fixture in `tests/lrm/sv/`, run it on the
//! reference simulator, and add a test that embeds its `T|` lines.

#[path = "lrm/annex_d_e.rs"]
mod annex_d_e;
#[path = "lrm/ch03_building_blocks.rs"]
mod ch03_building_blocks;
#[path = "lrm/ch04_scheduling.rs"]
mod ch04_scheduling;
#[path = "lrm/ch05_lexical.rs"]
mod ch05_lexical;
#[path = "lrm/ch06_data_types.rs"]
mod ch06_data_types;
#[path = "lrm/ch07_aggregates.rs"]
mod ch07_aggregates;
#[path = "lrm/ch08_classes.rs"]
mod ch08_classes;
#[path = "lrm/ch09_processes.rs"]
mod ch09_processes;
#[path = "lrm/ch10_assignments.rs"]
mod ch10_assignments;
#[path = "lrm/ch11_operators.rs"]
mod ch11_operators;
#[path = "lrm/ch12_procedural.rs"]
mod ch12_procedural;
#[path = "lrm/ch13_subroutines.rs"]
mod ch13_subroutines;
#[path = "lrm/ch14_clocking.rs"]
mod ch14_clocking;
#[path = "lrm/ch15_interprocess_sync.rs"]
mod ch15_interprocess_sync;
#[path = "lrm/ch16_assertions.rs"]
mod ch16_assertions;
#[path = "lrm/ch17_checkers.rs"]
mod ch17_checkers;
#[path = "lrm/ch18_constrained_random.rs"]
mod ch18_constrained_random;
#[path = "lrm/ch19_coverage.rs"]
mod ch19_coverage;
#[path = "lrm/ch20_system_tasks.rs"]
mod ch20_system_tasks;
#[path = "lrm/ch21_input_output.rs"]
mod ch21_input_output;
#[path = "lrm/ch22_directives.rs"]
mod ch22_directives;
#[path = "lrm/ch23_hierarchy.rs"]
mod ch23_hierarchy;
#[path = "lrm/ch24_programs.rs"]
mod ch24_programs;
#[path = "lrm/ch25_interfaces.rs"]
mod ch25_interfaces;
#[path = "lrm/ch26_packages.rs"]
mod ch26_packages;
#[path = "lrm/ch27_generate.rs"]
mod ch27_generate;
#[path = "lrm/ch28_gates.rs"]
mod ch28_gates;
#[path = "lrm/ch29_udps.rs"]
mod ch29_udps;
#[path = "lrm/ch30_specify.rs"]
mod ch30_specify;
#[path = "lrm/ch31_timing_checks.rs"]
mod ch31_timing_checks;
#[path = "lrm/ch32_sdf.rs"]
mod ch32_sdf;
#[path = "lrm/ch34_protected_envelopes.rs"]
mod ch34_protected_envelopes;
#[path = "lrm/ch35_dpi.rs"]
mod ch35_dpi;
#[path = "lrm/ch36_vpi.rs"]
mod ch36_vpi;
#[path = "lrm/constraint_real_bounds.rs"]
mod constraint_real_bounds;
#[path = "lrm/harness.rs"]
mod harness;
#[path = "lrm/implicit_static_subroutines_keep_their_variables_when_compiled.rs"]
mod implicit_static_subroutines_keep_their_variables_when_compiled;
