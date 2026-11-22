//! §11.4.2, §13.4: a member read through an element select whose index has a
//! side effect (`objs[i++].v`, `qs[i++].a`, `objs[next()].v`) evaluates the
//! index once, so `i++` advances `i` by one and the read returns the member of
//! the element that index selects. The lookups that find the element and size
//! the read had each evaluated the index again: `i++` ran up to thirteen
//! times, a function call five times. Expected values were checked on
//! Verilator 5.046.

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
fn issue_repro_reads_the_selected_element_once() {
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
    i = 0; b = objs[i++].v == 1; $display("R objs[i++].v == 1: %0d i=%0d", b, i);
    i = 0; r = objs[i++].v;      $display("R r = objs[i++].v:  %0d i=%0d", r, i);
    i = 0; b = qs[i++].a == 1;   $display("R qs[i++].a == 1:   %0d i=%0d", b, i);
    i = 0; r = qs[i++].a;        $display("R r = qs[i++].a:    %0d i=%0d", r, i);
    calls = 0; r = objs[next()].v; $display("R r = objs[next()].v: %0d calls=%0d", r, calls);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R objs[i++].v == 1: 1 i=1",
            "R r = objs[i++].v:  1 i=1",
            "R qs[i++].a == 1:   1 i=1",
            "R r = qs[i++].a:    1 i=1",
            "R r = objs[next()].v: 2 calls=1",
        ]
    );
}

const DESIGN: &str = r#"
module tb;
  class In;
    int w;
    function new(int x); w = x; endfunction
  endclass
  typedef struct packed { logic [11:0] a; logic [3:0] b; } s_t;
  typedef struct { int a; byte b; } u_t;
  class C;
    int v;
    byte b;
    s_t s;
    logic [11:0] arr[3];
    In in;
    function new(int x);
      v = x; b = 8'(x * 16); s = {12'(x * 3), 4'(x)}; in = new(x * 100);
      foreach (arr[k]) arr[k] = 12'(x * 10 + k);
    endfunction
    function int get(); return v; endfunction
  endclass
  class K;
    C items[$];
    int n;
    function int pick(); return items[n++].v; endfunction
    function int pick_s(); return items[n++].s.a; endfunction
  endclass
  C objs[$];
  C fa[4];
  C dy[];
  C aa[int];
  C c;
  K k;
  s_t qs[$];
  s_t fs[4];
  s_t m2[3][2];
  u_t qu[$];
  int i, j, r, calls;
  bit b;
  logic [15:0] w16;
  function int next();
    calls++;
    return calls;
  endfunction
  function int id(int x); return x; endfunction
  initial begin
    k = new;
    dy = new[4];
    for (int x = 0; x < 6; x++) begin
      c = new(x + 1); objs.push_back(c); k.items.push_back(c);
      qs.push_back({12'(x + 1), 4'(x)});
      qu.push_back('{x + 1, 8'(x + 10)});
      if (x < 4) begin
        c = new(x + 1); fa[x] = c;
        c = new(x + 1); dy[x] = c;
        fs[x] = {12'(x + 1), 4'(x)};
      end
      c = new(x + 1); aa[x * 2] = c;
    end
    foreach (m2[p, q]) m2[p][q] = {12'(p * 10 + q), 4'(q)};
    BODY
    $finish;
  end
endmodule
"#;

fn run_body(body: &str) -> Vec<String> {
    run(&DESIGN.replace("BODY", body))
}

#[test]
fn the_index_runs_once_in_every_context() {
    let lines = run_body(
        r#"
    i = 0; b = objs[i++].v == 1;            $display("R eq %0d i=%0d", b, i);
    i = 0; r = objs[i++].v;                 $display("R as %0d i=%0d", r, i);
    i = 0; r = objs[i++].v + objs[i++].v;   $display("R sum %0d i=%0d", r, i);
    i = 0; r = objs[i++].v * 10 + objs[i++].v; $display("R two %0d i=%0d", r, i);
    i = 0; $display("R disp %0d", objs[i++].v); $display("R disp i=%0d", i);
    i = 0; if (objs[i++].v == 1) $display("R if-taken i=%0d", i); else $display("R if-else i=%0d", i);
    i = 0; while (objs[i++].v < 3) ; $display("R while i=%0d", i);
    i = 0; r = id(objs[i++].v);             $display("R arg %0d i=%0d", r, i);
    i = 0; r <= objs[i++].v; #1             $display("R nba %0d i=%0d", r, i);
    calls = 0; r = objs[next()].v;          $display("R call %0d calls=%0d", r, calls);
"#,
    );
    assert_eq!(
        lines,
        [
            "R eq 1 i=1",
            "R as 1 i=1",
            "R sum 3 i=2",
            "R two 12 i=2",
            "R disp 1",
            "R disp i=1",
            "R if-taken i=1",
            "R while i=3",
            "R arg 1 i=1",
            "R nba 1 i=1",
            "R call 2 calls=1",
        ]
    );
}

