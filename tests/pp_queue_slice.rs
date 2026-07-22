//! §21.2.1.7 — `%p` of a DIRECT (unassigned) queue/array slice must render the
//! element list `'{...}`, not the concatenated packed value. `q[2:$]` resolves
//! `$` to the last live index. Verified against a commercial simulator.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn direct_queue_slice_p_format() {
    let src = r#"
module t;
  int q[$] = '{1,2,3,4,5};
  initial begin
    $display("A=%p", q[1:3]);   // '{2, 3, 4}
    $display("B=%p", q[2:$]);   // '{3, 4, 5}
    $display("C=%p", q[0:0]);   // '{1}
    $finish;
  end
endmodule
"#;
    let out = line(src);
    for w in ["A='{2, 3, 4}", "B='{3, 4, 5}", "C='{1}"] {
        assert!(out.iter().any(|m| m == w), "missing {:?}; got {:?}", w, out);
    }
}
