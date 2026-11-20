//! §13.4.1, §13.5.1: a function that returns a whole unpacked-struct element
//! of an array argument (`return d[0];`) returns that element. The element
//! of an array formal has no container value: its members are separate
//! leaves, so the whole-element read returned zeros (#1310). Covered: fixed,
//! dynamic, queue, associative and `ref` array formals, a two-dimensional
//! formal, a struct member of an element, an element of a struct's array
//! member, a local array, a design array, an index evaluated once (`d[$]`,
//! `d[d.size() - 1]`, `d[i++]`), the return variable assigned by name, and
//! class methods. The expected values are those of the LRM, which the
//! reference simulator gives for every line. Run with methods compiled (the
//! default and on the first call) and interpreted.

use std::process::Command;

fn run(src: &str, env: &[(&str, &str)]) -> Vec<String> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_struct_elem_return_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "top",
        path.to_str().unwrap(),
        "--no-cache",
    ])
    .env_remove("XEZIM_METHOD_CACHE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

fn check(src: &str, expected: &[&str]) {
    for env in [
        &[][..],
        &[("XEZIM_METHOD_TIER", "0")][..],
        &[("XEZIM_COMPILE_METHODS", "0")][..],
    ] {
        assert_eq!(run(src, env), expected, "{env:?}");
    }
}

#[test]
fn element_of_an_array_formal() {
    let src = r#"
module top;
  typedef struct {int a; int b;} P;
  typedef struct {real m[2]; real b;} S;
  typedef struct {P in; logic [3:0] k; string s;} N;
  function automatic P first(input P d[2]); return d[0]; endfunction
  function automatic P second(input P d[2]); return d[1]; endfunction
  function automatic P first_copy(input P d[2]); P t; t = d[0]; return t; endfunction
  function automatic P nth(input P d[2], input int i); return d[i]; endfunction
  function automatic P byname(input P d[2]); byname = d[1]; endfunction
  function automatic S fs(input S d[1]); return d[0]; endfunction
  function automatic S fd(input S d[]); return d[0]; endfunction
  function automatic S fq(input S d[$]); return d[$]; endfunction
  function automatic S fr(ref S d[1]); return d[0]; endfunction
  function automatic N fn(input N d[2]); return d[1]; endfunction
  function automatic P f2(input P d[2][2]); return d[1][0]; endfunction
  function automatic P fin(input N d[2]); return d[1].in; endfunction
  function automatic P fcond(input P d[2], input bit c); return c ? d[0] : d[1]; endfunction
  function automatic P fpar(input P d[2]); return (d[1]); endfunction
  function automatic P fassoc(input P d[string]); return d["k"]; endfunction
  function static P fstat(input P d[2]); return d[1]; endfunction
  task automatic tout(input P d[2], output P o); o = d[1]; endtask
  P arr[2]; P r, c; S sa[1], sd[], sq[$], sr; N na[2], nr; P g2[2][2]; P as[string];
  initial begin
    arr[0] = '{a:1, b:2}; arr[1] = '{a:3, b:4};
    r = first(arr); c = first_copy(arr);
    $display("T| first %0d %0d copy %0d %0d", r.a, r.b, c.a, c.b);
    r = second(arr); $display("T| second %0d %0d", r.a, r.b);
    r = nth(arr, 1); $display("T| nth %0d %0d", r.a, r.b);
    r = byname(arr); $display("T| byname %0d %0d", r.a, r.b);
    sa[0] = '{m:'{1.5, 2.5}, b:3.5};
    sr = fs(sa); $display("T| fs %g %g %g", sr.m[0], sr.m[1], sr.b);
    sd = new[1]; sd[0] = '{m:'{4.5, 5.5}, b:6.5};
    sr = fd(sd); $display("T| fd %g %g %g", sr.m[0], sr.m[1], sr.b);
    sq.push_back('{m:'{7.5, 8.5}, b:9.5}); sq.push_back('{m:'{0.5, 0.25}, b:0.125});
    sr = fq(sq); $display("T| fq %g %g %g", sr.m[0], sr.m[1], sr.b);
    sa[0].b = 10.5;
    sr = fr(sa); $display("T| fr %g %g %g", sr.m[0], sr.m[1], sr.b);
    na[1] = '{in:'{5, 6}, k:4'hx, s:"hi"};
    nr = fn(na); $display("T| fn %0d %0d %b %s", nr.in.a, nr.in.b, nr.k, nr.s);
    g2[1][0] = '{7, 8};
    r = f2(g2); $display("T| f2 %0d %0d", r.a, r.b);
    r = fin(na); $display("T| fin %0d %0d", r.a, r.b);
    r = fcond(arr, 0); $display("T| fcond %0d %0d", r.a, r.b);
    r = fpar(arr); $display("T| fpar %0d %0d", r.a, r.b);
    as["k"] = '{9, 10};
    r = fassoc(as); $display("T| fassoc %0d %0d", r.a, r.b);
    r = fstat(arr); $display("T| fstat %0d %0d", r.a, r.b);
    tout(arr, r); $display("T| tout %0d %0d", r.a, r.b);
    $display("T| expr %0d %0d", first(arr).b, nth(arr, 1).a + second(arr).b);
    $finish;
  end
endmodule
"#;
    check(
        src,
        &[
            "T| first 1 2 copy 1 2",
            "T| second 3 4",
            "T| nth 3 4",
            "T| byname 3 4",
            "T| fs 1.5 2.5 3.5",
            "T| fd 4.5 5.5 6.5",
            "T| fq 0.5 0.25 0.125",
            "T| fr 1.5 2.5 10.5",
            "T| fn 5 6 xxxx hi",
            "T| f2 7 8",
            "T| fin 5 6",
            "T| fcond 3 4",
            "T| fpar 3 4",
            "T| fassoc 9 10",
            "T| fstat 3 4",
            "T| tout 3 4",
            "T| expr 2 7",
        ],
    );
}