#[test]
fn collections_of_handles_and_structs_and_index_forms() {
    let lines = run_body(
        r#"
    i = 1; r = fa[i++].v;                   $display("R fixed %0d i=%0d", r, i);
    i = 1; r = dy[i++].v;                   $display("R dyn %0d i=%0d", r, i);
    i = 2; r = aa[i++].v;                   $display("R assoc %0d i=%0d", r, i);
    i = 0; r = qs[i++].a;                   $display("R qs %0d i=%0d", r, i);
    i = 1; r = fs[i++].b;                   $display("R fs %0d i=%0d", r, i);
    i = 0; r = qu[i++].b;                   $display("R qu %0d i=%0d", r, i);
    i = 0; j = 0; r = m2[i++][j++].a;       $display("R m2 %0d i=%0d j=%0d", r, i, j);
    i = 0; r = objs[++i].v;                 $display("R pre %0d i=%0d", r, i);
    i = 3; r = objs[i--].v;                 $display("R dec %0d i=%0d", r, i);
    i = 0; r = objs[(i += 2)].v;            $display("R asg %0d i=%0d", r, i);
    i = 0; r = objs[$ - i++].v;             $display("R dollar %0d i=%0d", r, i);
"#,
    );
    assert_eq!(
        lines,
        [
            "R fixed 2 i=2",
            "R dyn 2 i=2",
            "R assoc 2 i=3",
            "R qs 1 i=1",
            "R fs 1 i=2",
            "R qu 10 i=1",
            "R m2 0 i=1 j=1",
            "R pre 2 i=1",
            "R dec 4 i=2",
            "R asg 3 i=2",
            "R dollar 6 i=1",
        ]
    );
}

#[test]
fn members_below_the_element_and_class_property_queues() {
    let lines = run_body(
        r#"
    i = 0; r = objs[i++].in.w;              $display("R nested %0d i=%0d", r, i);
    i = 0; r = objs[i++].s.a;               $display("R deep %0d i=%0d", r, i);
    i = 0; j = 0; r = objs[i++].arr[j++];   $display("R arr %0d i=%0d j=%0d", r, i, j);
    i = 0; r = qs[i++].a[2];                $display("R bit %0d i=%0d", r, i);
    i = 0; r = objs[i++].get();             $display("R method %0d i=%0d", r, i);
    i = 0; r = k.items[i++].v;              $display("R prop %0d i=%0d", r, i);
    k.n = 1; r = k.pick();                  $display("R meth %0d n=%0d", r, k.n);
    k.n = 1; r = k.pick_s();                $display("R meth_s %0d n=%0d", r, k.n);
"#,
    );
    assert_eq!(
        lines,
        [
            "R nested 100 i=1",
            "R deep 3 i=1",
            "R arr 10 i=1 j=1",
            "R bit 0 i=1",
            "R method 1 i=1",
            "R prop 1 i=1",
            "R meth 2 n=2",
            "R meth_s 6 n=2",
        ]
    );
}

