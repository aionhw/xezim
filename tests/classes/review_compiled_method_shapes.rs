//! Review regressions of #800 and #804: a compiled method must give the
//! interpreter's result. A nested-block handle that shadows a formal or a
//! top-level local keeps its own class in a statement fallback; `C h =
//! new()` constructs the declared class when the name shadows a member or a
//! sibling block's local (§8.8); an element store into a member collection
//! of a specialized class takes that specialization's width (§8.25); an
//! `inout` copy-back takes the actual's width and sign (§13.5.2); `$cast`
//! under `&&` / `?:` runs only when selected (§11.4.7, §11.4.11). Expected
//! T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class B; int v = 1; function int nv(); return 1; endfunction endclass
class E extends B; int v = 5; function int nv(); return 2; endfunction endclass
class K; int x = 3; endclass
class L; int x = 7; endclass
class P #(int W = 8);
  bit [W-1:0] d[];
  function new(); d = new[1]; endfunction
  function void st(int a, int b); d[0] = a * b; endfunction
endclass
class O;
  function void st(P#(16) c, int a, int b); c.d[0] = a * b; endfunction
endclass
class T;
  L h;
  E d;
  function int f(int i);
    B h = new;
    int r = h.nv();
    begin
      E h;
      h = new;
      r += $countones(i) + h.nv() * 10 + h.v * 100;
    end
    return r;
  endfunction
  function int g(B h, int i);
    int r = h.nv();
    begin
      E h;
      h = new;
      r += $countones(i) + h.nv() * 10 + h.v * 100;
    end
    return r;
  endfunction
  function int shadow_new(int i);
    K h = new();
    return h.x + i;
  endfunction
  function int sibling_new(int i);
    int s = i;
    begin K h = new(); s += h.x; end
    begin L h = new(); s += h.x; end
    return s;
  endfunction
  function void io(inout bit [7:0] a, input int v); a = a + v; endfunction
  function int wide(int i);
    bit [3:0] n = 4'd11;
    io(n, 20);
    return n;
  endfunction
  function void ios(inout byte a); a = a - 1; endfunction
  function longint narrow(int i);
    int x = 200;
    ios(x);
    return {x, 8'h00};
  endfunction
  function int cast_and(B b); return (b != null) && $cast(d, b); endfunction
  function int cast_cond(B b); return (b == null) ? 0 : $cast(d, b); endfunction
endclass
module top;
  initial begin
    T t = new; B b = new; E keep = new;
    P#(16) p = new; O o = new;
    int a, c, s1, s2, n, own, foreign, k1, k2;
    longint r;
    for (int i = 0; i < 1500; i++) begin
      a = t.f(i); c = t.g(b, i);
      s1 += t.shadow_new(i); s2 += t.sibling_new(i);
      p.st(300, 300); own = p.d[0];
      o.st(p, 300, 301); foreign = p.d[0];
      n = t.wide(i); r = t.narrow(i);
      t.d = keep; k1 = t.cast_and(null) + (t.d == null);
      t.d = keep; k2 = t.cast_cond(null) + (t.d == null);
    end
    $display("T|f=%0d g=%0d", a, c);
    $display("T|new=%0d sibling=%0d", s1, s2);
    $display("T|own=%h foreign=%h", own, foreign);
    $display("T|n=%0d r=%h", n, r);
    $display("T|cast=%0d %0d", k1, k2);
  end
endmodule
"#;

fn run(compile: &str) -> Vec<String> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_review_shapes_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_TIER")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "simulation failed:\n{text}");
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn compiled_methods_match_the_interpreter_on_shadowing_widths_and_casts() {
    for compile in ["0", "1"] {
        assert_eq!(
            run(compile),
            [
                "T|f=529 g=529",
                "T|new=1128750 sibling=1139250",
                "T|own=00005f90 foreign=000060bc",
                "T|n=15 r=000000ffffffc700",
                "T|cast=0 0",
            ],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
