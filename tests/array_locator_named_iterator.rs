//! §7.12.1 array locator methods with a NAMED iterator argument:
//! `a.find(x) with (x > 3)`. The custom iterator name (`x`) was dropped, so the
//! filter's `x` was unbound and the method wrongly returned an empty queue. The
//! default-iterator form `a.find with (item > 3)` always worked.

use xezim::simulate;

fn out(src: &str, tag: &str) -> String {
    let sim = simulate(src, 100).expect("sim");
    sim.output.iter().find(|o| o.message.starts_with(tag))
        .map(|o| o.message.clone()).unwrap_or_default()
}

#[test]
fn find_with_named_iterator() {
    let src = "module t; int a[] = '{5,3,8,1,3}; int r[$];\n\
      initial begin\n\
        r = a.find(x) with (x > 3);        $display(\"A %p\", r);\n\
        r = a.find_index(y) with (y == 3);  $display(\"B %p\", r);\n\
        r = a.find_first(z) with (z < 4);   $display(\"C %p\", r);\n\
        r = a.find with (item > 3);         $display(\"D %p\", r);\n\
      $finish; end endmodule";
    assert_eq!(out(src, "A "), "A '{5, 8}", "named iterator find");
    assert_eq!(out(src, "B "), "B '{1, 4}", "named iterator find_index");
    assert_eq!(out(src, "C "), "C '{3}",    "named iterator find_first");
    assert_eq!(out(src, "D "), "D '{5, 8}", "default iterator still works");
}
