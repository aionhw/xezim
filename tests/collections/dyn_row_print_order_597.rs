//! IEEE 1800-2017 §21.2.1.7, §10.9.1: `%p` prints a fixed row of a dynamic
//! array or queue from its declared left bound, as it prints a fixed array:
//! `int a[][1:0]` printed `'{'{10, 20}}` where `int f[1:0]` prints
//! `'{20, 10}` (#597, reported by akashlevy; both now print from the left
//! bound). Also: a class member shadows a same-named module array in its
//! methods (§8.10), so `a[0][1] = 20` there wrote the module's array; and
//! `push_back('{1, 2, 3})` fills a `[3:1]` row from its left bound.
//!
use xezim::simulate;

fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

#[test]
fn reported_testbench() {
    let src = r#"// §21.2.1.7: %p prints an unpacked array from its left bound to its right
// bound; a row declared [1:0] prints index 1 first.
class C;
  int a[][1:0];
  function void run();
    a = new[1];
    a[0][1] = 20;
    a[0][0] = 10;
    $display("cls %p expect '{'{20, 10}}", a);
  endfunction
endclass
module t;
  int a[][1:0];
  int f[1:0];
  initial begin
    C c = new;
    c.run();
    a = new[1];
    a[0][1] = 20;
    a[0][0] = 10;
    f[1] = 20;
    f[0] = 10;
    $display("mod %p expect '{'{20, 10}}", a);
    $display("fixed %p expect '{20, 10}", f);
  end
endmodule

"#;
    assert_eq!(
        shown(src, &["cls ", "mod ", "fixed "]),
        [
            "cls '{'{20, 10}} expect '{'{20, 10}}",
            "mod '{'{20, 10}} expect '{'{20, 10}}",
            "fixed '{20, 10} expect '{20, 10}"
        ]
    );
}

#[test]
fn rows_print_from_the_left_bound_and_members_shadow() {
    let src = r#"class C;
  int a[][1:0];
  int n[][1:0][0:1];
  function void run();
    a = new[1]; a[0][1] = 20; a[0][0] = 10;
    n = new[1]; n[0][1][0] = 1; n[0][1][1] = 2; n[0][0][0] = 3; n[0][0][1] = 4;
    $display("R %0d %0d %0d %0d", a[0][1], a[0][0], n[0][1][1], n[0][0][0]);
    $display("P %p %p", a, n);
  endfunction
endclass
module t;
  int a[][1:0];
  int n[][1:0][0:1];
  int z[$][3:1];
  initial begin
    C c = new; c.run();
    a = new[1]; a[0][1] = 7;
    n = new[1]; n[0][1][1] = 5;
    z.push_back('{1, 2, 3});
    $display("M %0d %p %p %p", a[0][1], a, n, z);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["R ", "P ", "M "]),
        [
            "R 20 10 2 3",
            "P '{'{20, 10}} '{'{'{1, 2}, '{3, 4}}}",
            "M 7 '{'{7, 0}} '{'{'{0, 5}, '{0, 0}}} '{'{1, 2, 3}}"
        ]
    );
}
