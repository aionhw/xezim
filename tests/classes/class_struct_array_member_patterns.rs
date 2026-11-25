//! IEEE 1800-2023 §10.9.1, §10.9.2: a structure pattern assigned to a class
//! property whose structure has a fixed unpacked-array member gives that
//! member each element: a nested pattern element by element, and a value
//! reached through a type key or `default:` to every element.

#[test]
fn struct_pattern_reaches_array_members_of_a_class_property() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef struct {int a[2]; int b;} sa_t;
  typedef struct {int m[2][2]; byte c;} sm_t;
  typedef struct {int a[1:0]; string s[2];} ss_t;
  class K;
    sa_t sa;
    sm_t sm;
    ss_t ss;
    function void f1(); sa = '{int: 5, default: 6}; endfunction
    function void f2(); sm = '{default: 3, c: 1}; endfunction
    function void f3(); ss = '{a: '{1: 7, 0: 8}, s: '{default: "q"}}; endfunction
    function void f4(); sm = '{m: '{'{1, 2}, '{3, 4}}, c: 2}; endfunction
    function void f5(); sa = '{a: '{int: 8}, b: 1}; endfunction
    function void f6(); sa = '{int: 9}; endfunction
    function void f7(); sa = '{b: 2, default: 3}; endfunction
  endclass
  K k;
  sa_t m;
  initial begin
    k = new;
    k.f1(); $display("T|1 %p", k.sa);
    k.f2(); $display("T|2 %p", k.sm);
    k.f3(); $display("T|3 %p %0d", k.ss, k.ss.a[1]);
    k.f4(); $display("T|4 %p", k.sm);
    k.sa = '{default: 1}; $display("T|5 %p", k.sa);
    k.sa = '{a: '{9, 8}, default: 2}; $display("T|6 %p", k.sa);
    k.sa = '{int: 4, default: 6}; $display("T|7 %p", k.sa);
    k.f5(); $display("T|8 %p", k.sa);
    k.f6(); $display("T|9 %p", k.sa);
    k.f7(); $display("T|10 %p", k.sa);
    m = '{int: 4, default: 6}; $display("T|11 %p", m);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|1 '{a:'{5, 5}, b:5}",
            "T|2 '{m:'{'{3, 3}, '{3, 3}}, c:1}",
            r#"T|3 '{a:'{7, 8}, s:'{"q", "q"}} 7"#,
            "T|4 '{m:'{'{1, 2}, '{3, 4}}, c:2}",
            "T|5 '{a:'{1, 1}, b:1}",
            "T|6 '{a:'{9, 8}, b:2}",
            "T|7 '{a:'{4, 4}, b:4}",
            "T|8 '{a:'{8, 8}, b:1}",
            "T|9 '{a:'{9, 9}, b:9}",
            "T|10 '{a:'{3, 3}, b:2}",
            "T|11 '{a:'{4, 4}, b:4}",
        ]
    );
}

/// §7.6 and §10.9.1: a class-property array can be the value of a
/// multidimensional array's positional pattern item.
#[test]
fn array_pattern_copies_rows_from_class_struct_arrays() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef struct { int a; byte b; } S;
  class C;
    S rows[2];
    S dst[2][2];
    function void copy();
      dst = '{rows, this.rows};
      $display("class %0d %0d %0d %0d", dst[0][0].a, dst[0][1].b, dst[1][1].a, dst[1][0].b);
      dst[1][0].a = 9;
    endfunction
  endclass
  C c;
  S dst[2][2];
  initial begin
    c = new;
    c.rows[0] = '{1, 2};
    c.rows[1] = '{3, 4};
    dst = '{c.rows, c.rows};
    $display("module %0d %0d %0d %0d", dst[0][0].a, dst[0][1].b, dst[1][1].a, dst[1][0].b);
    c.copy();
    $display("updated %0d %0d", c.dst[1][0].a, c.rows[0].a);
    dst = '{c.dst[1], c.dst[0]};
    $display("rows %0d %0d %0d %0d", dst[0][0].a, dst[0][1].b, dst[1][0].a, dst[1][1].b);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(
        lines,
        ["module 1 4 3 2", "class 1 4 3 2", "updated 9 1", "rows 9 4 1 4"]
    );
}

/// §7.6 and §8.4: fixed unpacked struct arrays copy in declared order across
/// class properties, module arrays and multidimensional rows.
#[test]
fn direct_class_struct_array_copies_preserve_fields() {
    let sim = xezim::simulate(
        r#"
module top;
typedef struct { int a; byte b; } S;
class C;
  S src[3:2];
  S d[5:4];
  S rows[2][2];
  function void run();
    d = src;
    rows[1] = this.src;
    rows[0] = rows[1];
    $display("in %0d %0d %0d %0d",d[5].a,d[4].b,rows[0][0].a,rows[1][1].b);
  endfunction
endclass
C c; S md[2]; S src[2];
initial begin
  c=new;
  c.src[3]='{11,12}; c.src[2]='{13,14};
  src[0]='{21,22}; src[1]='{23,24};
  md=c.src;
  $display("out %0d %0d",md[0].a,md[1].b);
  c.d=src;
  $display("back %0d %0d",c.d[5].a,c.d[4].b);
  c.run();
end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["out 11 14", "back 21 24", "in 11 14 11 14"]);
}

/// §7.6: a direct array copy includes fixed-array fields of unpacked structs.
#[test]
fn direct_fixed_struct_array_copy_includes_nested_array_fields() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef struct { int val[2]; } S;
  class C;
    S src[1];
    S dst[3:3];
  endclass
  C c;
  S out_arr[7:7];
  initial begin
    c=new;
    c.src[0].val[0]=42;
    c.src[0].val[1]=17;
    out_arr=c.src;
    c.dst=out_arr;
    $display("fixed nested %0d %0d %0d %0d",out_arr[7].val[0],out_arr[7].val[1],c.dst[3].val[0],c.dst[3].val[1]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["fixed nested 42 17 42 17"]);
}

/// §7.4.6 and §7.6: an overlapping struct-array slice samples each
/// field before any destination element is overwritten.
#[test]
fn overlapping_class_struct_array_slices_copy_every_field() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef struct { int n; byte b; } S;
  class C; S a[3:0]; endclass
  C c;
  S out_arr[2];
  initial begin
    c=new;
    c.a[3]='{1,11}; c.a[2]='{2,22}; c.a[1]='{3,33}; c.a[0]='{4,44};
    out_arr=c.a[3:2];
    c.a[2:1]=c.a[3:2];
    $display("slice %0d %0d %0d %0d %0d %0d",out_arr[0].n,out_arr[1].b,c.a[2].n,c.a[2].b,c.a[1].n,c.a[1].b);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["slice 1 22 1 11 2 22"]);
}

/// §7.6 and §7.10: copying a queue to a descending fixed array and back
/// preserves element order and all unpacked struct fields.
#[test]
fn class_queue_and_descending_fixed_struct_array_copy_in_order() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef struct { int n; } S;
  class C; S a[5:4]; S q[$]; endclass
  C c;
  S q[$];
  initial begin
    c=new;
    c.q = '{'{7},'{9}};
    c.a=c.q;
    q=c.a;
    $display("rank %0d %0d %0d %0d %0d",c.a[5].n,c.a[4].n,q.size(),q[0].n,q[1].n);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(lines, ["rank 7 9 2 7 9"]);
}
