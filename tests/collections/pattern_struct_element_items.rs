//! IEEE 1800-2023 §10.9.1, §10.9.2, §7.2, §10.4.1: an assignment pattern
//! item that is a whole unpacked structure (an element of an array of them,
//! a structure variable) is matched to a structure element and supplies its
//! members. A pattern that reads its own target (`sa = '{sa[1], sa[0]}`)
//! reads every item before any element is written; that capture took each
//! structure as one value and filled every member with it, storing zeros.
//! The same items with an intra-assignment delay are checked too (§9.4.5).

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn whole_structure_items_self_read_and_delayed() {
    let out = lines(
        r#"
module top;
  typedef struct { int a; int b; } S;
  typedef struct { int x; int y[2]; } T;
  S sa[2] = '{'{1, 2}, '{3, 4}};
  S sb[2] = '{'{5, 6}, '{7, 8}};
  S s1 = '{9, 10};
  S tb[2], tc[2], td[2];
  T ta[2] = '{'{1, '{2, 3}}, '{4, '{5, 6}}};
  S m2[2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}};
  class K;
    S ks[2] = '{'{1, 2}, '{3, 4}};
    function void swap(); ks = '{ks[1], ks[0]}; endfunction
  endclass
  K k;
  initial begin
    sa = '{sa[1], sa[0]};
    $display("T|sa=%p", sa);
    ta = '{ta[1], ta[0]};
    $display("T|ta=%p", ta);
    m2[0] = '{m2[0][1], m2[0][0]};
    $display("T|m2=%p", m2);
    tb = '{s1, sb[0]};
    $display("T|tb=%p", tb);
    tc = #1 '{sb[1], s1};
    $display("T|tc=%p", tc);
    td <= #1 '{sb[1], sb[0]};
    sb[1] = '{0, 0};
    #2 $display("T|td=%p", td);
    k = new; k.swap(); $display("T|ks=%p", k.ks);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|sa='{'{a:3, b:4}, '{a:1, b:2}}",
            "T|ta='{'{x:4, y:'{5, 6}}, '{x:1, y:'{2, 3}}}",
            "T|m2='{'{'{a:3, b:4}, '{a:1, b:2}}, '{'{a:5, b:6}, '{a:7, b:8}}}",
            "T|tb='{'{a:9, b:10}, '{a:5, b:6}}",
            "T|tc='{'{a:7, b:8}, '{a:9, b:10}}",
            "T|td='{'{a:7, b:8}, '{a:5, b:6}}",
            "T|ks='{'{a:3, b:4}, '{a:1, b:2}}",
        ]
    );
}
