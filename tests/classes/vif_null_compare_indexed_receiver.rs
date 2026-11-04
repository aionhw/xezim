//! §25.9, §7.4: `cfg.kids[0].vif == null` compares the vif BINDING of the
//! object the indexed element names. A receiver with an element select
//! (dynamic array, queue, fixed array, local array) was unclassifiable, so
//! a bound vif compared equal to null while a plain handle alias of the
//! same element compared correctly. Expected values are the reference
//! simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn indexed_receiver_vif_null_comparison() {
    const SRC: &str = r#"interface c_if; endinterface
class cc;
  virtual c_if vif;
endclass
class cfg_t;
  cc kids[];
  cc q[$];
  cc fixed[2];
endclass
module top;
  c_if b();
  initial begin
    cfg_t cfg = new();
    cc arr[2];
    cfg.kids = new[2];
    cfg.kids[0] = new();
    cfg.kids[1] = new();
    cfg.kids[0].vif = b;
    cfg.q.push_back(cfg.kids[0]);
    cfg.fixed[1] = cfg.kids[0];
    arr[0] = cfg.kids[0];
    $display("T|dyn %0d", cfg.kids[0].vif == null);
    $display("T|dyn-unbound %0d", cfg.kids[1].vif == null);
    $display("T|dyn!= %0d", cfg.kids[0].vif != null);
    $display("T|queue %0d", cfg.q[0].vif == null);
    $display("T|fixed %0d", cfg.fixed[1].vif == null);
    $display("T|local %0d", arr[0].vif == null);
    $display("T|same %0d", cfg.kids[0].vif == arr[0].vif);
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|dyn 0",
            "T|dyn-unbound 1",
            "T|dyn!= 1",
            "T|queue 0",
            "T|fixed 0",
            "T|local 0",
            "T|same 1",
        ]
    );
}
