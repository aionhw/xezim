//! IEEE 1800-2023 §23.8, §23.8.1 (#721): the scope tree upward names are
//! resolved against holds each instance path and definition name once. The
//! names still resolve per instance: a call of an enclosing module's
//! function, a reference headed by the definition name of an enclosing
//! instance, and one headed by the name of a sibling instance that the top
//! module also uses. Expected lines are the reference simulator's.

use xezim::simulate;

#[test]
fn upward_names_resolve_in_each_instance_of_repeated_definitions() {
    let sim = simulate(
        r#"
module probe(output logic [7:0] result, output logic [7:0] via_def, output logic [7:0] via_sib);
  initial begin
    result = read_id();
    #1;
    via_def = cluster.tag;
    via_sib = helper.tag;
  end
endmodule
module helper_unit #(parameter int T = 0);
  logic [7:0] tag = T;
endmodule
module cluster #(parameter int ID = 1)(output wire [7:0] result, output wire [7:0] d, output wire [7:0] s);
  logic [7:0] tag = ID + 100;
  function automatic int read_id();
    return ID;
  endfunction
  helper_unit #(.T(ID + 50)) helper();
  for (genvar i = 0; i < 2; i++) begin : g
    helper_unit #(.T(ID * 10 + i)) h();
  end
  probe nested(.result(result), .via_def(d), .via_sib(s));
endmodule
module top;
  wire [7:0] a, b, ad, bd, as, bs;
  cluster #(.ID(3)) first(.result(a), .d(ad), .s(as));
  cluster #(.ID(9)) second(.result(b), .d(bd), .s(bs));
  helper_unit #(.T(77)) helper();
  initial begin
    #5;
    $display("T|ids %0d %0d", a, b);
    $display("T|by definition name %0d %0d", ad, bd);
    $display("T|by sibling instance name %0d %0d", as, bs);
    $display("T|generated %0d %0d %0d %0d", first.g[0].h.tag, first.g[1].h.tag,
             second.g[0].h.tag, second.g[1].h.tag);
    $finish;
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|ids 3 9",
            "T|by definition name 103 109",
            "T|by sibling instance name 53 59",
            "T|generated 30 31 90 91",
        ]
    );
}
