//! IEEE 1800-2023 §13.4.1, §7.2: assigning a whole unpacked structure to a
//! function's return variable by name (`f = t;`, with `t` a local, a formal,
//! a static local, a design variable or a class property) returns that
//! structure, as `return t;` does. The return variable's members kept their
//! defaults, so the call returned zeros.

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn whole_struct_assigned_to_the_return_variable() {
    let out = lines(
        r#"
module top;
  typedef struct {int a; int b;} P;
  typedef struct {int x; P p; int y[2];} N;
  function automatic P byname_t(input P d[2]); P t; t = d[1]; byname_t = t; endfunction
  function automatic P byname_x(input P x); byname_x = x; endfunction
  function P st_t(input P x); P t; t = x; st_t = t; endfunction
  P g;
  function automatic P from_g(); from_g = g; endfunction
  function automatic N nest(input N n); N t; t = n; t.x++; nest = t; endfunction
  class K;
    P kp;
    function P getp(); getp = kp; endfunction
  endclass
  K k;
  P arr[2], r;
  N nn, rn;
  initial begin
    arr[1] = '{a:3, b:4};
    g = '{a:5, b:6};
    r = byname_t(arr); $display("T|byname_t %0d %0d", r.a, r.b);
    r = byname_x(arr[1]); $display("T|byname_x %0d %0d", r.a, r.b);
    r = st_t(arr[1]); $display("T|st_t %0d %0d", r.a, r.b);
    r = from_g(); $display("T|from_g %0d %0d", r.a, r.b);
    nn = '{x:1, p:'{a:2, b:3}, y:'{4, 5}};
    rn = nest(nn); $display("T|nest %p", rn);
    k = new; k.kp = '{a:7, b:8}; r = k.getp(); $display("T|getp %0d %0d", r.a, r.b);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|byname_t 3 4",
            "T|byname_x 3 4",
            "T|st_t 3 4",
            "T|from_g 5 6",
            "T|nest '{x:2, p:'{a:2, b:3}, y:'{4, 5}}",
            "T|getp 7 8",
        ]
    );
}
