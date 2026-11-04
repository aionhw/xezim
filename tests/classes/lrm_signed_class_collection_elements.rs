//! IEEE 1800-2023 §6.11.1 / §7.5 / §7.10: elements of a class queue / dynamic-array property with a signed element type keep that signedness (`byte dyn[]` element 8'hff reads -1).
//!
//! Expected lines come from the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const SGN: &str = r#"
// top: t8_5
class P; byte dyn[]; byte fa[2]; endclass
module t8_5;
  P p; byte md[]; 
  initial begin p = new; p.dyn = new[2]; p.dyn[1] = 8'hff; p.fa[1] = -2; md = new[1]; md[0] = -1;
    $display("T|a|cls dyn=%p fa=%p mod=%p el=%0d", p.dyn, p.fa, md, p.dyn[1]); end
endmodule
"#;

const SGN_FAMILY: &str = r#"
class P; byte dyn[]; shortint sd[]; int id[]; longint ld[]; logic signed [7:0] ls[];
  byte q[$]; shortint sq[$]; byte aa[int]; int ia[string]; byte fa[2];
  bit signed [3:0] b4[]; byte unsigned ub[];
endclass
module t;
  P p; int x;
  initial begin p = new;
    p.dyn = new[2]; p.dyn[1] = 8'hff;
    p.sd = new[1]; p.sd[0] = 16'hfffe;
    p.id = new[1]; p.id[0] = -5;
    p.ld = new[1]; p.ld[0] = -7;
    p.ls = new[1]; p.ls[0] = 8'h80;
    p.q.push_back(8'hfd); p.sq.push_back(-300);
    p.aa[3] = 8'hf0; p.ia["k"] = -9;
    p.b4 = new[1]; p.b4[0] = 4'hf; p.ub = new[1]; p.ub[0] = 8'hff;
    $display("T|a|dyn=%p el=%0d sd=%p el=%0d id=%0d ld=%0d ls=%0d", p.dyn, p.dyn[1], p.sd, p.sd[0], p.id[0], p.ld[0], p.ls[0]);
    $display("T|b|q=%p el=%0d sq=%0d aa=%p el=%0d ia=%0d", p.q, p.q[0], p.sq[0], p.aa, p.aa[3], p.ia["k"]);
    $display("T|c|b4=%0d ub=%0d lt=%0d sum=%0d", p.b4[0], p.ub[0], p.dyn[1] < 0, p.dyn.sum());
    x = p.dyn[1]; $display("T|d|x=%0d pop=%0d min=%p", x, p.q.pop_front(), p.dyn.min());
    foreach (p.dyn[i]) $display("T|e|%0d %0d", i, p.dyn[i]);
  end
endmodule
"#;

#[test]
fn audit_repro() {
    let want = ["T|a|cls dyn='{0, -1} fa='{0, -2} mod='{-1} el=-1"];
    assert_eq!(t_lines(SGN), want);
}

#[test]
fn signed_element_types() {
    let want = [
        "T|a|dyn='{0, -1} el=-1 sd='{-2} el=-2 id=-5 ld=-7 ls=-128",
        "T|b|q='{-3} el=-3 sq=-300 aa='{3:-16} el=-16 ia=-9",
        "T|c|b4=-1 ub=255 lt=1 sum=-1",
        "T|d|x=-1 pop=-3 min='{-1}",
        "T|e|0 0",
        "T|e|1 -1",
    ];
    assert_eq!(t_lines(SGN_FAMILY), want);
}
