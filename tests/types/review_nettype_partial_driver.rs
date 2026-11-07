//! §10.3: "the left-hand side of a continuous assignment to a net of a
//! user-defined nettype shall not contain any indexing or select operations
//! into the data type of the nettype". `assign p.V = v;` elaborated and gave
//! each member a driver of its own. Reading a member and driving the whole net
//! stay legal. Cross-checked against the reference simulator.

const TYPES: &str = "
typedef struct { real V; int code; } e_t;
nettype e_t enet;
";

fn sim(body: &str) -> Result<Vec<String>, String> {
    xezim::simulate(&format!("{TYPES}{body}"), 1_000_000).map(|r| {
        r.output
            .iter()
            .map(|o| o.message.trim().to_string())
            .collect()
    })
}

#[test]
fn member_select_on_the_left_is_rejected() {
    let cases = [
        (
            "n",
            "module top; real v = 1.0; enet n; assign n.V = v; endmodule",
        ),
        (
            "p",
            "module c(input real v, output enet p); assign p.V = v; assign p.code = 1; endmodule
             module top; real v; enet n; c u(.v(v), .p(n)); endmodule",
        ),
        (
            "p",
            "module c(input real v, inout enet p); assign p.code = 1; endmodule
             module top; real v; enet n; c u(.v(v), .p(n)); endmodule",
        ),
    ];
    for (net, body) in cases {
        match sim(body) {
            Ok(out) => panic!("accepted: {body}\n{out:?}"),
            Err(e) => assert!(
                e.contains(&format!(
                    "net '{net}' of a user-defined nettype selects part of it"
                )),
                "{body}\n{e}"
            ),
        }
    }
}

#[test]
fn member_reads_and_whole_drivers_stay_legal() {
    let out = sim("
module rd(input enet p, output real o); assign o = p.V; endmodule
module top;
  real v = 1.0, o, w;
  enet n;
  assign n = '{v, 2};
  assign w = n.V;
  rd r(.p(n), .o(o));
  initial #1 $display(\"R %.1f %.1f %0d\", o, w, n.code);
endmodule
")
    .expect("legal nettype use rejected");
    assert!(out.iter().any(|l| l == "R 1.0 1.0 2"), "{out:?}");
}
