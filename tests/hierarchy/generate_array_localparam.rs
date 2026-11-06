//! §27, §6.20.2, §7.4: a parameter declared in a generate scope has the same
//! shapes as one in the module body. An unpacked array read as zero, in place
//! and through a child's parameter override, and an index of a packed array
//! bit-selected.

use xezim::{simulate, simulate_multi};

/// `(name, value)` for each expected signal, under a named top and under the
/// top the simulator finds itself. Only the named top showed the bug.
#[track_caller]
fn expect(src: &str, max_time: u64, want: &[(&str, u64)]) {
    let named = simulate_multi(
        &[src.to_string()],
        max_time,
        Some("tb"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let found = simulate(src, max_time).expect("simulate failed");
    for (how, sim) in [("named top", named), ("found top", found)] {
        let read = |n: &str| sim.get_signal(n).and_then(|v| v.to_u64());
        for (name, value) in want {
            let got = read(name).or_else(|| read(&format!("tb.{name}")));
            assert_eq!(got, Some(*value), "{name} under a {how}");
        }
    }
}

/// Each design drives `y0` and `y1`, expected to read 9 and 0x3FF.
#[track_caller]
fn reads_both(body: &str) {
    let src = format!(
        "module child #(parameter logic [31:0] W [2] = '{{default: '0}})
           (output logic [31:0] o0, output logic [31:0] o1);
           assign o0 = W[0];
           assign o1 = W[1];
         endmodule
         module tb;
           wire [31:0] y0, y1;
           {body}
         endmodule"
    );
    expect(&src, 10, &[("y0", 9), ("y1", 0x3ff)]);
}

const L: &str = "localparam logic [31:0] L [2] = '{32'h9, 32'h3FF};";

#[test]
fn read_in_place() {
    reads_both(&format!(
        "if (1) begin : g {L} assign y0 = L[0]; assign y1 = L[1]; end"
    ));
}

#[test]
fn read_in_an_else_branch() {
    reads_both(&format!(
        "if (0) begin : o assign y0 = 0; assign y1 = 0; end
         else begin : g {L} assign y0 = L[0]; assign y1 = L[1]; end"
    ));
}

#[test]
fn read_in_a_generate_for() {
    reads_both(&format!(
        "for (genvar i = 0; i < 1; i++) begin : g {L}
           assign y0 = L[0]; assign y1 = L[1];
         end"
    ));
}

#[test]
fn read_in_a_generate_case() {
    reads_both(&format!(
        "case (1) 1: begin : g {L} assign y0 = L[0]; assign y1 = L[1]; end endcase"
    ));
}

#[test]
fn passed_to_a_child_as_an_override() {
    reads_both(&format!(
        "if (1) begin : g {L} child #(.W(L)) c (.o0(y0), .o1(y1)); end"
    ));
}

#[test]
fn read_through_a_variable_index() {
    reads_both(&format!(
        "if (1) begin : g {L}
           int i = 0;
           logic [31:0] a, b;
           initial begin i = 0; #1 a = L[i]; i = 1; #1 b = L[i]; end
           assign y0 = a; assign y1 = b;
         end"
    ));
}

#[test]
fn dimension_from_a_sibling_localparam() {
    reads_both(
        "if (1) begin : g
           localparam int N = 2;
           localparam logic [31:0] L [N] = '{32'h9, 32'h3FF};
           assign y0 = L[0]; assign y1 = L[1];
         end",
    );
}

#[test]
fn packed_two_dimensional() {
    reads_both(
        "if (1) begin : g
           localparam logic [1:0][31:0] L = {32'h3FF, 32'h9};
           assign y0 = L[0]; assign y1 = L[1];
         end",
    );
}

#[test]
fn in_a_generate_scope_of_an_instance() {
    let src = "module m #(parameter bit EN = 1) (output logic [31:0] o0, output logic [31:0] o1);
        if (!EN) begin : g_off assign o0 = 0; assign o1 = 0; end
        else begin : g_on
          localparam int unsigned N = 2;
          localparam logic [31:0] W [N] = '{32'h9, 32'h3FF};
          assign o0 = W[0]; assign o1 = W[1];
        end
      endmodule
      module tb; wire [31:0] y0, y1; m u (.o0(y0), .o1(y1)); endmodule";
    expect(src, 10, &[("y0", 9), ("y1", 0x3ff)]);
}

/// A write through the mask this reduces is what the NPU's registers depend on.
#[test]
fn drives_a_register_write_mask() {
    let src = "module tb;
      logic clk = 0; always #5 clk = ~clk;
      logic [31:0] r = 0, y;
      if (1) begin : g
        localparam logic [31:0] WMASK [1] = '{32'h0000_00FF};
        always_ff @(posedge clk) r <= (r & ~WMASK[0]) | (32'hABCD_EF12 & WMASK[0]);
      end
      assign y = r;
    endmodule";
    expect(src, 20, &[("y", 0x12)]);
}

/// An unpacked array sized by a generate-scope localparam has fixed elements,
/// so a bit select of one is a bit select (§7.4, §11.5.1). It read `x`, while a
/// part select of the same element read the right bits.
#[test]
fn bit_select_of_an_array_sized_by_a_sibling_localparam() {
    let src = "module child #(parameter int N = 4) (output logic [31:0] o [N]);
      logic [31:0] r [N];
      initial begin r[0] = 32'h9; r[1] = 0; r[2] = 32'h3FF; r[3] = 0; end
      assign o = r;
    endmodule
    module tb;
      wire b0, b3;
      wire [3:0] n;
      if (1) begin : g
        localparam int unsigned N = 4;
        localparam int unsigned K = 0;
        logic [31:0] r [N];
        child #(.N(N)) c (.o(r));
        assign b0 = r[K][0];
        assign b3 = r[K][3];
        assign n = r[K][3:0];
      end
    endmodule";
    expect(src, 10, &[("b0", 1), ("b3", 1), ("n", 9)]);
}
