//! Recursive `%p` (IEEE 1800-2017 §21.2.1.7): the assignment-pattern form must
//! descend through nested structs, unpacked arrays of structs and packed
//! members, print enum members by LABEL, quote strings, and render reals as
//! reals. Previously `%p` was one level deep — a nested aggregate printed `x`.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef enum bit [1:0] { IDLE, BUSY, ERR } st_e;
  typedef struct packed { bit [3:0] vlan; bit [11:0] id; } tag_t;
  typedef struct { st_e status; tag_t tag; string name; } node_t;
  typedef struct { int cid; node_t nodes[2]; real eff; } cluster_t;

  cluster_t c;
  node_t    arr[2];

  initial begin
    c.cid = 999;
    c.nodes[0].status = BUSY;
    c.nodes[0].tag    = '{vlan: 4'hA, id: 12'h5F3};
    c.nodes[0].name   = "ALPHA";
    c.nodes[1].status = ERR;
    c.nodes[1].tag    = '{vlan: 4'hF, id: 12'hFFF};
    c.nodes[1].name   = "BETA";
    c.eff = 0.5;

    arr[0].status = IDLE; arr[0].tag = '{vlan:4'h1, id:12'h002}; arr[0].name = "A0";
    arr[1].status = BUSY; arr[1].tag = '{vlan:4'h3, id:12'h004}; arr[1].name = "A1";

    $display("CLUSTER=%p", c);
    $display("ARR=%p", arr);
  end
endmodule
"#;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn p_format_descends_nested_aggregates() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Nested struct + array-of-struct + packed member + enum label + string + real.
    assert_eq!(
        line(&sim, "CLUSTER="),
        concat!(
            "CLUSTER='{cid:999, nodes:'{",
            "'{status:BUSY, tag:'{vlan:10, id:1523}, name:\"ALPHA\"}, ",
            "'{status:ERR, tag:'{vlan:15, id:4095}, name:\"BETA\"}",
            "}, eff:0.5}"
        )
    );

    // A top-level unpacked array of structs prints as an element list.
    assert_eq!(
        line(&sim, "ARR="),
        concat!(
            "ARR='{",
            "'{status:IDLE, tag:'{vlan:1, id:2}, name:\"A0\"}, ",
            "'{status:BUSY, tag:'{vlan:3, id:4}, name:\"A1\"}",
            "}"
        )
    );
}
