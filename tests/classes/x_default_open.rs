//! Initial values and defaults next to the aggregate class property x
//! defaults: collection reads past the end or of a missing key (§7.8.6,
//! §7.10.1), associative-array `default:` initializers (§7.9.11), static and
//! returned unpacked structs (§6.8, §13.4.1), mixed 2-/4-state packed structs
//! (§7.2.1) and program instances (§24.3). Expected values come from the
//! reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn class_queue_read_past_the_end() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f; integer n;} rec_t;
class holder; logic [3:0] dq[$]; rec_t rq[$]; endclass
module top;
  holder h;
  initial begin
    h = new;
    $display("T|cls_q %b %0d %b %0d", h.dq[3], h.rq[1].n === 'x, h.rq[1].d, h.rq[1].f);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|cls_q xxxx 1 xxxx 0"]);
}

#[test]
fn class_assoc_default_initializer() {
    let src = r#"
class holder; logic [7:0] wa[int] = '{default: 8'h5A}; endclass
module top;
  holder h;
  initial begin
    h = new;
    $display("T|cls_def %h %h %0d", h.wa[9], h.wa[1], h.wa.num());
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|cls_def 5a 5a 0"]);
}

#[test]
fn class_two_dimensional_assoc_missing_element() {
    let src = r#"
class holder; logic [3:0] la2[int][int]; endclass
module top;
  holder h;
  initial begin
    h = new;
    $display("T|cls_2d %b %0d", h.la2[1][2], h.la2.num());
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|cls_2d xxxx 0"]);
}

#[test]
fn module_struct_collection_defaults() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f; integer n;} rec_t;
module top;
  rec_t mra[string]; rec_t mq[$];
  initial begin
    $display("T|mod %b %b %0d", mra["x"].d, mra["x"].f, mra["x"].n === 'x);
    $display("T|modq %b %b %0d", mq[0].d, mq[0].f, mq.size());
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|mod xxxx 0000 1", "T|modq xxxx 0000 0"]);
}

#[test]
fn percent_p_of_struct_property_with_arrays() {
    let src = r#"
class inner; int v = 7; endclass
typedef struct {logic [3:0] arr[3]; bit [3:0] barr[2]; inner h; integer n;} box_t;
class holder; box_t box; endclass
module top;
  holder h;
  initial begin
    h = new;
    $display("T|cls_p %p", h.box);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        ["T|cls_p '{arr:'{'hX, 'hX, 'hX}, barr:'{'h0, 'h0}, h:null, n:x}"]
    );
}

#[test]
fn static_unpacked_struct_property() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f;} rec_t;
class sc; static rec_t sr; endclass
module top;
  initial $display("T|static %b %b", sc::sr.d, sc::sr.f);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|static xxxx 0000"]);
}

#[test]
fn mixed_packed_struct_starts_unknown() {
    let src = r#"
typedef struct packed {bit [3:0] b; logic [3:0] l;} mix_t;
interface ifc; mix_t m; endinterface
module top;
  ifc u();
  mix_t mx;
  initial $display("T|mix %b %b %b %b", mx, mx.b, mx.l, u.m);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|mix xxxxxxxx 0000 xxxx xxxxxxxx"]);
}

#[test]
fn function_struct_return_default() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f;} rec_t;
module top;
  function automatic rec_t fr(); endfunction
  initial $display("T|fret %b %b", fr().d, fr().f);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|fret xxxx 0000"]);
}

#[test]
fn instantiated_program_initial_runs_reactive() {
    let src = r#"
program prg;
  logic [3:0] pv; int pi;
  initial $display("T|prog %b %0d", pv, pi);
endprogram
module top;
  prg pp();
  initial $display("T|mod");
endmodule
"#;
    assert_eq!(t_lines(src), ["T|mod", "T|prog xxxx 0"]);
}

#[test]
fn class_collection_reads_past_the_end_and_missing_keys() {
    let src = r#"
class inner; endclass
class holder;
  logic [3:0] dq[$]; int iq[$]; string sq[$]; inner hq[$]; real rq[$]; logic [7:0] dd[]; integer nq[$];
  logic [3:0] la2[int][int]; int ia2[int][string]; logic [3:0] la3[int][int][int];
  function void show(); $display("T|m %b %0d %b", dq[2], iq[1], la2[3][4]); endfunction
endclass
module top;
  holder h;
  logic [3:0] ml2[int][int];
  initial begin
    h = new;
    $display("T|a %b %0d \"%s\" %0d %0d", h.dq[3], h.iq[0], h.sq[0], h.hq[0] == null, h.nq[0] === 'x);
    $display("T|b %b %b %b", h.dd[1], h.dq[$], h.dq[-1]);
    $display("T|c %f", h.rq[0]);
    h.show();
    $display("T|d %0d %b %b", h.ia2[1]["k"], h.la3[1][2][3], ml2[1][2]);
    h.la2[1][2] = 4'h5; h.ia2[2]["z"] = 9;
    $display("T|e %h %b %0d %0d %0d", h.la2[1][2], h.la2[1][3], h.la2.num(), h.ia2[2]["z"], h.ia2[2]["y"]);
    h.dq.push_back(4'h3);
    $display("T|f %b %b %b", h.dq[0], h.dq[1], h.dq[$]);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|a xxxx 0 \"\" 1 1",
            "T|b xxxxxxxx xxxx xxxx",
            "T|c 0.000000",
            "T|m xxxx 0 xxxx",
            "T|d 0 xxxx xxxx",
            "T|e 5 xxxx 1 9 0",
            "T|f 0011 xxxx 0011",
        ]
    );
}

