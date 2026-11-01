//! §7.12.1: locator methods on an associative array search in key order and
//! the `_index` forms return keys of the index type. Only arrays whose keys
//! were all non-negative integers were searched; string and signed keys
//! returned an empty queue.
use xezim::simulate;

/// The `$display` lines of `src` that start with one of `prefixes`.
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
fn string_keyed_locators() {
    let src = r#"class C; int m[string]; endclass
module tb;
  int a[string];
  string sk[$];
  int v[$];
  C c;
  initial begin
    a["x"] = 7; a["y"] = 1; a["z]"] = 9; a["123"] = 8;
    sk = a.find_index with (item > 5);           $display("A %p", sk);
    v = a.find with (item > 5);                  $display("A %p", v);
    sk = a.find_first_index with (item > 5);     $display("A %p", sk);
    sk = a.find_last_index with (item > 5);      $display("A %p", sk);
    sk = a.find_index with (item.index == "z]"); $display("A %p", sk);
    sk = a.unique_index;                         $display("A %p", sk);
    v = a.min;                                   $display("A %p", v);
    $display("A %p %s", a.find_index with (item > 8), a.find_first_index with (item == 1)[0]);
    c = new; c.m["k[0]"] = 5; c.m["j"] = 1;
    sk = c.m.find_index with (item > 2);         $display("C %p", sk);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["A", "C"]),
        [
            r#"A '{"123", "x", "z]"}"#,
            "A '{8, 7, 9}",
            r#"A '{"123"}"#,
            r#"A '{"z]"}"#,
            r#"A '{"z]"}"#,
            r#"A '{"123", "x", "y", "z]"}"#,
            "A '{1}",
            r#"A '{"z]"} y"#,
            r#"C '{"k[0]"}"#,
        ]
    );
}

#[test]
fn signed_keyed_locators() {
    let src = r#"module tb;
  int n[int];
  byte b[shortint];
  int ik[$];
  int v[$];
  initial begin
    n[-5] = 7; n[3] = 1; n[10] = 9;
    b[-2] = 3; b[4] = 8;
    ik = n.find_index with (item > 5);        $display("N %p", ik);
    v = n.find with (item > 5);               $display("N %p", v);
    ik = n.find_index with (item.index < 0);  $display("N %p", ik);
    $display("N %0d", (n.find_first_index with (item == 7))[0]);
    ik = b.find_index with (item > 0);        $display("B %p", ik);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["N", "B"]),
        ["N '{-5, 10}", "N '{7, 9}", "N '{-5}", "N -5", "B '{-2, 4}",]
    );
}

/// Full-range unsigned keys, negative keys with the element forms, and
/// `%p` of a locator call on a class member.
#[test]
fn wide_keys_element_forms_and_direct_print() {
    let src = r#"class C; string sm[string]; endclass
module tb;
  int w[bit [63:0]];
  int n[int];
  bit [63:0] wk[$];
  int v[$];
  C c;
  initial begin
    w[64'hFFFF_FFFF_FFFF_FFFF] = 1; w[5] = 2;
    wk = w.find_index with (item.index > 64'h7FFF_FFFF_FFFF_FFFF);
    $display("W %h %0d", wk[0], wk.size());
    n[-5] = 7; n[3] = 1; n[10] = 9; n[-1] = 7;
    v = n.find_first with (item == 7);        $display("N %p", v);
    v = n.find_last_index with (item == 7);   $display("N %p", v);
    v = n.max;                                $display("N %p", v);
    v = n.find(x) with (x.index < 0);         $display("N %p", v);
    c = new; c.sm["a"] = "w"; c.sm["b"] = "v";
    $display("C %p %p", c.sm.find with (item == "w"), c.sm.find_index with (item == "v"));
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["W", "N", "C"]),
        [
            "W ffffffffffffffff 1",
            "N '{7}",
            "N '{-1}",
            "N '{9}",
            "N '{7, 7}",
            r#"C '{"w"} '{"b"}"#,
        ]
    );
}
