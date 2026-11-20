//! IEEE 1800-2023 §10.10: the forms around #1322 in which an unpacked array
//! concatenation item is itself an array — a slice of a fixed array or of one
//! of its rows (§7.4.5), a concatenation returned from a function (§13.4.1)
//! or initializing a class queue property (§8.7), a fixed-array declaration
//! initializer (§6.8) and a continuous assignment (§10.3). Each array item
//! contributes all of its elements, left to right.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn slices_of_fixed_arrays_and_rows() {
    // §7.4.5: `[b +: w]` and `[b -: w]` select in the array's direction, and
    // an element outside the declared range reads the element default.
    check(
        r#"
module top;
  int a1[4] = '{6, 7, 8, 9};
  int w[4] = '{1, 2, 3, 4};
  int dw[3:0] = '{4, 3, 2, 1};
  int s3[2][3] = '{'{1, 2, 3}, '{6, 7, 8}};
  logic [7:0] b4[4] = '{8'h11, 8'h22, 8'h33, 8'h44};
  string sw[3] = '{"a", "b", "c"};
  int dst[3], q[$], i, d4[4];
  logic [7:0] bd[3];
  string ss[3];
  class C;
    int m[4];
    function new(); m = '{10, 20, 30, 40}; endfunction
  endclass
  C c;
  initial begin
    dst = {a1[0+:2], 5};      $display("T|up %p", dst);
    dst = {w[3-:2], 9};       $display("T|down %p", dst);
    dst = {w[1:2], 9};        $display("T|range %p", dst);
    dst = {dw[2:1], 9};       $display("T|descending %p", dst);
    dst = {dw[1+:2], 9};      $display("T|descending up %p", dst);
    dst = {s3[1][0:1], 5};    $display("T|row %p", dst);
    dst = {s3[1][2-:2], 5};   $display("T|row down %p", dst);
    i = 1;
    dst = {s3[i][i+:2], 5};   $display("T|row variable %p", dst);
    i = 3;
    dst = {a1[i+:2], 5};      $display("T|past end %p", dst);
    bd = {b4[i+:2], 8'h05};   $display("T|four-state past end %h %h %h", bd[0], bd[1], bd[2]);
    q = {a1[1+:2], w[0:1]};   $display("T|queue %p", q);
    ss = {sw[1:2], "z"};      $display("T|string %p", ss);
    c = new;
    dst = {c.m[1+:2], 5};     $display("T|class %p", dst);
    d4 = {a1[0+:2], w[2:3]};  $display("T|two slices %p", d4);
    dst <= {w[3-:2], 7};
    #1 $display("T|nonblocking %p", dst);
  end
endmodule
"#,
        &[
            "T|up '{6, 7, 5}",
            "T|down '{3, 4, 9}",
            "T|range '{2, 3, 9}",
            "T|descending '{3, 2, 9}",
            "T|descending up '{3, 2, 9}",
            "T|row '{6, 7, 5}",
            "T|row down '{7, 8, 5}",
            "T|row variable '{7, 8, 5}",
            "T|past end '{9, 0, 5}",
            "T|four-state past end 44 xx 05",
            "T|queue '{7, 8, 1, 2}",
            r#"T|string '{"b", "c", "z"}"#,
            "T|class '{20, 30, 5}",
            "T|two slices '{6, 7, 3, 4}",
            "T|nonblocking '{3, 4, 7}",
        ],
    );
}