#[test]
fn class_struct_collection_missing_elements() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f; integer n;} rec_t;
class holder; logic [3:0] dq[$]; rec_t rq[$]; rec_t ra[int]; logic [3:0] la2[int][int]; bit [3:0] ba2[int][int]; rec_t rd[];
  function void show(); $display("T|in %b %b %b %b", rq[1].d, rq[1].f, ra[3].d, ra[3].f); endfunction
endclass
module top;
  holder h;
  rec_t r;
  initial begin
    h = new;
    $display("T|a %b %b", h.rq[1].d, h.rq[1].f);
    $display("T|b %b %b", h.ra[1].d, h.ra[1].f);
    $display("T|c %b %b", h.la2[1][2], h.ba2[1][2]);
    $display("T|d %b %b", h.rd[0].d, h.rd[0].f);
    h.show();
    r.d = 5; r.f = 3; r.n = 7;
    h.rq.push_back(r);
    $display("T|e %b %b %0d %b %b", h.rq[0].d, h.rq[0].f, h.rq[0].n, h.rq[1].d, h.rq[1].f);
    h.ra[2] = r;
    $display("T|f %b %b %b %b", h.ra[2].d, h.ra[2].f, h.ra[5].d, h.ra[5].f);
    $display("T|g %0d %0d", h.rq.size(), h.ra.num());
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|a xxxx 0000",
            "T|b xxxx 0000",
            "T|c xxxx 0000",
            "T|d xxxx 0000",
            "T|in xxxx 0000 xxxx 0000",
            "T|e 0101 0011 7 xxxx 0000",
            "T|f 0101 0011 xxxx 0000",
            "T|g 1 1",
        ]
    );
}

#[test]
fn class_assoc_pattern_initializers_and_assignments() {
    let src = r#"
class holder;
  logic [7:0] wa[int] = '{default: 8'h5A};
  logic [7:0] wb[int] = '{1: 8'h11, 3: 8'h33, default: 8'hEE};
  int ia[string] = '{"a": 1, default: -1};
  logic [7:0] wc[int];
  function void setc(); wc = '{2: 8'h22, default: 8'hCC}; endfunction
endclass
class kid extends holder; endclass
module top;
  holder h, h2; kid k;
  logic [7:0] ma[int] = '{default: 8'h77};
  initial begin
    h = new; h2 = new; k = new;
    $display("T|a %h %h %0d", h.wa[9], h.wa[1], h.wa.num());
    $display("T|b %h %h %h %0d", h.wb[1], h.wb[3], h.wb[4], h.wb.num());
    $display("T|c %0d %0d %0d", h.ia["a"], h.ia["zz"], h.ia.num());
    h.setc();
    $display("T|d %h %h %0d %h", h.wc[2], h.wc[5], h.wc.num(), h2.wc[5]);
    h.wa[9] = 8'h01; h.wa.delete(9);
    $display("T|e %h %h", h.wa[9], k.wb[7]);
    $display("T|f %h %0d", ma[3], ma.num());
    h2.wa = '{default: 8'h99};
    $display("T|g %h %h", h2.wa[0], h.wa[0]);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|a 5a 5a 0",
            "T|b 11 33 ee 2",
            "T|c 1 -1 1",
            "T|d 22 cc 1 xx",
            "T|e 5a ee",
            "T|f 77 0",
            "T|g 99 5a",
        ]
    );
}

#[test]
fn module_struct_collection_reads() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f; integer n; int i;} rec_t;
module top;
  rec_t mra[string]; rec_t mq[$]; rec_t md[]; rec_t mi[int];
  int k = 0;
  function automatic void f();
    rec_t lq[$];
    $display("T|fn %b %b %b %b", mq[0].d, mq[0].f, lq[1].d, lq[1].f);
  endfunction
  initial begin
    $display("T|mod %b %b %0d %0d", mra["x"].d, mra["x"].f, mra["x"].n === 'x, mra["x"].i);
    $display("T|modq %b %b %0d %0d", mq[0].d, mq[0].f, mq[k].n === 'x, mq.size());
    $display("T|modd %b %b %b", md[0].d, md[0].f, mi[3].f);
    f();
    mq.push_back('{1, 2, 3, 4});
    $display("T|after %b %b %b %b %0d", mq[0].d, mq[0].f, mq[1].d, mq[1].f, mq[$].i);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|mod xxxx 0000 1 0",
            "T|modq xxxx 0000 1 0",
            "T|modd xxxx 0000 0000",
            "T|fn xxxx 0000 xxxx 0000",
            "T|after 0001 0010 xxxx 0000 4",
        ]
    );
}

