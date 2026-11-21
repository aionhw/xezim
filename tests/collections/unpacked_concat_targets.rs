//! IEEE 1800-2023 §10.10, §7.6: an unpacked array concatenation written to a
//! row of a multidimensional array or to a fixed-array structure member fills
//! the target from its left bound, and an array item contributes all of its
//! elements wherever it appears: as a `new[n](…)` initializer (§7.5.1), as the
//! chosen operand of a conditional (§11.4.11), under a cast to an unpacked
//! array type (§6.24.1) and as a structure member. Expected lines were checked
//! on the reference simulator.

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
fn rows_and_structure_members_as_targets() {
    // §7.6: the first element goes to the left bound of a descending member
    // of an array element or of an object.
    check(
        r#"
module top;
  typedef struct { int down[5:3]; int up[3]; } st_t;
  typedef struct { int a[3]; } in_t;
  typedef struct { in_t inner; logic [7:0] b[3]; } out_t;
  class C;
    int fa[5:3];
    int fu[3];
  endclass
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int dst2[2][3];
  int r3[2][2][3];
  st_t sar[2];
  out_t s;
  C h;
  int k;
  function automatic int f(); k++; return 1; endfunction
  initial begin
    dst2[1] = {src[1], src[0][1]};
    $display("T|row %0d %0d %0d", dst2[1][0], dst2[1][1], dst2[1][2]);
    sar[1].up = {6, 7, 5};
    $display("T|array member %0d %0d %0d", sar[1].up[0], sar[1].up[1], sar[1].up[2]);
    sar[1].up = {src[1], 5};
    $display("T|array member row %0d %0d %0d", sar[1].up[0], sar[1].up[1], sar[1].up[2]);
    sar[0].down = {6, 7, 5};
    $display("T|array descending member %0d %0d %0d", sar[0].down[5], sar[0].down[4], sar[0].down[3]);
    r3[1][0] = {src[1], 5};
    $display("T|three-dimensional row %0d %0d %0d", r3[1][0][0], r3[1][0][1], r3[1][0][2]);
    s.inner.a = {src[1], 5};
    $display("T|nested member %0d %0d %0d", s.inner.a[0], s.inner.a[1], s.inner.a[2]);
    s.b = {8'h1, 8'h2, 8'h3};
    $display("T|packed elements %h %h %h", s.b[0], s.b[1], s.b[2]);
    h = new;
    h.fa = {src[1], 5};
    $display("T|class descending %0d %0d %0d", h.fa[5], h.fa[4], h.fa[3]);
    h.fu = {6, 7, 5};
    $display("T|class %0d %0d %0d", h.fu[0], h.fu[1], h.fu[2]);
    dst2[f()] = {src[f()], 5};
    $display("T|selects %0d %0d %0d calls=%0d", dst2[1][0], dst2[1][1], dst2[1][2], k);
    dst2[0] = {dst2[1][2], dst2[1][0], dst2[1][1]};
    $display("T|from own rows %0d %0d %0d", dst2[0][0], dst2[0][1], dst2[0][2]);
  end
endmodule
"#,
        &[
            "T|row 6 7 5",
            "T|array member 6 7 5",
            "T|array member row 6 7 5",
            "T|array descending member 6 7 5",
            "T|three-dimensional row 6 7 5",
            "T|nested member 6 7 5",
            "T|packed elements 01 02 03",
            "T|class descending 6 7 5",
            "T|class 6 7 5",
            "T|selects 6 7 5 calls=2",
            "T|from own rows 5 6 7",
        ],
    );
}

#[test]
fn descending_array_formals_fill_from_the_left_bound() {
    // §7.6, §13.5: an array formal keeps the direction of its declaration.
    check(
        r#"
module top;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int o[5:3];
  int io[5:3] = '{1, 2, 3};
  task automatic tout(output int x[5:3]);
    x = {6, 7, 5};
  endtask
  task automatic trow(output int x[5:3]);
    x = {src[1], 5};
  endtask
  task automatic tin(input int x[5:3]);
    x = {6, 7, 5};
    $display("T|input %0d %0d %0d", x[5], x[4], x[3]);
  endtask
  task automatic tinout(inout int x[5:3]);
    x = {x[3], 8, 9};
  endtask
  function automatic int fsum(int x[2:0]);
    x = {1, 2, 3};
    return x[2] * 100 + x[1] * 10 + x[0];
  endfunction
  initial begin
    tout(o);     $display("T|output %0d %0d %0d", o[5], o[4], o[3]);
    trow(o);     $display("T|output row %0d %0d %0d", o[5], o[4], o[3]);
    tin(o);
    tinout(io);  $display("T|inout %0d %0d %0d", io[5], io[4], io[3]);
    $display("T|function %0d", fsum(o));
  end
endmodule
"#,
        &[
            "T|output 6 7 5",
            "T|output row 6 7 5",
            "T|input 6 7 5",
            "T|inout 3 8 9",
            "T|function 123",
        ],
    );
}

#[test]
fn concatenation_initializes_a_sized_new() {
    // §7.5.1: the initializer's elements fill the first elements; the rest
    // take the element default, and the initializer is read before resizing.
    check(
        r#"
module top;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int w[2] = '{6, 7};
  int d[];
  logic [7:0] b[];
  initial begin
    d = new[3]({6, 7, 5});              $display("T|scalars %p", d);
    d = new[3]({src[1], src[0][1]});    $display("T|row %p", d);
    d = new[4]({w, 5});                 $display("T|longer %p", d);
    d = new[2]({src[1], 9});            $display("T|shorter %p", d);
    b = new[3]({8'h1, 8'h2});           $display("T|four-state %h %h %h", b[0], b[1], b[2]);
    d = new[3]({d, 8});                 $display("T|self %p", d);
  end
endmodule
"#,
        &[
            "T|scalars '{6, 7, 5}",
            "T|row '{6, 7, 5}",
            "T|longer '{6, 7, 5, 0}",
            "T|shorter '{6, 7}",
            "T|four-state 01 02 xx",
            "T|self '{6, 7, 8}",
        ],
    );
}

#[test]
fn conditional_cast_and_member_array_items() {
    check(
        r#"
module top;
  typedef int row_t[2];
  typedef struct { int a[2]; } sa_t;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int w[2] = '{8, 9};
  int dst[3];
  int q[$];
  sa_t sa[2];
  sa_t one;
  bit c;
  initial begin
    c = 1;
    dst = {(c ? src[1] : src[0]), 5};   $display("T|conditional %p", dst);
    c = 0;
    dst = {c ? src[1] : w, 5};          $display("T|conditional else %p", dst);
    dst = {row_t'(src[1]), 5};          $display("T|cast %p", dst);
    dst = {row_t'(w), 5};               $display("T|cast whole %p", dst);
    sa[1].a = '{6, 7};
    q = {sa[1].a, 5};                   $display("T|member queue %p", q);
    dst = {sa[1].a, 5};                 $display("T|member %p", dst);
    one.a = '{1, 2};
    dst = {3, one.a};                   $display("T|scalar member %p", dst);
  end
endmodule
"#,
        &[
            "T|conditional '{6, 7, 5}",
            "T|conditional else '{8, 9, 5}",
            "T|cast '{6, 7, 5}",
            "T|cast whole '{8, 9, 5}",
            "T|member queue '{6, 7, 5}",
            "T|member '{6, 7, 5}",
            "T|scalar member '{3, 1, 2}",
        ],
    );
}
