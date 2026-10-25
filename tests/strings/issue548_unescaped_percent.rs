//! Issue #548: Unknown / unescaped format specifier in $display, $write, and $sformatf.
//!
//! When an unescaped '%' is followed by a character not in IEEE 1800-2023 Table 21-2
//! (or trailing at the end of the format string) and no unconsumed arguments remain,
//! xezim previously dropped both the '%' and the trailing character silently.
//! Both '%' and the trailing character should be preserved literally without
//! desynchronizing positional arguments.

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

const SRC: &str = r#"
module tb;
  string s1, s2, s3;
  initial begin
    // 1. Simple unescaped '%'
    $display("D1=[Progress: 100% Complete]");

    // 2. Standard IEEE 1800 escaped '%%'
    $display("D2=[Progress: 100%% Complete]");

    // 3. Trailing percent at EOF
    $display("D3=[Done %]");

    // 4. Interleaved positional arguments
    $display("D4=[Count: %0d, Progress: 100%, Status: %s]", 42, "VALID");

    // 5. Flagged and width-padded invalid specifier
    $display("D5=[Prefix %-04.2% Suffix]");

    // 6. Verification across $write
    $write("W1=[write 100% Done]\n");
    $write("W2=[Count: %0d, Progress: 100%, Status: %s]\n", 99, "PASS");

    // 7. Verification across $sformatf
    s1 = $sformatf("sfmt 100% Done");
    s2 = $sformatf("Count: %0d, Progress: 100%, Status: %s", 7, "ACTIVE");
    s3 = $sformatf("Done %");
    $display("S1=[%s]", s1);
    $display("S2=[%s]", s2);
    $display("S3=[%s]", s3);

    $finish;
  end
endmodule
"#;

#[test]
fn issue548_unescaped_percent_preserved_in_display_write_sformatf() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(line(&sim, "D1="), "D1=[Progress: 100% Complete]");
    assert_eq!(line(&sim, "D2="), "D2=[Progress: 100% Complete]");
    assert_eq!(line(&sim, "D3="), "D3=[Done %]");
    assert_eq!(
        line(&sim, "D4="),
        "D4=[Count: 42, Progress: 100%, Status: VALID]"
    );
    assert_eq!(line(&sim, "D5="), "D5=[Prefix %-04.2% Suffix]");
    assert_eq!(line(&sim, "W1="), "W1=[write 100% Done]");
    assert_eq!(
        line(&sim, "W2="),
        "W2=[Count: 99, Progress: 100%, Status: PASS]"
    );
    assert_eq!(line(&sim, "S1="), "S1=[sfmt 100% Done]");
    assert_eq!(
        line(&sim, "S2="),
        "S2=[Count: 7, Progress: 100%, Status: ACTIVE]"
    );
    assert_eq!(line(&sim, "S3="), "S3=[Done %]");
}
