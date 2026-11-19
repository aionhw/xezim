//! IEEE 1800-2023 §9.4.5, §10.9.1, §10.10: an unpacked array item of a
//! pattern with an intra-assignment delay supplies its elements when the
//! target is a class property, whose shape the capture does not know, as
//! it does for a module variable.

use xezim::simulate;

#[test]
fn delayed_array_items_reach_a_class_property() {
    let sim = simulate(
        r#"
module top;
  class K;
    int km [2] = '{51, 52};
    int kt [2][2];
    int ku [2][2];
    task run();
      kt = #1 '{km, km};
      ku = #1 '{km, '{7, 8}};
    endtask
  endclass
  K k;
  initial begin
    k = new;
    fork
      k.run();
    join_none
    #0 k.km = '{0, 0};
    #3 $display("kt=%p ku=%p", k.kt, k.ku);
  end
endmodule
"#,
        1000,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(msgs, ["kt='{'{51, 52}, '{51, 52}} ku='{'{0, 0}, '{7, 8}}"]);
}
