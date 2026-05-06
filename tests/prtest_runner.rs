//! Automated test runner for iverilog PR regression tests.
//! Tests are from the prtest collection of iverilog bug-fix verification files.
//!
//! Each test parses and simulates a .v file, checking for "PASSED" in output.

use std::path::Path;
use std::process::Command;

fn run_prtest(filename: &str) {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/prtest");
    let test_file = test_dir.join(filename);
    assert!(
        test_file.exists(),
        "Test file not found: {}",
        test_file.display()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Check for parse errors
    assert!(
        !stdout.contains("Parse errors"),
        "Parse error in {}: {}",
        filename,
        stdout
    );

    // Check for simulation errors
    assert!(
        !stdout.contains("Simulation error"),
        "Simulation error in {}: {}",
        filename,
        stdout
    );

    // Check for PASSED (if test has self-check)
    if stdout.contains("PASSED") || stdout.contains("FAILED") {
        assert!(
            stdout.contains("PASSED") && !stdout.contains("FAILED"),
            "Test {} did not pass. Output:\n{}{}",
            filename,
            stdout,
            stderr
        );
    }
}

#[test]
fn test_pr_pr1000() {
    run_prtest("pr1000.v");
}

#[test]
fn test_pr_pr1002() {
    run_prtest("pr1002.v");
}

#[test]
fn test_pr_pr1002a() {
    run_prtest("pr1002a.v");
}

#[test]
fn test_pr_pr1024() {
    run_prtest("pr1024.v");
}

#[test]
fn test_pr_pr1087() {
    run_prtest("pr1087.v");
}

#[test]
fn test_pr_pr1101() {
    run_prtest("pr1101.v");
}

#[test]
fn test_pr_pr136() {
    run_prtest("pr136.v");
}

#[test]
fn test_pr_pr1367855() {
    run_prtest("pr1367855.v");
}

#[test]
fn test_pr_pr1449749a() {
    run_prtest("pr1449749a.v");
}

#[test]
fn test_pr_pr1528093() {
    run_prtest("pr1528093.v");
}

#[test]
fn test_pr_pr1561597() {
    run_prtest("pr1561597.v");
}

#[test]
fn test_pr_pr1570635() {
    run_prtest("pr1570635.v");
}

#[test]
fn test_pr_pr1570635b() {
    run_prtest("pr1570635b.v");
}

#[test]
fn test_pr_pr1581580() {
    run_prtest("pr1581580.v");
}

#[test]
fn test_pr_pr1601896() {
    run_prtest("pr1601896.v");
}

#[test]
fn test_pr_pr1601898() {
    run_prtest("pr1601898.v");
}

#[test]
fn test_pr_pr1609611() {
    run_prtest("pr1609611.v");
}

#[test]
fn test_pr_pr1612693() {
    run_prtest("pr1612693.v");
}

#[test]
fn test_pr_pr1625912() {
    run_prtest("pr1625912.v");
}

#[test]
fn test_pr_pr1662508() {
    run_prtest("pr1662508.v");
}

#[test]
fn test_pr_pr1676836() {
    run_prtest("pr1676836.v");
}

#[test]
fn test_pr_pr1682887() {
    run_prtest("pr1682887.v");
}

#[test]
fn test_pr_pr1693890() {
    run_prtest("pr1693890.v");
}

#[test]
fn test_pr_pr1694427() {
    run_prtest("pr1694427.v");
}

#[test]
fn test_pr_pr1695334() {
    run_prtest("pr1695334.v");
}

#[test]
fn test_pr_pr1697250() {
    run_prtest("pr1697250.v");
}

#[test]
fn test_pr_pr1777103() {
    run_prtest("pr1777103.v");
}

#[test]
fn test_pr_pr1784984() {
    run_prtest("pr1784984.v");
}

#[test]
fn test_pr_pr1822658() {
    run_prtest("pr1822658.v");
}

#[test]
fn test_pr_pr1875866() {
    run_prtest("pr1875866.v");
}

#[test]
fn test_pr_pr1875866b() {
    run_prtest("pr1875866b.v");
}

#[test]
fn test_pr_pr1877740() {
    run_prtest("pr1877740.v");
}

#[test]
fn test_pr_pr1879226() {
    run_prtest("pr1879226.v");
}

#[test]
fn test_pr_pr1909940() {
    run_prtest("pr1909940.v");
}

#[test]
fn test_pr_pr1909940b() {
    run_prtest("pr1909940b.v");
}

#[test]
fn test_pr_pr1912843() {
    run_prtest("pr1912843.v");
}

#[test]
fn test_pr_pr1916261() {
    run_prtest("pr1916261.v");
}

#[test]
fn test_pr_pr1916261a() {
    run_prtest("pr1916261a.v");
}

#[test]
fn test_pr_pr1924845() {
    run_prtest("pr1924845.v");
}

#[test]
fn test_pr_pr1925356() {
    run_prtest("pr1925356.v");
}

#[test]
fn test_pr_pr1939165() {
    run_prtest("pr1939165.v");
}

#[test]
fn test_pr_pr1946411() {
    run_prtest("pr1946411.v");
}

#[test]
fn test_pr_pr1948110() {
    run_prtest("pr1948110.v");
}

#[test]
fn test_pr_pr1948342() {
    run_prtest("pr1948342.v");
}

#[test]
fn test_pr_pr1990029() {
    run_prtest("pr1990029.v");
}

#[test]
fn test_pr_pr1990164() {
    run_prtest("pr1990164.v");
}

#[test]
fn test_pr_pr1990269() {
    run_prtest("pr1990269.v");
}

#[test]
fn test_pr_pr2011429() {
    run_prtest("pr2011429.v");
}

#[test]
fn test_pr_pr2013758() {
    run_prtest("pr2013758.v");
}

#[test]
fn test_pr_pr2014673() {
    run_prtest("pr2014673.v");
}

#[test]
fn test_pr_pr2015466() {
    run_prtest("pr2015466.v");
}

#[test]
fn test_pr_pr2018235b() {
    run_prtest("pr2018235b.v");
}

#[test]
fn test_pr_pr2018305() {
    run_prtest("pr2018305.v");
}

#[test]
fn test_pr_pr2030767() {
    run_prtest("pr2030767.v");
}

#[test]
fn test_pr_pr2076425() {
    run_prtest("pr2076425.v");
}

#[test]
fn test_pr_pr2085984() {
    run_prtest("pr2085984.v");
}

#[test]
fn test_pr_pr2117473() {
    run_prtest("pr2117473.v");
}

#[test]
fn test_pr_pr2117488() {
    run_prtest("pr2117488.v");
}

#[test]
fn test_pr_pr2123190() {
    run_prtest("pr2123190.v");
}

#[test]
fn test_pr_pr2146620c() {
    run_prtest("pr2146620c.v");
}

#[test]
fn test_pr_pr2166188() {
    run_prtest("pr2166188.v");
}

#[test]
fn test_pr_pr2181249() {
    run_prtest("pr2181249.v");
}

#[test]
fn test_pr_pr2202846a() {
    run_prtest("pr2202846a.v");
}

#[test]
fn test_pr_pr2208681() {
    run_prtest("pr2208681.v");
}

#[test]
fn test_pr_pr2215342() {
    run_prtest("pr2215342.v");
}

#[test]
fn test_pr_pr2224949() {
    run_prtest("pr2224949.v");
}

#[test]
fn test_pr_pr2270035() {
    run_prtest("pr2270035.v");
}

#[test]
fn test_pr_pr2355304b() {
    run_prtest("pr2355304b.v");
}

#[test]
fn test_pr_pr2358264() {
    run_prtest("pr2358264.v");
}

#[test]
fn test_pr_pr2425055a() {
    run_prtest("pr2425055a.v");
}

#[test]
fn test_pr_pr2428890() {
    run_prtest("pr2428890.v");
}

#[test]
fn test_pr_pr2434688() {
    run_prtest("pr2434688.v");
}

#[test]
fn test_pr_pr2434688b() {
    run_prtest("pr2434688b.v");
}

#[test]
fn test_pr_pr2450244() {
    run_prtest("pr2450244.v");
}

#[test]
fn test_pr_pr2453002() {
    run_prtest("pr2453002.v");
}

#[test]
fn test_pr_pr2456943() {
    run_prtest("pr2456943.v");
}

#[test]
fn test_pr_pr2459681() {
    run_prtest("pr2459681.v");
}

#[test]
fn test_pr_pr2476430() {
    run_prtest("pr2476430.v");
}

#[test]
fn test_pr_pr2503208() {
    run_prtest("pr2503208.v");
}

#[test]
fn test_pr_pr2593733() {
    run_prtest("pr2593733.v");
}

#[test]
fn test_pr_pr2688910() {
    run_prtest("pr2688910.v");
}

#[test]
fn test_pr_pr2715748() {
    run_prtest("pr2715748.v");
}

#[test]
fn test_pr_pr2722339a() {
    run_prtest("pr2722339a.v");
}

#[test]
fn test_pr_pr2722339b() {
    run_prtest("pr2722339b.v");
}

#[test]
fn test_pr_pr2723712() {
    run_prtest("pr2723712.v");
}

#[test]
fn test_pr_pr2806449() {
    run_prtest("pr2806449.v");
}

#[test]
fn test_pr_pr2818823() {
    run_prtest("pr2818823.v");
}

#[test]
fn test_pr_pr2865563() {
    run_prtest("pr2865563.v");
}

#[test]
fn test_pr_pr2877555() {
    run_prtest("pr2877555.v");
}

#[test]
fn test_pr_pr2922063() {
    run_prtest("pr2922063.v");
}

#[test]
fn test_pr_pr2929913() {
    run_prtest("pr2929913.v");
}

#[test]
fn test_pr_pr2943394() {
    run_prtest("pr2943394.v");
}

#[test]
fn test_pr_pr2969724() {
    run_prtest("pr2969724.v");
}

#[test]
fn test_pr_pr2971207() {
    run_prtest("pr2971207.v");
}

#[test]
fn test_pr_pr2974216() {
    run_prtest("pr2974216.v");
}

#[test]
fn test_pr_pr2974216b() {
    run_prtest("pr2974216b.v");
}

#[test]
fn test_pr_pr2976242b() {
    run_prtest("pr2976242b.v");
}

#[test]
fn test_pr_pr2991457b() {
    run_prtest("pr2991457b.v");
}

#[test]
fn test_pr_pr2994193() {
    run_prtest("pr2994193.v");
}

#[test]
fn test_pr_pr2998515() {
    run_prtest("pr2998515.v");
}

#[test]
fn test_pr_pr3022502() {
    run_prtest("pr3022502.v");
}

#[test]
fn test_pr_pr304() {
    run_prtest("pr304.v");
}

#[test]
fn test_pr_pr307() {
    run_prtest("pr307.v");
}

#[test]
fn test_pr_pr3077640() {
    run_prtest("pr3077640.v");
}

#[test]
fn test_pr_pr307a() {
    run_prtest("pr307a.v");
}

#[test]
fn test_pr_pr312() {
    run_prtest("pr312.v");
}

#[test]
fn test_pr_pr3366114() {
    run_prtest("pr3366114.v");
}

#[test]
fn test_pr_pr3527022() {
    run_prtest("pr3527022.v");
}

#[test]
fn test_pr_pr3539372() {
    run_prtest("pr3539372.v");
}

#[test]
fn test_pr_pr355() {
    run_prtest("pr355.v");
}

#[test]
fn test_pr_pr3561350() {
    run_prtest("pr3561350.v");
}

#[test]
fn test_pr_pr3563412() {
    run_prtest("pr3563412.v");
}

#[test]
fn test_pr_pr445() {
    run_prtest("pr445.v");
}

#[test]
fn test_pr_pr509() {
    run_prtest("pr509.v");
}

#[test]
fn test_pr_pr513() {
    run_prtest("pr513.v");
}

#[test]
fn test_pr_pr538() {
    run_prtest("pr538.v");
}

#[test]
fn test_pr_pr585() {
    run_prtest("pr585.v");
}

#[test]
fn test_pr_pr602() {
    run_prtest("pr602.v");
}

#[test]
fn test_pr_pr617() {
    run_prtest("pr617.v");
}

#[test]
fn test_pr_pr710() {
    run_prtest("pr710.v");
}

#[test]
fn test_pr_pr721() {
    run_prtest("pr721.v");
}

#[test]
fn test_pr_pr722() {
    run_prtest("pr722.v");
}

#[test]
fn test_pr_pr757() {
    run_prtest("pr757.v");
}

#[test]
fn test_pr_pr810() {
    run_prtest("pr810.v");
}

#[test]
fn test_pr_pr823() {
    run_prtest("pr823.v");
}

#[test]
fn test_pr_pr841() {
    run_prtest("pr841.v");
}

#[test]
fn test_pr_pr859() {
    run_prtest("pr859.v");
}

#[test]
fn test_pr_pr860() {
    run_prtest("pr860.v");
}

#[test]
fn test_pr_pr913() {
    run_prtest("pr913.v");
}

#[test]
fn test_pr_pr973() {
    run_prtest("pr973.v");
}

#[test]
fn test_pr_pr990() {
    run_prtest("pr990.v");
}

#[test]
fn test_pr_prng() {
    run_prtest("prng.v");
}