#[test]
fn static_unpacked_struct_is_shared() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f; integer n; int i;} rec_t;
class sc; static rec_t sr; static rec_t sw;
  static function void show(); $display("T|in %b %b %0d %0d", sr.d, sr.f, sr.n === 'x, sr.i); endfunction
  function void ishow(); $display("T|ii %b %b", sr.d, sr.f); endfunction
endclass
module top;
  sc o;
  initial begin
    $display("T|static %b %b %0d %0d", sc::sr.d, sc::sr.f, sc::sr.n === 'x, sc::sr.i);
    sc::show();
    o = new; o.ishow();
    $display("T|h %b %b", o.sr.d, o.sr.f);
    sc::sw.d = 4'h3;
    $display("T|w %b %b %b", sc::sw.d, sc::sw.f, o.sw.d);
    $display("T|p %p", sc::sr);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|static xxxx 0000 1 0",
            "T|in xxxx 0000 1 0",
            "T|ii xxxx 0000",
            "T|h xxxx 0000",
            "T|w 0011 0000 0011",
            "T|p '{d:'hX, f:'h0, n:x, i:0}",
        ]
    );
}

#[test]
fn mixed_packed_struct_member_reads_and_writes() {
    let src = r#"
typedef struct packed {bit [3:0] b; logic [3:0] l;} mix_t;
typedef struct packed {bit [3:0] b; int i;} two_t;
interface ifc; mix_t m; endinterface
class C; mix_t cm; endclass
module top;
  ifc u();
  mix_t mx, my, arr[2];
  two_t tw;
  C c;
  int r;
  initial begin
    $display("T|a %b %b %b %b", mx, mx.b, mx.l, u.m);
    my.b = 4'bx1z0; my.l = 4'bx1z0;
    $display("T|c %b %b %b", my, my.b, my.l);
    my = 8'bxxxx_1010;
    $display("T|d %b %b %b %0d", my, my.b, my.l, my.b == 0);
    r = mx.b + 1;
    $display("T|e %0d %b", r, u.m.b);
    c = new;
    $display("T|f %b %b", c.cm, c.cm.b);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|a xxxxxxxx 0000 xxxx xxxxxxxx",
            "T|c 0100x1z0 0100 x1z0",
            "T|d xxxx1010 0000 1010 1",
            "T|e 1 0000",
            "T|f xxxxxxxx 0000",
        ]
    );
}

#[test]
fn struct_returns_of_functions_and_methods() {
    let src = r#"
typedef struct {logic [3:0] d; bit [3:0] f; int i; integer n;} rec_t;
typedef struct packed {bit [3:0] b; logic [3:0] l;} mix_t;
class C;
  function rec_t m(); endfunction
  static function rec_t sm(); endfunction
endclass
module top;
  function automatic rec_t fr(); endfunction
  function rec_t fs(); endfunction
  function automatic mix_t fm(); endfunction
  function automatic rec_t fp(); fp.d = 4'h1; endfunction
  C c;
  rec_t r;
  initial begin
    c = new;
    $display("T|fret %b %b %0d %0d", fr().d, fr().f, fr().i, fr().n === 'x);
    $display("T|fs %b %b", fs().d, fs().f);
    $display("T|fm %b", fm());
    $display("T|fp %b %b", fp().d, fp().f);
    $display("T|m %b %b %b %b", c.m().d, c.m().f, C::sm().d, C::sm().f);
    r = fr();
    $display("T|r %b %b %0d", r.d, r.f, r.i);
    $display("T|p %p", fr());
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|fret xxxx 0000 0 1",
            "T|fs xxxx 0000",
            "T|fm xxxxxxxx",
            "T|fp 0001 0000",
            "T|m xxxx 0000 xxxx 0000",
            "T|r xxxx 0000 0",
            "T|p '{d:'hX, f:'h0, i:0, n:x}",
        ]
    );
}

/// §24.3: each initial block of each program instance is a process of its own,
/// scheduled in the reactive region and named after its instance (`%m`).
#[test]
fn program_instances_run_each_initial_block_reactive() {
    let src = r#"
program prg;
  logic [3:0] pv; int pi;
  initial begin $display("T|prog0 %m %b %0d %0t", pv, pi, $time); #3 $display("T|prog0b %m %0t", $time); end
  initial $display("T|prog1 %m %0t", $time);
endprogram
module top;
  prg pp();
  prg pq();
  initial $display("T|mod %0t", $time);
  initial begin #1 $display("T|mod1 %0t", $time); #9 $finish; end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|mod 0",
            "T|prog0 top.pp xxxx 0 0",
            "T|prog1 top.pp 0",
            "T|prog0 top.pq xxxx 0 0",
            "T|prog1 top.pq 0",
            "T|mod1 1",
            "T|prog0b top.pp 3",
            "T|prog0b top.pq 3",
        ]
    );
}
