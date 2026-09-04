//! A bit-select of a packed-struct PORT member (`inp.sram_renA[2]`) inside an
//! instance compiled as a two-bit element select whenever ANY other module
//! declared a packed multi-dimensional array with the same leaf name
//! (`logic [3:0][1:0] sram_renA`): the compiler's element-width lookup fell
//! back to the bare leaf name, which the elaborator keys per module. Bits 2
//! and 3 of the member then read x while bit 0 happened to read 0.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const DESIGN: &str = r#"
module rdbuf_mem;
  logic [3:0][1:0] sram_renA;
  logic [1:0] cen_q;
  initial sram_renA = '0;
  assign cen_q = sram_renA[2];
endmodule

typedef struct packed {
  logic [3:0] sram_renA;
  logic [3:0] sram_wenB;
} prebuf_in_t;

module prebuf_consumer (input prebuf_in_t inp);
  logic cen_b0, cen_b2, cen_b3;
  always @(*) begin
    cen_b0 = inp.sram_renA[0];
    cen_b2 = inp.sram_renA[2];
    cen_b3 = inp.sram_renA[3];
  end
  initial #5 $display("INTERP b2=%b b3=%b whole=%b", inp.sram_renA[2], inp.sram_renA[3], inp.sram_renA);
endmodule

module mwe_top;
  prebuf_in_t net_inp;
  rdbuf_mem u_rdbuf_mem ();
  prebuf_consumer u_consumer (.inp(net_inp));
  initial begin
    net_inp.sram_renA = 4'b0100;
    net_inp.sram_wenB = 4'b0000;
    #10;
    $display("COMB q=%b b0=%b b2=%b b3=%b", u_rdbuf_mem.cen_q, u_consumer.cen_b0, u_consumer.cen_b2, u_consumer.cen_b3);
    $finish;
  end
endmodule
"#;

#[test]
fn struct_member_bit_select_ignores_same_named_array_elsewhere() {
    let msgs = messages(DESIGN);
    assert!(msgs.iter().any(|m| m == "COMB q=00 b0=0 b2=1 b3=0"), "compiled comb block; got {msgs:?}");
    assert!(msgs.iter().any(|m| m == "INTERP b2=1 b3=0 whole=0100"), "interpreter reads; got {msgs:?}");
}

// Siblings of the same collision, all red before the fix: a RANGE select on
// the port member (`inp.mem[7:4]` read xxxx) and a bit-select on a struct
// VARIABLE's member (`s.mem[2]` read 0) in a design where another module
// declares `logic [3:0][1:0] mem`. Nested members (`inp.sub.f[2]`),
// unpacked arrays, and plain same-named variables were never affected.
#[test]
fn range_select_and_struct_variable_member_ignore_same_named_array() {
    let msgs = messages(
        r#"
typedef struct packed { logic [7:0] mem; logic [7:0] pad; } sp_t;
module arr_m; logic [3:0][1:0] mem; logic [1:0] o; initial mem = 8'hE4; assign o = mem[2]; endmodule
module port_m (input sp_t inp);
  logic o; logic [3:0] r;
  always @(*) begin o = inp.mem[2]; r = inp.mem[7:4]; end
endmodule
module var_m;
  sp_t s; logic o;
  initial begin s.mem = 8'b0000_0100; s.pad = 0; end
  always @(*) o = s.mem[2];
endmodule
module top;
  sp_t sp; arr_m ua(); port_m up(.inp(sp)); var_m uv();
  initial begin
    sp.mem = 8'b1010_0100; sp.pad = 0;
    #10;
    $display("P port=%b range=%b var=%b arr=%b", up.o, up.r, uv.o, ua.o);
    $finish;
  end
endmodule
"#,
    );
    assert!(msgs.iter().any(|m| m == "P port=1 range=1010 var=1 arr=10"), "got {msgs:?}");
}
