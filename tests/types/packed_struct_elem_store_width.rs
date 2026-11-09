//! IEEE 1800-2017 §7.4.1, §7.4.5, §10.4.2, §10.3: on `le_t [1:0] b [2]`
//! with `le_t` a packed struct, `b[0][1]` is one 40-bit struct and a store
//! to it writes all 40 bits. Nonblocking stores, continuous assignments and
//! delayed nonblocking stores were cut to the width of the packed array
//! dimensions alone, which leave out the struct's own width: one bit for a
//! whole element, two for a row of two elements. Blocking stores were right.
//! Expected values are from the reference simulator.
//!
//! Two packed selects into a typedef-named struct array (`le_t [1:0][1:0]
//! dt [2]`, `dt[1][1][0]`) still need the packed dimensions of a typedef
//! type from the elaborator (aionhw/xezim#984), so the two-level case here
//! uses an inline struct type.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m.trim() == want),
        "missing {want:?} in {msgs:#?}"
    );
}

#[test]
fn nonblocking_store_of_a_struct_element() {
    let msgs = messages(
        "package q; typedef struct packed { logic [31:0] pc; logic [7:0] m; } ent_t; endpackage
module t;
  typedef logic [39:0] w_t;
  typedef struct packed { logic [31:0] pc; logic [7:0] m; } le_t;
  logic clk = 0;
  logic [39:0] v = 40'hDEAD0000A5;
  q::ent_t [1:0] a [1:0];
  le_t [1:0] b [1:0];
  w_t [1:0] c [1:0];
  struct packed { logic [31:0] pc; logic [7:0] m; } [1:0] d [1:0];
  q::ent_t [1:0] e;
  q::ent_t [1:0] f [2][2];
  int i = 1;
  always @(posedge clk) begin
    a[0][1] <= v; b[0][1] <= v; c[0][1] <= v; d[0][1] <= v; e[1] <= v;
    f[1][1][0] <= v; a[i][i] <= v;
  end
  initial begin
    #1 clk = 1;
    #1 $display(\"pkgT=%h localT=%h typedefvec=%h inline=%h packedonly=%h twoD=%h dyn=%h\",
                a[0][1], b[0][1], c[0][1], d[0][1], e[1], f[1][1][0], a[1][1]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "pkgT=dead0000a5 localT=dead0000a5 typedefvec=dead0000a5 inline=dead0000a5 \
         packedonly=dead0000a5 twoD=dead0000a5 dyn=dead0000a5",
    );
}

#[test]
fn ascending_offset_three_dimensional_and_row_stores() {
    let msgs = messages(
        "module t;
  typedef struct packed { logic [31:0] pc; logic [7:0] m; } le_t;
  logic clk = 0;
  logic [39:0] v = 40'hDEAD0000A5, v2 = 40'h1234567890;
  le_t [0:1] u [2];
  le_t [2:1] o [1:2];
  le_t [1:0] k [2][2][2];
  struct packed { logic [31:0] pc; logic [7:0] m; } [1:0][1:0] dd [2];
  always @(posedge clk) begin
    u[1][0] <= v; u[1][1] <= v2;
    o[2][1] <= v; o[2][2] <= v2;
    k[1][0][1][1] <= v;
    dd[1][1][0] <= v; dd[1][0] <= {v2, v};
  end
  initial begin
    k[1][0][1] = '0; dd[1] = '0;
    #1 clk = 1;
    #1 $display(\"u=%h o=%h k=%h dd=%h\", u[1], o[2], k[1][0][1], dd[1]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "u=dead0000a51234567890 o=1234567890dead0000a5 k=dead0000a50000000000 \
         dd=0000000000dead0000a51234567890dead0000a5",
    );
}

#[test]
fn continuous_always_ff_and_delayed_stores() {
    let msgs = messages(
        "module t;
  typedef struct packed { logic [31:0] pc; logic [7:0] m; } le_t;
  logic clk = 0;
  logic [39:0] v = 40'hDEAD0000A5;
  le_t [1:0] ca [2];
  le_t [1:0] ff [2];
  le_t [1:0] dl [2];
  assign ca[0][1] = v;
  assign ca[0][0] = 40'h0;
  assign ca[1] = 80'h0;
  always_ff @(posedge clk) ff[1][0] <= v;
  initial begin
    ff[1] = '0; dl[0] = '0;
    dl[0][1] <= #1 v;
    #1 clk = 1;
    #1 $display(\"ca=%h ff=%h dl=%h\", ca[0], ff[1], dl[0]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "ca=dead0000a50000000000 ff=0000000000dead0000a5 dl=dead0000a50000000000",
    );
}

/// Guard: member and part-select stores, whole rows, blocking stores, a
/// packed-only array of structs and a typedef vector element already store
/// the full value.
#[test]
fn stores_that_already_work() {
    let msgs = messages(
        "module t;
  typedef struct packed { logic [31:0] pc; logic [7:0] m; } le_t;
  logic clk = 0;
  logic [39:0] v = 40'hDEAD0000A5, v2 = 40'h1234567890;
  le_t [1:0] n [2];
  le_t [1:0] s [2];
  le_t [1:0] b [2];
  le_t [1:0] e;
  typedef logic [39:0] w_t;
  w_t [1:0] w [2];
  always @(posedge clk) begin
    n[0][1].pc <= 32'hCAFEF00D; n[0][1].m <= 8'h5A; n[0][0][7:0] <= 8'h77;
    s[1] <= {v, v2};
    b[1][0] = v;
    e[1] <= v;
    w[0][1] <= v;
  end
  initial begin
    n[0] = '0; b[1] = '0; e = '0; w[0] = '0;
    b[0][1] = v2;
    #1 clk = 1;
    #1 $display(\"n=%h s=%h b=%h b0=%h e=%h w=%h\", n[0], s[1], b[1], b[0][1], e, w[0]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "n=cafef00d5a0000000077 s=dead0000a51234567890 b=0000000000dead0000a5 \
         b0=1234567890 e=dead0000a50000000000 w=dead0000a50000000000",
    );
}
