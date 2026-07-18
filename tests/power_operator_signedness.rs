//! §11.8.1 / §11.4.3: the `**` operator result is signed iff BOTH operands are
//! signed. xezim previously returned an unsigned value, so a negative power
//! (`(-2)**3`) had correct bits but printed as unsigned (4294967288 vs -8).
//! Also §11.4.3: a negative integer exponent yields 0 for |base|>1.

use xezim::simulate;

fn line(src: &str, tag: &str) -> String {
    let sim = simulate(src, 100).expect("sim");
    sim.output.iter().find(|o| o.message.starts_with(tag))
        .map(|o| o.message.clone()).unwrap_or_default()
}

#[test]
fn negative_power_is_signed() {
    let src = "module t; initial begin\n\
        $display(\"A %0d\", (-2)**3);\n\
        $display(\"B %0d\", (-1)**5);\n\
        $display(\"C %0d\", (-3)**2);\n\
        $display(\"D %0d\", 2**3);\n\
        $finish; end endmodule";
    assert_eq!(line(src, "A "), "A -8", "(-2)**3 must be signed -8");
    assert_eq!(line(src, "B "), "B -1", "(-1)**5 must be signed -1");
    assert_eq!(line(src, "C "), "C 9",  "(-3)**2 = 9");
    assert_eq!(line(src, "D "), "D 8",  "2**3 = 8 (unsigned operands)");
}

#[test]
fn negative_integer_exponent() {
    let src = "module t; initial begin\n\
        $display(\"A %0d\", 2**-1);\n\
        $display(\"B %0d\", 5**-3);\n\
        $display(\"C %0d\", 1**-9);\n\
        $finish; end endmodule";
    assert_eq!(line(src, "A "), "A 0", "|base|>1, neg exp -> 0");
    assert_eq!(line(src, "B "), "B 0", "|base|>1, neg exp -> 0");
    assert_eq!(line(src, "C "), "C 1", "1**anything = 1");
}
