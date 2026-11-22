//! §23.6: `$root.top` names the top-level instance, so a `$root` name that
//! selects a member after an element select reads the same object the
//! relative name does: a class property through a queue or array element
//! (`$root.tb.objs[1].v`, §8.4, §7.10), a field of a packed-struct element
//! (`$root.tb.qs[i].a`, §7.2.1) or of an unpacked-struct element, with a
//! constant, variable or `$` index, in the top or in an instance below it,
//! and a member of a variable in a generate block. The read keeps its
//! member's width in a concatenation and sizes a self-determined expression
//! (§11.6.1). Expected values were checked on Verilator 5.046.

fn run(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn root_member_of_a_queue_element_reads_like_the_relative_name() {
    let lines = run(r#"
module tb;
  class C;
    int v;
    function new(int x);
      v = x;
    endfunction
  endclass
  typedef struct packed { byte a; byte b; } s_t;
  C objs[$];
  C c;
  s_t qs[$];
  int r1, r2, r3, r4;
  int d[$];
  initial begin
    c = new(5);
    objs.push_back(c);
    c = new(7);
    objs.push_back(c);
    qs.push_back(16'h0102);
    qs.push_back(16'h0304);
    r1 = $root.tb.objs[1].v;
    r2 = tb.objs[1].v;
    r3 = $root.tb.qs[1].a;
    r4 = tb.qs[1].a;
    $display("R objs[1].v: root=%0d rel=%0d plain=%0d", r1, r2, objs[1].v);
    $display("R qs[1].a: root=%0d rel=%0d plain=%0d", r3, r4, qs[1].a);
    d = {$root.tb.objs[1].v, $root.tb.qs[1].a};
    $display("R concat %p", d);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R objs[1].v: root=7 rel=7 plain=7",
            "R qs[1].a: root=3 rel=3 plain=3",
            "R concat '{7, 3}",
        ]
    );
}

#[test]
fn root_member_after_variable_last_and_array_selects() {
    let lines = run(r#"
module sub;
  class D; int w; function new(int x); w = x; endfunction endclass
  D ds[$];
  D da[2];
  initial begin
    begin D t_; t_ = new(21); ds.push_back(t_); end
    begin D t_; t_ = new(22); ds.push_back(t_); end
    da[0] = new(31);
    da[1] = new(32);
  end
endmodule
module tb;
  class C;
    int v;
    function new(int x);
      v = x;
    endfunction
  endclass
  typedef struct packed { byte a; byte b; } s_t;
  typedef struct { byte a; int b; } u_t;
  C objs[$];
  C arr[2];
  C c;
  s_t qs[$];
  s_t sa[2];
  u_t qu[$];
  u_t uv;
  int i;
  sub u();
  initial begin
    c = new(5);
    objs.push_back(c);
    c = new(7);
    objs.push_back(c);
    arr[0] = new(8);
    arr[1] = new(9);
    qs.push_back(16'h0102);
    qs.push_back(16'h0304);
    sa[1] = 16'h1122;
    uv.a = 8'h41; uv.b = 42;
    qu.push_back(uv);
    uv.a = 8'h43; uv.b = 44;
    qu.push_back(uv);
    i = 1;
    #1;
    $display("R A %0d %0d %0d", $root.tb.objs[i].v, $root.tb.objs[0].v, $root.tb.arr[1].v);
    $display("R B %0d %0d %h %h", $root.tb.qs[i].b, $root.tb.qs[0].b, $root.tb.sa[1].a, $root.tb.sa[i].b);
    $display("R C %0d %0d %0d", $root.tb.qu[1].a, $root.tb.qu[1].b, $root.tb.qu[i].b);
    $display("R D %0d %0d %0d", $root.tb.u.ds[1].w, $root.tb.u.da[1].w, tb.u.ds[1].w);
    $display("R E %0d %0d", $root.tb.objs[$].v, $root.tb.qs[$].a);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R A 7 5 9",
            "R B 4 2 11 22",
            "R C 67 44 44",
            "R D 22 32 22",
            "R E 7 3",
        ]
    );
}

#[test]
fn root_member_keeps_its_width_in_expressions() {
    let lines = run(r#"
module tb;
  class C;
    byte v;
    function new(byte x);
      v = x;
    endfunction
  endclass
  typedef struct packed { byte a; byte b; } s_t;
  C objs[$];
  C c;
  s_t qs[$];
  s_t sa[2];
  logic [23:0] w;
  int i;
  initial begin
    c = new(8'h12);
    objs.push_back(c);
    c = new(8'h34);
    objs.push_back(c);
    qs.push_back(16'h0102);
    qs.push_back(16'h0304);
    sa[1] = 16'h5566;
    i = 1;
    w = {$root.tb.qs[1].a, $root.tb.objs[1].v, $root.tb.sa[i].b};
    $display("R w=%h bits=%0d %0d", w, $bits($root.tb.qs[1].a), $bits($root.tb.sa[1].b));
    $display("R cmp %0d %0d", $root.tb.qs[1].a == 8'h03, $root.tb.objs[i].v + 1);
  end
endmodule
"#);
    assert_eq!(lines, ["R w=033466 bits=8 8", "R cmp 1 53"]);
}

/// §11.6.1: in a self-determined expression the member's own width sizes the
/// other operands, so `(8'h80 << 1)` keeps its carry out of bit 7.
#[test]
fn root_member_widens_the_other_operands() {
    let lines = run(r#"
module tb;
  class C;
    int v;
    function new(int x);
      v = x;
    endfunction
  endclass
  typedef struct packed { logic [11:0] a; logic [3:0] b; } s_t;
  typedef struct { logic [11:0] a; int b; } u_t;
  C objs[$];
  C c;
  s_t qs[$];
  u_t qu[$];
  u_t uv;
  int i;
  logic [31:0] w;
  initial begin
    c = new(1);
    objs.push_back(c);
    qs.push_back({12'h001, 4'h0});
    uv.a = 12'h001; uv.b = 1;
    qu.push_back(uv);
    i = 0;
    $display("R a %0d %0d", $root.tb.objs[0].v + (8'h80 << 1'b1), $root.tb.objs[i].v + (8'h80 << 1'b1));
    $display("R b %0d %0d", $root.tb.qs[0].a + (8'h80 << 1'b1), $root.tb.qu[i].a + (8'h80 << 1'b1));
    $display("R c %0d", ($root.tb.objs[0].v + (8'h80 << 1'b1)) == 257);
    w = {$root.tb.objs[0].v + (8'h80 << 1'b1)};
    $display("R d %0d %0d", w, $bits($root.tb.objs[0].v + 8'h1));
    $display("R e %0d", $root.tb.qs[0].a + 8'hff + 8'h01);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R a 257 257",
            "R b 257 257",
            "R c 1",
            "R d 257 32",
            "R e 257",
        ]
    );
}

/// §11.4.2, §13.4.1: an element select with a side effect is evaluated once
/// per read of the name, also while the read's width is worked out.
#[test]
fn root_member_evaluates_a_side_effect_select_once() {
    let lines = run(r#"
module tb;
  class C;
    int v;
    function new(int x);
      v = x;
    endfunction
  endclass
  typedef struct packed { logic [11:0] a; logic [3:0] b; } s_t;
  C objs[$];
  C c;
  s_t qs[$];
  int i, r, calls;
  bit b;
  function int next();
    calls++;
    return calls;
  endfunction
  initial begin
    for (int k = 0; k < 6; k++) begin
      c = new(k + 1);
      objs.push_back(c);
      qs.push_back({12'(k + 1), 4'h0});
    end
    i = 0; b = $root.tb.objs[i++].v == 1; $display("R a %0d i=%0d", b, i);
    i = 0; r = $root.tb.objs[i++].v + 8'h1; $display("R b %0d i=%0d", r, i);
    i = 0; $display("R c %0d", $root.tb.objs[i++].v + (8'h80 << 1'b1)); $display("R c i=%0d", i);
    i = 0; b = $root.tb.qs[i++].a == 1; $display("R d %0d i=%0d", b, i);
    i = 0; $display("R e %0d", $root.tb.qs[i++].a + (8'h80 << 1'b1)); $display("R e i=%0d", i);
    calls = 0; r = $root.tb.objs[next()].v; $display("R f %0d calls=%0d", r, calls);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R a 1 i=1",
            "R b 2 i=1",
            "R c 257",
            "R c i=1",
            "R d 1 i=1",
            "R e 257",
            "R e i=1",
            "R f 2 calls=1",
        ]
    );
}

/// §11.6.1, §8.4, §8.25: with a side-effect select, the member's width comes
/// from its declared type, or, for a property of a parameterized class, from
/// a live element: never from a null one.
#[test]
fn root_member_with_a_side_effect_select_is_sized_by_its_declared_type() {
    let lines = run(r#"
module tb;
  class C;
    byte v;
    function new(byte x);
      v = x;
    endfunction
  endclass
  class K;
    bit [63:0] v;
  endclass
  class P #(int W = 8);
    logic [W-1:0] v;
    K child;
    function new(int x);
      v = x;
    endfunction
  endclass
  typedef struct packed { logic [7:0] a; logic [3:0] b; } s_t;
  C objs[$];
  C c;
  P pobjs[$];
  P p;
  P #(.W(12)) wobjs[$];
  P #(.W(12)) w;
  s_t qs[$];
  int calls;
  bit b;
  function int next();
    calls++;
    return calls;
  endfunction
  initial begin
    objs.push_back(null);
    c = new(16);
    objs.push_back(c);
    pobjs.push_back(null);
    p = new(16);
    pobjs.push_back(p);
    wobjs.push_back(null);
    w = new(16);
    wobjs.push_back(w);
    qs.push_back({8'h00, 4'h0});
    qs.push_back({8'h10, 4'h0});
    calls = 0;
    b = $root.tb.objs[next()].v === (4'h8 << 1'b1);
    $display("R a %0d calls=%0d", b, calls);
    calls = 0;
    b = $root.tb.qs[next()].a === (4'h8 << 1'b1);
    $display("R b %0d calls=%0d", b, calls);
    calls = 0;
    b = $root.tb.pobjs[next()].v === (4'h8 << 1'b1);
    $display("R c %0d calls=%0d", b, calls);
    calls = 0;
    b = $root.tb.wobjs[next()].v === (4'h8 << 1'b1);
    $display("R d %0d calls=%0d", b, calls);
    // pobjs[1] holds a null child: sizing must not take its width from it.
    p = new(1);
    p.child = new;
    p.child.v = 64'h10_0000_0000;
    pobjs.push_back(p);
    for (int n = 0; n < 2; n++) begin
      calls = 1;
      b = $root.tb.pobjs[next()].child.v === (36'h8_0000_0000 << 1'b1);
      $display("R e %0d calls=%0d", b, calls);
    end
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R a 1 calls=1",
            "R b 1 calls=1",
            "R c 1 calls=1",
            "R d 1 calls=1",
            "R e 1 calls=2",
            "R e 1 calls=2",
        ]
    );
}

#[test]
fn root_names_through_generate_blocks_and_instance_arrays() {
    let lines = run(r#"
module leaf #(parameter int K = 0);
  int x = K;
  int mem[3];
  initial mem[2] = K + 1;
endmodule
module tb;
  class C; int v; function new(int x); v = x; endfunction endclass
  typedef struct packed { byte a; byte b; } s_t;
  for (genvar g = 0; g < 2; g++) begin : gl
    int x = 10 + g;
    int mem[3];
    s_t s;
    C obj;
    leaf #(.K(100 + g)) l();
    initial begin
      mem[1] = 20 + g;
      s = 16'hab00 + g;
      obj = new(40 + g);
    end
  end
  leaf #(.K(7)) la[2] ();
  initial begin
    #1;
    $display("R G %0d %0d %0d", $root.tb.gl[1].x, $root.tb.gl[0].l.x, $root.tb.la[1].x);
    $display("R M %0d %0d %0d", $root.tb.gl[1].mem[1], $root.tb.gl[0].l.mem[2], $root.tb.la[1].mem[2]);
    $display("R S %0d %0d", $root.tb.gl[1].s.b, $root.tb.gl[1].obj.v);
  end
endmodule
"#);
    assert_eq!(lines, ["R G 11 100 7", "R M 21 101 8", "R S 1 41"]);
}