#[test]
fn returned_concatenation_and_queue_property_initializer() {
    // §13.4.1: the returned concatenation is a value of the return type;
    // §8.7: so is a class queue property's initializer.
    check(
        r#"
module top;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int whole[2] = '{6, 7};
  typedef int arr3_t[3];
  typedef int q_t[$];
  int dst[3], q[$];
  function automatic arr3_t rf();  return {src[1], src[0][1]}; endfunction
  function automatic arr3_t rw();  return {whole, 9}; endfunction
  function automatic q_t rq();     return {src[1], src[0][1]}; endfunction
  function automatic q_t rqw();    return {whole, whole[0], 8}; endfunction
  function automatic q_t rsl();    int l[4] = '{1, 2, 3, 4}; return {l[1+:2], 9}; endfunction
  class K;
    int q[$] = {whole, 5};
    int e[$] = {1, 2};
  endclass
  K k;
  initial begin
    dst = rf();  $display("T|row %p", dst);
    dst = rw();  $display("T|whole %p", dst);
    q = rq();    $display("T|queue row %0d %p", q.size(), q);
    q = rqw();   $display("T|queue whole %0d %p", q.size(), q);
    q = rsl();   $display("T|local slice %p", q);
    $display("T|direct %p", rw());
    k = new;
    $display("T|property %p %p", k.q, k.e);
  end
endmodule
"#,
        &[
            "T|row '{6, 7, 5}",
            "T|whole '{6, 7, 9}",
            "T|queue row 3 '{6, 7, 5}",
            "T|queue whole 4 '{6, 7, 6, 8}",
            "T|local slice '{2, 3, 9}",
            "T|direct '{6, 7, 9}",
            "T|property '{6, 7, 5} '{1, 2}",
        ],
    );
}

#[test]
fn fixed_array_declaration_initializers() {
    // §6.8: the initializer is an assignment of the whole array, its first
    // element at the left bound (§7.6).
    check(
        r#"
package pk;
  int pw[2] = '{6, 7};
  int pd[3] = {pw, 5};
  int pdn[5:3] = {6, 7, 5};
endpackage
module child;
  int csrc[2][2] = '{'{4, 5}, '{6, 7}};
  int cd[3] = {csrc[1], csrc[0][1]};
endmodule
module top;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int whole[2] = '{6, 7};
  int d1[3] = {6, 7, 5};
  int d2[3] = {whole, 5};
  int d3[3] = {src[1], src[0][1]};
  int dn[5:3] = {whole, 5};
  int off[1:3] = {src[1], 5};
  logic [7:0] lb[3] = {8'h1, 8'h2, 8'h3};
  string sd[2] = {"x", "y"};
  child u();
  for (genvar g = 0; g < 1; g++) begin : gb
    int gd[3] = {whole, 4};
  end
  initial begin
    $display("T|d1 %p d2 %p d3 %p", d1, d2, d3);
    $display("T|dn %0d %0d %0d off %0d %0d %0d", dn[5], dn[4], dn[3], off[1], off[2], off[3]);
    $display("T|lb %0d %0d %0d sd %p", lb[0], lb[1], lb[2], sd);
    $display("T|pkg %p %0d %0d %0d", pk::pd, pk::pdn[5], pk::pdn[4], pk::pdn[3]);
    $display("T|child %p gen %p", u.cd, gb[0].gd);
  end
endmodule
"#,
        &[
            "T|d1 '{6, 7, 5} d2 '{6, 7, 5} d3 '{6, 7, 5}",
            "T|dn 6 7 5 off 6 7 5",
            r#"T|lb 1 2 3 sd '{"x", "y"}"#,
            "T|pkg '{6, 7, 5} 6 7 5",
            "T|child '{6, 7, 5} gen '{6, 7, 4}",
        ],
    );
}

#[test]
fn continuous_assignment_of_a_concatenation() {
    // §10.3: each element follows its source.
    check(
        r#"
module top;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int whole[2] = '{6, 7};
  int c1[3], c2[3], c3[3], c4[5:3], c5[3];
  int s5;
  assign c1 = {6, 7, 5};
  assign c2 = {whole, 5};
  assign c3 = {src[1], src[0][1]};
  assign c4 = {whole, 5};
  assign c5 = {whole[1], s5, 5};
  initial begin
    s5 = 3;
    #1 $display("T|c1 %p c2 %p c3 %p c4 %p c5 %p", c1, c2, c3, c4, c5);
    whole[0] = 9; src[1][1] = 8; s5 = 4;
    #1 $display("T|c1 %p c2 %p c3 %p c4 %p c5 %p", c1, c2, c3, c4, c5);
  end
endmodule
"#,
        &[
            "T|c1 '{6, 7, 5} c2 '{6, 7, 5} c3 '{6, 7, 5} c4 '{6, 7, 5} c5 '{7, 3, 5}",
            "T|c1 '{6, 7, 5} c2 '{9, 7, 5} c3 '{6, 8, 5} c4 '{9, 7, 5} c5 '{7, 4, 5}",
        ],
    );
}