#[test]
fn the_read_keeps_the_members_width() {
    let lines = run_body(
        r#"
    i = 0; r = objs[i++].b + 8'hff;         $display("R wb %0d i=%0d", r, i);
    i = 0; w16 = {qs[i++].a, 4'h5};         $display("R wcat %h i=%0d", w16, i);
    i = 0; $display("R wdisp %h %h", objs[i++].b, qs[i++].a); $display("R wdisp i=%0d", i);
"#,
    );
    assert_eq!(
        lines,
        [
            "R wb 271 i=1",
            "R wcat 0015 i=1",
            "R wdisp 10 002",
            "R wdisp i=2"
        ]
    );
}

/// The probes that size an operand of a comparison, a shift or an arithmetic
/// operator size a copy of the read whose side-effect selects name an
/// element without being evaluated, past null handles at the front of a
/// queue (1100 of them in `many`), an associative array and a class-property
/// queue, and keep the member's own width (8 and 12 bits below, where a sum
/// wraps), for an associative array of packed structs too.
#[test]
fn sizing_the_read_runs_no_side_effect() {
    let lines = run(r#"module tb;
  typedef struct packed { logic [11:0] a; logic [3:0] b; } s_t;
  class In;
    int w;
    function new(int x); w = x; endfunction
  endclass
  class C;
    int v;
    byte b;
    logic [11:0] arr[3];
    In in;
    function new(int x);
      v = x; b = 8'(x * 16); in = new(x * 100);
      foreach (arr[k]) arr[k] = 12'(x * 10 + k);
    endfunction
  endclass
  class H;
    C items[$];
  endclass
  C objs[$];
  C nul[$];
  C many[$];
  H hh;
  C an[int];
  C as_[string];
  C c;
  s_t qs[$];
  s_t sa[int];
  int i, r, calls;
  bit b;
  function int next();
    calls++;
    return calls;
  endfunction
  function string key();
    calls++;
    return "k";
  endfunction
  initial begin
    for (int x = 0; x < 8; x++) begin
      c = new(x + 1); objs.push_back(c);
      qs.push_back({12'(x + 1), 4'(x)});
    end
    nul.push_back(null); c = new(1); nul.push_back(c);
    an[5] = null; c = new(1); an[7] = c;
    sa[3] = {12'd1, 4'd0}; sa[9] = {12'd2, 4'd0};
    repeat (1100) many.push_back(null); c = new(1); many.push_back(c);
    hh = new; hh.items.push_back(null); c = new(1); hh.items.push_back(c);
    c = new(1); as_["k"] = c; as_["a"] = null;
    calls = 0; r = objs[next()].v * 10 + objs[next()].v; $display("R c-arith %0d calls=%0d", r, calls);
    calls = 0; b = objs[next()].v == 2;                  $display("R c-eq %0d calls=%0d", b, calls);
    calls = 0; r = (calls < 100) ? objs[next()].v : 0;   $display("R c-cond %0d calls=%0d", r, calls);
    calls = 0; b = qs[next()].a == 2;                    $display("R c-qs %0d calls=%0d", b, calls);
    calls = 0; r = objs[next()].b << 1;                  $display("R c-shl %0d calls=%0d", r, calls);
    calls = 0; b = objs[next()].arr[1] == 21;            $display("R c-arr %0d calls=%0d", b, calls);
    calls = 0; r = (objs[next()].v > 1) && (objs[next()].v > 2); $display("R c-and %0d calls=%0d", r, calls);
    i = 0; b = objs[i++].arr[1] == 11;                   $display("R arr-eq %0d i=%0d", b, i);
    i = 0; r = objs[i++].b << 1;                         $display("R shl %0d i=%0d", r, i);
    i = 0; b = objs[i++].in.w == 100;                    $display("R in-eq %0d i=%0d", b, i);
    i = 0; b = qs[i++].a[2] == 0;                        $display("R bit-eq %0d i=%0d", b, i);
    i = 1; b = nul[i++].b + 8'hff == 8'h0f;              $display("R null-w %0d i=%0d", b, i);
    i = 7; b = an[i++].b + 8'hff == 8'h0f;               $display("R assoc-w %0d i=%0d", b, i);
    i = 1100; $display("R far %0d", many[i++].b + 8'hff); $display("R far i=%0d", i);
    i = 1; b = hh.items[i++].b + 8'hff == 8'h0f;         $display("R prop-w %0d i=%0d", b, i);
    calls = 0; b = as_[key()].b + 8'hff == 8'h0f;        $display("R str-w %0d calls=%0d", b, calls);
    i = 0; b = objs[i++].b + 8'hff == 8'h0f;             $display("R w8 %0d i=%0d", b, i);
    i = 0; b = qs[i++].a + 12'hfff == 12'h000;           $display("R w12 %0d i=%0d", b, i);
    i = 9; b = sa[i++].a + 12'hffe == 12'h000;           $display("R sa-w %0d i=%0d", b, i);
    $finish;
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R c-arith 23 calls=2",
            "R c-eq 1 calls=1",
            "R c-cond 2 calls=1",
            "R c-qs 1 calls=1",
            "R c-shl 64 calls=1",
            "R c-arr 1 calls=1",
            "R c-and 1 calls=2",
            "R arr-eq 1 i=1",
            "R shl 32 i=1",
            "R in-eq 1 i=1",
            "R bit-eq 1 i=1",
            "R null-w 1 i=2",
            "R assoc-w 1 i=8",
            "R far 15",
            "R far i=1101",
            "R prop-w 1 i=2",
            "R str-w 1 calls=1",
            "R w8 1 i=1",
            "R w12 1 i=1",
            "R sa-w 1 i=10",
        ]
    );
}
