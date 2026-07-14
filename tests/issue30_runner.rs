//! GitHub issue #30 — the reporter's IEEE 1800-2017 §18 constrained-random
//! compliance suite, run verbatim. Each file self-checks and prints TEST_PASS
//! or TEST_FAIL count=N. Randomization is involved, so these also guard
//! against flaky solving, not just outright breakage.

use xezim::simulate;

fn assert_pass(name: &str, src: &str) {
    let sim = simulate(src, 200_000)
        .unwrap_or_else(|e| panic!("{}: simulate failed: {}", name, e));
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("TEST_PASS")),
        "{} did not pass:\n{}",
        name,
        msgs.join("\n")
    );
}

#[test]
fn issue30_00_basic_cons_checks() {
    assert_pass("00_basic_cons_checks", include_str!("issue30/00_basic_cons_checks.sv"));
}

#[test]
fn issue30_01_basic_cons_checks() {
    assert_pass("01_basic_cons_checks", include_str!("issue30/01_basic_cons_checks.sv"));
}

#[test]
fn issue30_02_basic_cons_checks() {
    assert_pass("02_basic_cons_checks", include_str!("issue30/02_basic_cons_checks.sv"));
}

#[test]
fn issue30_03_basic_array_rand() {
    assert_pass("03_basic_array_rand", include_str!("issue30/03_basic_array_rand.sv"));
}

#[test]
fn issue30_04_sys_user_funcs_in_cons() {
    assert_pass("04_sys_user_funcs_in_cons", include_str!("issue30/04_sys_user_funcs_in_cons.sv"));
}

#[test]
fn issue30_05_complex_class_struct_union_enum() {
    assert_pass("05_complex_class_struct_union_enum", include_str!("issue30/05_complex_class_struct_union_enum.sv"));
}

#[test]
fn issue30_06_advanced_class_inherit_misc_rand() {
    assert_pass("06_advanced_class_inherit_misc_rand", include_str!("issue30/06_advanced_class_inherit_misc_rand.sv"));
}

#[test]
fn issue30_07_advanced_qaarr_uniq_var_order() {
    assert_pass("07_advanced_qaarr_uniq_var_order", include_str!("issue30/07_advanced_qaarr_uniq_var_order.sv"));
}

#[test]
fn issue30_08_cons_guard_soft_cons() {
    assert_pass("08_cons_guard_soft_cons", include_str!("issue30/08_cons_guard_soft_cons.sv"));
}

#[test]
fn issue30_09_loc_scope_dyn_wt_inline_cons_chk() {
    assert_pass("09_loc_scope_dyn_wt_inline_cons_chk", include_str!("issue30/09_loc_scope_dyn_wt_inline_cons_chk.sv"));
}

#[test]
fn issue30_10_std_randomize_checks() {
    assert_pass("10_std_randomize_checks", include_str!("issue30/10_std_randomize_checks.sv"));
}

#[test]
fn issue30_11_rand_func_stability_srandom() {
    assert_pass("11_rand_func_stability_srandom", include_str!("issue30/11_rand_func_stability_srandom.sv"));
}

#[test]
fn issue30_12_rand_func_stability_randstate() {
    assert_pass("12_rand_func_stability_randstate", include_str!("issue30/12_rand_func_stability_randstate.sv"));
}
