//! IEEE 1800-2023 §16.12.8, §16.12.10, §16.12.11, §16.12.13, §16.12.16:
//! `implies`, `nexttime [n]`, `s_nexttime [n]`, the ranged `always`,
//! `s_always`, `eventually` and `s_eventually`, and the property `case`
//! (aionhw/xezim-core#220) run end to end. Each assertion's pass and fail
//! times, pass actions included, are the reference simulator's. `implies`
//! starts both operands at the same tick and is vacuous when its antecedent
//! fails (§16.14.8 z), so it runs no pass action then; as `(not p) or q` it
//! ran one, and an `x` antecedent failed it. The strong forms fail at the
//! end of simulation while an obligation is open (§16.12.2).
use std::collections::BTreeMap;
use xezim::simulate;

const SRC: &str = r#"// Each assertion prints its failures (F) and passes (P) with the tick time.
module t;
  initial $assertvacuousoff(0);
  logic clk = 0;
  logic a, b, c;
  logic [1:0] m;
  logic [31:0] pa = 32'b1011_0110_1110_0101_1001_1101_0011_1011;
  logic [31:0] pb = 32'b1101_1011_0111_0110_1110_0011_1101_0110;
  logic [31:0] pc = 32'b0110_1101_1011_1001_0111_1100_1010_1101;
  int k = 0;
  always #5 clk = ~clk;
  always @(negedge clk) begin
    a <= pa[k]; b <= pb[k]; c <= pc[k]; m <= {pb[k], pa[k]}; k <= k + 1;
  end
  im1: assert property (@(posedge clk) a implies b) $display("im1 P %0t", $time); else $display("im1 F %0t", $time);
  im2: assert property (@(posedge clk) (a ##1 b) implies (b ##1 c)) $display("im2 P %0t", $time); else $display("im2 F %0t", $time);
  nx1: assert property (@(posedge clk) a |-> nexttime [2] b) $display("nx1 P %0t", $time); else $display("nx1 F %0t", $time);
  nx2: assert property (@(posedge clk) a |-> s_nexttime [2] b) $display("nx2 P %0t", $time); else $display("nx2 F %0t", $time);
  nx3: assert property (@(posedge clk) a |-> nexttime [0] b) $display("nx3 P %0t", $time); else $display("nx3 F %0t", $time);
  al1: assert property (@(posedge clk) a |-> always [1:2] b) $display("al1 P %0t", $time); else $display("al1 F %0t", $time);
  al2: assert property (@(posedge clk) a |-> s_always [0:1] c) $display("al2 P %0t", $time); else $display("al2 F %0t", $time);
  al3: assert property (@(posedge clk) a |-> always [2:$] (b || c)) $display("al3 P %0t", $time); else $display("al3 F %0t", $time);
  ev1: assert property (@(posedge clk) a |-> eventually [1:3] (b && c)) $display("ev1 P %0t", $time); else $display("ev1 F %0t", $time);
  ev2: assert property (@(posedge clk) a |-> s_eventually [1:2] (b && !c)) $display("ev2 P %0t", $time); else $display("ev2 F %0t", $time);
  ev3: assert property (@(posedge clk) a |-> s_eventually [2:$] (b && c && !a)) $display("ev3 P %0t", $time); else $display("ev3 F %0t", $time);
  property pc1;
    @(posedge clk) case (m) 2'd0: b; 2'd1, 2'd2: c; default: a ##1 b; endcase
  endproperty
  cs1: assert property (pc1) $display("cs1 P %0t", $time); else $display("cs1 F %0t", $time);
  property pc2;
    @(posedge clk) case (m) 2'd3: b |-> ##1 c; endcase
  endproperty
  cs2: assert property (pc2) $display("cs2 P %0t", $time); else $display("cs2 F %0t", $time);
  initial #127 $finish;
endmodule
"#;

#[test]
fn property_operators_match_their_definitions() {
    let result = simulate(SRC, 1_000).unwrap();
    let mut got: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in &result.output {
        let f: Vec<&str> = entry.message.split_whitespace().collect();
        if f.len() == 3 && (f[1] == "P" || f[1] == "F") {
            got.entry(f[0].to_string())
                .or_default()
                .push(format!("{} {}", f[1], f[2]));
        }
    }
    let mut want: BTreeMap<String, Vec<String>> = [
        ("al1", "P 35 F 45 F 65 F 65 P 85 F 115 F 125"),
        ("al2", "F 25 F 25 F 55 F 55 F 75 F 95 P 125 F 127"),
        (
            "cs1",
            "F 5 P 15 P 35 P 35 P 45 F 65 P 65 F 75 P 85 P 105 F 105 P 115 P 125",
        ),
        ("cs2", "P 35 P 65 F 105"),
        ("ev1", "P 35 P 35 F 75 P 85 P 85 F 125"),
        ("ev2", "P 25 F 45 P 55 P 75 P 75 P 105 F 127 F 127"),
        ("ev3", "P 35 P 85 P 85 P 85 P 85 F 127 F 127 F 127"),
        ("im1", "F 15 P 25 F 45 P 55 F 65 P 95 F 115 F 125"),
        ("im2", "F 25 P 35 F 55 F 75 F 105 F 127"),
        ("nx1", "P 35 F 45 F 65 P 75 P 85 F 115"),
        ("nx2", "P 35 F 45 F 65 P 75 P 85 F 115 F 127 F 127"),
        ("nx3", "F 15 P 25 F 45 P 55 F 65 P 95 F 115 F 125"),
    ]
    .into_iter()
    .map(|(k, v)| {
        (
            k.to_string(),
            v.split(' ')
                .collect::<Vec<_>>()
                .chunks(2)
                .map(|c| c.join(" "))
                .collect(),
        )
    })
    .collect();
    // Results in one time step may come in either order.
    let by_time = |v: &mut Vec<String>| {
        v.sort_by_key(|r| {
            let (kind, t) = r.split_once(' ').unwrap();
            (t.parse::<u64>().unwrap(), kind.to_string())
        })
    };
    got.values_mut().for_each(by_time);
    want.values_mut().for_each(by_time);
    assert_eq!(got, want);
}