#[test]
fn element_forms_and_held_indices() {
    let src = r#"
module top;
  typedef struct {int a; int b;} P;
  typedef struct {P in[2]; real r;} W;
  P garr[2];
  function automatic P byname(input P d[2]); byname = d[1]; endfunction
  function automatic P bymem(input P d[2]); bymem.a = d[1].a; bymem.b = d[0].b; endfunction
  function automatic P lastq(input P d[$]); return d[$]; endfunction
  function automatic P lastd(input P d[]); return d[d.size()-1]; endfunction
  function automatic P inc(input P d[2], ref int i); return d[i++]; endfunction
  function automatic P loc(); P l[2]; l[0] = '{1, 2}; l[1] = '{11, 12}; return l[1]; endfunction
  function automatic P glob(input int i); return garr[i]; endfunction
  function automatic P pass(input P d[2]); return pick(d[1]); endfunction
  function automatic P pick(input P x); return x; endfunction
  function automatic P win(input W w); return w.in[1]; endfunction
  function automatic P wd(input W d[2]); return d[1].in[0]; endfunction
  function automatic int show(input P x); return x.a * 100 + x.b; endfunction
  function automatic P neg(input P d[2], input int i); if (i < 0) return d[0]; else begin P z; z = d[i]; z.a = -z.a; return z; end endfunction
  function automatic P oob(input P d[2]); return d[5]; endfunction
  P arr[2], r, q[$], dd[]; W w, wa[2]; int k;
  wire [31:0] ca = pick(arr[1]).a + glob(1).b;
  initial begin
    arr[0] = '{a:1, b:2}; arr[1] = '{a:3, b:4}; garr[0] = '{5, 6}; garr[1] = '{7, 8};
    r = byname(arr); $display("T| byname %0d %0d", r.a, r.b);
    r = bymem(arr); $display("T| bymem %0d %0d", r.a, r.b);
    q.push_back('{21, 22}); q.push_back('{23, 24});
    r = lastq(q); $display("T| lastq %0d %0d", r.a, r.b);
    dd = new[3]; dd[2] = '{31, 32};
    r = lastd(dd); $display("T| lastd %0d %0d", r.a, r.b);
    k = 0; r = inc(arr, k); $display("T| inc %0d %0d k=%0d", r.a, r.b, k);
    r = loc(); $display("T| loc %0d %0d", r.a, r.b);
    r = glob(1); $display("T| glob %0d %0d", r.a, r.b);
    r = pass(arr); $display("T| pass %0d %0d", r.a, r.b);
    w.in[1] = '{41, 42}; r = win(w); $display("T| win %0d %0d", r.a, r.b);
    wa[1].in[0] = '{51, 52}; r = wd(wa); $display("T| wd %0d %0d", r.a, r.b);
    $display("T| show %0d %0d", show(glob(0)), show(pass(arr)));
    r = neg(arr, 1); $display("T| neg %0d %0d", r.a, r.b);
    r = neg(arr, -1); $display("T| neg %0d %0d", r.a, r.b);
    r = oob(arr); $display("T| oob %0d %0d", r.a, r.b);
    #1 $display("T| ca %0d", ca);
    $finish;
  end
endmodule
"#;
    check(
        src,
        &[
            "T| byname 3 4",
            "T| bymem 3 2",
            "T| lastq 23 24",
            "T| lastd 31 32",
            "T| inc 1 2 k=1",
            "T| loc 11 12",
            "T| glob 7 8",
            "T| pass 3 4",
            "T| win 41 42",
            "T| wd 51 52",
            "T| show 506 304",
            "T| neg -3 4",
            "T| neg 1 2",
            "T| oob 0 0",
            "T| ca 11",
        ],
    );
}

#[test]
fn resolver_and_class_methods() {
    let src = r#"
`timescale 1ns/1ps
typedef struct {real m[2]; real b;} S;
typedef struct {real v; int n;} T;
function automatic S fres(input S d[]); return d[0]; endfunction
function automatic T tres(input T d[]); int k = 0; foreach (d[i]) if (d[i].v > d[k].v) k = i; return d[k]; endfunction
nettype S NS with fres;
nettype T NT with tres;
class C;
  typedef struct {int a; int b;} P;
  P arr[2];
  function P get(input P d[2]); return d[1]; endfunction
  function P own(); return arr[1]; endfunction
  static function P sget(input P d[2]); return d[0]; endfunction
endclass
module top;
  NS n; NT t;
  assign n = '{m:'{1.5, 2.5}, b:3.5};
  assign t = '{1.25, 3};
  assign t = '{2.5, 7};
  C c = new; C::P r;
  initial begin
    #1 $display("T| n %g %g %g", n.m[0], n.m[1], n.b);
    $display("T| t %g %0d", t.v, t.n);
    c.arr[0] = '{1, 2}; c.arr[1] = '{3, 4};
    r = c.get(c.arr); $display("T| get %0d %0d", r.a, r.b);
    r = c.own(); $display("T| own %0d %0d", r.a, r.b);
    r = C::sget(c.arr); $display("T| sget %0d %0d", r.a, r.b);
    $finish;
  end
endmodule
"#;
    check(
        src,
        &[
            "T| n 1.5 2.5 3.5",
            "T| t 2.5 7",
            "T| get 3 4",
            "T| own 3 4",
            "T| sget 1 2",
        ],
    );
}
