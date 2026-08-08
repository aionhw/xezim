use std::fs;
use xezim::*;

// Real uvm-1.2 package: elaborates cleanly and the run_test/coreservice/
// factory bootstrap executes, but uvm_root construction (the phase
// scheduler) currently does not terminate. Ignored until the phasing
// engine runs to completion — running it would hang the test suite.
#[test]
#[ignore = "real uvm-1.2: uvm_root phase-scheduler does not yet terminate"]
fn test_uvm_complete() {
    let uvm_pkg = fs::read_to_string("uvm-1.2/src/uvm_pkg.sv").expect("Could not read uvm_pkg.sv");
    let test_src = fs::read_to_string("tests/uvm/uvm_complete_test.sv")
        .expect("Could not read uvm_complete_test.sv");

    let include_dirs = vec!["uvm-1.2/src".to_string()];

    // UVM needs UVM_NO_DPI if we don't have the DPI library
    let defines = vec![("UVM_NO_DPI".to_string(), None)];

    let res = simulate_multi(
        &[uvm_pkg, test_src],
        2000,
        SimOptions {
            top_module_name: Some("top".to_string()),
            include_dirs: include_dirs.to_vec(),
            defines: defines.to_vec(),
            ..Default::default()
        },
    );

    assert!(res.is_ok(), "UVM Complete test failed: {:?}", res.err());
}

#[test]
#[ignore = "real uvm-1.2: uvm_root phase-scheduler does not yet terminate"]
fn test_uvm_hello_world() {
    let uvm_pkg = fs::read_to_string("uvm-1.2/src/uvm_pkg.sv").expect("Could not read uvm_pkg.sv");
    let test_src = fs::read_to_string("uvm-1.2/examples/simple/hello_world/hello_world.sv")
        .expect("Could not read hello_world.sv");

    let include_dirs = vec![
        "uvm-1.2/src".to_string(),
        "uvm-1.2/examples/simple/hello_world".to_string(),
    ];

    let defines = vec![("UVM_NO_DPI".to_string(), None)];

    let res = simulate_multi(
        &[uvm_pkg, test_src],
        10000,
        SimOptions {
            top_module_name: Some("hello_world".to_string()),
            include_dirs: include_dirs.to_vec(),
            defines: defines.to_vec(),
            ..Default::default()
        },
    );

    assert!(res.is_ok(), "UVM Hello World test failed: {:?}", res.err());
}
