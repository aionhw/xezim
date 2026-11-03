//! §13.4.1, §7.2.2: a function's implicit result variable is a variable of
//! the return type, so the members of an unpacked-struct result start at
//! their declared defaults. A function that writes only some members through
//! its name (`f.c = 1;`) returned 0 or x in the others.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn result_members_start_at_their_defaults() {
    let out = lines(
        r#"
typedef struct { int a = 3; logic [7:0] b = 8'h5a; real r = 2.5; logic [3:0] c; } s_t;
typedef struct { s_t in; int k = 7; } o_t;
module top;
  function automatic s_t f(); f.c = 4'h1; endfunction
  function automatic o_t g(); g.in.a = 9; endfunction
  class C; function s_t m(); m.a = 4; endfunction endclass
  s_t s; o_t o; C c;
  initial begin
    s = f(); $display("R %0d %h %g %h", s.a, s.b, s.r, s.c);
    o = g(); $display("R %0d %h %0d", o.in.a, o.in.b, o.k);
    c = new; s = c.m(); $display("R %0d %h %g %h", s.a, s.b, s.r, s.c);
  end
endmodule
"#,
    );
    assert_eq!(out, ["R 3 5a 2.5 1", "R 9 5a 7", "R 4 5a 2.5 x"]);
}
