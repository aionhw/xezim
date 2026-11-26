//! IEEE 1800-2023 §20.6.1, §8.25: `$typename` of a class property declared
//! with a specialization names that specialization, a parameter of the
//! holding class read in the holding object's specialization; it named the
//! holding object's class, or the default specialization.

#[test]
fn typename_of_a_specialized_class_property() {
    let sim = xezim::simulate(
        r#"
class drv_c #(int W = 8);
endclass
class t_c #(type T = int);
endclass
class holder_c #(int W = 8);
  drv_c #(.W(W)) p;
  drv_c #(W) q;
  drv_c #(4) r;
  drv_c s;
  t_c #(byte) t;
endclass
module top;
  holder_c #(16) h = new;
  drv_c #(16) m;
  initial begin
    holder_c #(32) l = new;
    $display("T|%s|%s|%s|%s|%s", $typename(h.p), $typename(h.q), $typename(h.r), $typename(h.s), $typename(h.t));
    $display("T|%s|%s", $typename(m), $typename(h));
    $display("T|%s|%s", $typename(l.p), $typename(l.q));
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|class drv_c #(16)|class drv_c #(16)|class drv_c #(4)|class drv_c #(8)|class t_c #(byte)",
            "T|class drv_c #(16)|class holder_c #(16)",
            "T|class drv_c #(32)|class drv_c #(32)",
        ]
    );
}
