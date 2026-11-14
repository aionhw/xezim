//! §4.6 b), §10.4.2: nonblocking updates to one variable commit in the order
//! they were scheduled. A whole-variable update after a partial one wins; a
//! partial update after a whole one lands on top of it; and a partial update
//! with no whole one is applied to the value the variable holds in the NBA
//! region, so a blocking write to its other bits in the meantime survives
//! (#1050). Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// A partial update followed by a blocking write to the variable's other
/// bits, in one process and across two; a whole update after a partial one
/// of an unknown variable.
#[test]
fn a_partial_update_keeps_a_later_blocking_write_to_other_bits() {
    assert_eq!(
        lines(
            r#"module t;
  typedef struct packed { logic vld; logic [7:0] addr; } p_t;
  p_t q, qb; logic [7:0] w, vb, x;
  event e;
  initial begin
    q = '0; qb = '0; vb = 0; w = 0;
    #1;
    q.vld <= 1'b1;
    q.addr = 8'h55;
    {w[7], w[0]} <= 2'b11;
    w[3:1] = 3'b111;
    x[3:0] <= 4'h5;
    x <= x + 1;
    #1 $display("I q.vld=%b q.addr=%h w=%h x=%h", q.vld, q.addr, w, x);
    #1 vb[7:6] <= 2'b11; qb.vld <= 1'b1; -> e;
    #1 $display("X vb=%h qb.vld=%b qb.addr=%h", vb, qb.vld, qb.addr);
    $finish;
  end
  initial begin @e; vb[1] = 1'b1; qb.addr = 8'h77; end
endmodule
"#
        ),
        [
            r#"I q.vld=1 q.addr=55 w=8f x=xx"#,
            r#"X vb=c2 qb.vld=1 qb.addr=77"#,
        ]
    );
}

/// The partial-then-whole order on a module variable, its reverse and a
/// triple, a variable indexed part-select, a concatenation, an interface
/// variable, a submodule variable, a package variable, an unpacked-array
/// element, a class property, a static property, a task, a virtual interface
/// and an intra-assignment delay.
#[test]
fn partial_and_whole_updates_commit_in_schedule_order_in_every_scope() {
    assert_eq!(
        lines(
            r#"// partial/whole NBA ordering across contexts and lvalue kinds
package pk; typedef struct packed { logic vld; logic [6:0] d; } p_t; p_t pv; endpackage
interface ifc; import pk::*; p_t b; modport mp(output b); endinterface
module sub; import pk::*; p_t s; endmodule
module t;
  import pk::*;
  p_t m, m2, m3, m4, arr[2]; logic [15:0] v, v2; int sel = 1;
  ifc i(); sub u();
  logic clk = 0;
  class C; p_t p; logic [63:0] w; static p_t sp; endclass
  C c;
  task tk; m4.vld <= 1'b0; m4 <= 8'hFF; endtask
  task automatic drive(virtual ifc vi); vi.b.vld <= 1'b0; vi.b <= 8'hC3; endtask
  initial begin
    c = new;
    m = 0; m2 = 0; m3 = 0; v = 0; v2 = 0; i.b = 0; u.s = 0; pv = 0; arr[0] = 0; arr[1] = 0; m4 = 0;
    c.p = 0; C::sp = 0;
    #1;
    m.vld <= 1'b0; m <= 8'hFF;
    m2 <= 8'hFF; m2.vld <= 1'b0;
    m3.d[3:0] <= 4'h0; m3 <= 8'hFF; m3.d[1] <= 1'b0;
    v[sel*4 +: 4] <= 4'h0; v <= 16'hFFFF;
    {v2[15:12], v2[3:0]} <= 8'h00; v2 <= 16'hFFFF;
    i.b.vld <= 1'b0; i.b <= 8'hFF;
    u.s.vld <= 1'b0; u.s <= 8'hFF;
    pk::pv.vld <= 1'b0; pk::pv <= 8'hFF;
    arr[1].vld <= 1'b0; arr[1] <= 8'hFF;
    c.p.vld <= 1'b0; c.p <= 8'hFF;
    C::sp.vld <= 1'b0; C::sp <= 8'hFF;
    tk();
    #1;
    $display("m=%h m2=%h m3=%h v=%h v2=%h i.b=%h u.s=%h pv=%h arr1=%h c.p=%h sp=%h m4=%h", m, m2, m3, v, v2, i.b, u.s, pk::pv, arr[1], c.p, C::sp, m4);
    drive(i);
    #1 $display("vif i.b=%h", i.b);
    // partial NBA with #0 / #1 intra delay then whole
    m.vld <= #1 1'b0; m <= 8'h11;
    #2 $display("delay m=%h", m);
    // whole then partial then later blocking
    m <= 8'h0F; m.vld <= 1'b1; m.d[0] = 1'b0;
    #1 $display("wpb m=%h", m);
    $finish;
  end
endmodule
"#
        ),
        [
            r#"m=ff m2=7f m3=fd v=ffff v2=ffff i.b=ff u.s=ff pv=ff arr1=ff c.p=ff sp=ff m4=ff"#,
            r#"vif i.b=c3"#,
            r#"delay m=11"#,
            r#"wpb m=8f"#,
        ]
    );
}

