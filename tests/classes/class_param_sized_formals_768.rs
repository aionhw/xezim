//! IEEE 1800-2017 §8.25, §13.5, §18.4.2: in a parameterized class, a formal,
//! return or local typed `bit [W-1:0]`, or by a class typedef of it
//! (`typedef bit [W-1:0] vec_t;`), is as wide as the object's specialization
//! makes it, with the type's signedness, in every specialization and after
//! the method is compiled hot. Formals kept the actual's 32 bits, a typedef
//! formal read 1 bit, and a typedef local was 32 bits wide. Each element of
//! a `randc` array cycles through its values (#768).

use xezim::simulate;

fn tagged(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T "))
        .collect()
}

#[test]
fn widths_follow_the_specialization() {
    let out = tagged(
        r#"
class fc #(int W = 8);
  typedef bit [W-1:0] vec_t;
  function int f(bit [W-1:0] a); return a; endfunction
  task automatic t(bit [W-1:0] a, output int r); r = a; endtask
  static function int sf(bit [W-1:0] a); return a; endfunction
  function int df(bit [W-1:0] a = 32'hDEAD_BEEF); return a; endfunction
  function int fb(bit [W-1:0] a); return $bits(a); endfunction
  function int tf(vec_t a); return a; endfunction
  function vec_t tr(int a); return a; endfunction
  function int tl(); vec_t t; t = '1; return t; endfunction
endclass
class sc #(int W = 8);
  typedef bit signed [W-1:0] s_t;
  function int fs(s_t a); return a; endfunction
  function s_t rs(int a); return a; endfunction
  function int ls(); s_t t; t = '1; return t; endfunction
  function int sb(bit signed [W-1:0] a); return a; endfunction
endclass
module tb;
  initial begin
    fc #(8) f8 = new; fc #(16) f16 = new; fc fd = new; sc #(8) s = new;
    int r, bad8, bad16;
    f8.t(32'hDEAD_BEEF, r);
    $display("T f %0d %0d %0d %0d %0d", f8.f(32'hDEAD_BEEF), r, f8.sf(32'hDEAD_BEEF), f8.df(), f8.fb(5));
    $display("T ctx %0d spec16 %0d default %0d", f16.f(4'b1111 + 4'b0001), f16.f(32'hDEAD_BEEF), fd.f(32'hDEAD_BEEF));
    $display("T typedef %0d %0d %0d", f8.tf(32'hDEAD_BEEF), f8.tr(32'h1FF), f8.tl());
    $display("T signed %0d %0d %0d %0d", s.fs(32'h1FF), s.rs(32'h1FF), s.ls(), s.sb(32'h1FF));
    for (int k = 0; k < 3000; k++) begin
      if (f8.f(32'hDEAD_BEEF) != 239 || f8.tf(32'hDEAD_BEEF) != 239 || f8.tr(32'h1FF) != 255 || f8.tl() != 255) bad8++;
      if (f16.f(32'hDEAD_BEEF) != 48879 || f16.tf(32'hDEAD_BEEF) != 48879 || f16.tr(32'h1FFFF) != 65535 || f16.tl() != 65535) bad16++;
    end
    $display("T hot %0d %0d", bad8, bad16);
  end
endmodule
"#,
    );
    let want = [
        "T f 239 239 239 239 8",
        "T ctx 16 spec16 48879 default 239",
        "T typedef 239 255 255",
        "T signed -1 -1 -1 -1",
        "T hot 0 0",
    ];
    assert_eq!(out, want, "{out:#?}");
}

#[test]
fn each_element_of_a_randc_array_cycles() {
    let out = tagged(
        r#"
class rc; randc bit [3:0] arr[2]; randc bit [1:0] m[2][2]; endclass
module tb;
  initial begin
    rc o = new;
    int d0, d1, dm;
    bit h0[16], h1[16], hm[4];
    for (int k = 0; k < 16; k++) begin void'(o.randomize()); h0[o.arr[0]] = 1; h1[o.arr[1]] = 1; end
    for (int k = 0; k < 4; k++) begin void'(o.randomize()); hm[o.m[1][0]] = 1; end
    foreach (h0[i]) begin d0 += h0[i]; d1 += h1[i]; end
    foreach (hm[i]) dm += hm[i];
    $display("T randc %0d %0d %0d", d0, d1, dm);
  end
endmodule
"#,
    );
    assert_eq!(out, ["T randc 16 16 4"], "{out:#?}");
}