/// A partial update after a whole one: a member bit, an ascending member's
/// part- and bit-select, a packed-array member, an indexed part-select, a
/// concatenation, an unpacked-array element's member, ascending vectors and
/// a two-dimensional packed array; then partial updates followed by blocking
/// writes, and partial-whole-partial sequences.
#[test]
fn a_partial_update_after_a_whole_one_lands_on_its_bits() {
    assert_eq!(
        lines(
            r#"// whole-then-partial and partial-then-whole over many lvalue shapes
module t;
  typedef struct packed { logic vld; logic [0:7] asc; logic [7:0] d; logic [1:0][3:0] nib; } s_t;
  s_t a, b, c, e, f, g, h, k, arr[2], arr2[2];
  logic [15:0] v, w2; logic [0:15] va;
  logic [3:0][3:0] mda;
  int i = 2;
  initial begin
    a = '0; b = '0; c = '0; e = '0; f = '0; g = '0; h = '0; k = '0; arr[0] = '0; arr[1] = '0; arr2[0] = '0; arr2[1] = '0;
    v = 0; w2 = 0; va = 0; mda = 0;
    #1;
    // whole then partial: the partial must land on top of the whole
    a <= '1; a.d[3] <= 1'b0;
    b <= '1; b.asc[0:3] <= 4'h0;
    c <= '1; c.asc[i] <= 1'b0;
    e <= '1; e.nib[1] <= 4'h0;
    f <= '1; f.d[i+:2] <= 2'b00;
    g <= '1; {g.vld, g.d[0]} <= 2'b00;
    arr[1] <= '1; arr[1].d[7:4] <= 4'h0;
    v <= 16'hFFFF; v[i*4 +: 4] <= 4'h0;
    va <= 16'hFFFF; va[0:3] <= 4'h0; va[i +: 2] <= 2'b00;
    mda <= '1; mda[2][1:0] <= 2'b00; mda[1] <= 4'h0;
    #1;
    $display("a=%h b=%h c=%h e=%h f=%h g=%h arr1=%h v=%h va=%h mda=%h", a, b, c, e, f, g, arr[1], v, va, mda);
    // partial then blocking write to other bits, same step
    h.vld <= 1'b1; h.d = 8'h5A;
    k.asc[1] <= 1'b1; k.asc[7] = 1'b1;
    arr2[0].d[0] <= 1'b1; arr2[0].vld = 1'b1;
    w2[15:12] <= 4'hF; w2[0] = 1'b1;
    #1;
    $display("h=%h k=%h arr2_0=%h w2=%h", h, k, arr2[0], w2);
    // partial, whole, partial; and partial twice to one member
    a.d <= 8'h11; a <= '0; a.vld <= 1'b1; a.d[0] <= 1'b1;
    b.d <= 8'h22; b.d[7:4] <= 4'h3;
    #1;
    $display("a=%h b=%h", a, b);
    $finish;
  end
endmodule
"#
        ),
        [
            r#"a=1fff7ff b=10fffff c=1dfffff e=1ffff0f f=1fff3ff g=0fffeff arr1=1ff0fff v=f0ff va=0fff mda=fc0f"#,
            r#"h=1005a00 k=0410000 arr2_0=1000100 w2=f001"#,
            r#"a=1000100 b=10f32ff"#,
        ]
    );
}
