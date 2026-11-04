//! IEEE 1800 §§11.4.13, 11.8.2, 18.3–18.5 and 20.9: joint collection
//! constraints, unary set items, wide population counts and package enums.

use xezim::simulate;

fn check(src: &str) {
    let sim = simulate(src, 100).expect("simulate failed");
    let results: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|m| m.starts_with("RESULT"))
        .collect();
    assert_eq!(results, ["RESULT bad=0"], "{:#?}", sim.output);
}

#[test]
fn unary_inside_and_dist_items_850() {
    check(
        r#"
class item; rand bit en; endclass
class wide_item; rand bit [3:0] en; endclass
module top;
  initial begin
    item r = new();
    wide_item w = new();
    bit cur = 0;
    bit [3:0] v = 4'b1010;
    int bad = 0;
    repeat (20) begin
      if (!r.randomize() with { en inside {!cur}; }) bad++;
      else if (r.en == cur) bad++;
      cur = r.en;
    end
    repeat (20) begin
      if (!r.randomize() with { en dist { ~cur := 1 }; }) bad++;
      else if (r.en == cur) bad++;
      cur = r.en;
    end
    // §11.8.2: complement receives the four-bit comparison context.
    cur = 0;
    if (!w.randomize() with { en inside {~cur}; }) bad++;
    else if (w.en !== 4'd15) bad++;
    cur = 1;
    if (!w.randomize() with { en inside {~cur}; }) bad++;
    else if (w.en !== 4'd14) bad++;
    // Reduction operands keep their self-determined width.
    if (!r.randomize() with { en inside {&v}; }) bad++;
    else if (r.en !== 1'b0) bad++;
    if (!r.randomize() with { en inside {~&v}; }) bad++;
    else if (r.en !== 1'b1) bad++;
    if (!r.randomize() with { en inside {|v}; }) bad++;
    else if (r.en !== 1'b1) bad++;
    if (!r.randomize() with { en inside {~|v}; }) bad++;
    else if (r.en !== 1'b0) bad++;
    if (!r.randomize() with { en inside {^v}; }) bad++;
    else if (r.en !== 1'b0) bad++;
    if (!r.randomize() with { en inside {~^v}; }) bad++;
    else if (r.en !== 1'b1) bad++;
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

#[test]
fn whole_wide_countones_constraints_852() {
    const SRC: &str = r#"
class item;
  rand bit [@MSB@:0] m;
  constraint c { $countones(m) inside {[1:2]}; m[@MSB@] == 1'b1; }
endclass
module top;
  bit [@MSB@:0] m;
  initial begin
    item c = new();
    int bad = 0;
    int r, n;
    repeat (8) begin
      r = std::randomize(m) with {
        $countones(m) inside {[1:2]}; m[@MSB@] == 1'b1;
      };
      n = $countones(m);
      if (r != 1 || !(n inside {[1:2]}) || m[@MSB@] !== 1'b1) bad++;
      r = c.randomize();
      n = $countones(c.m);
      if (r != 1 || !(n inside {[1:2]}) || c.m[@MSB@] !== 1'b1) bad++;
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#;
    for width in [64, 65, 78, 128] {
        check(&SRC.replace("@MSB@", &(width - 1).to_string()));
    }
}

#[test]
fn impossible_wide_count_restores_value_852() {
    check(
        r#"
module top;
  bit [77:0] m = '1;
  bit [77:0] previous;
  initial begin
    int r, bad = 0;
    previous = m;
    r = std::randomize(m) with { $countones(m) == 79; };
    if (r != 0 || m !== previous) bad++;
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

#[test]
fn package_struct_enum_domains_862() {
    // A unique typedef keeps its bare declaration key; the qualified member
    // type still needs a domain. Package homonyms below exercise the other
    // path, where the prepass has already qualified the declarations.
    check(
        r#"
package p;
  typedef enum bit [2:0] { M_A = 1, M_B = 5 } mode_t;
  typedef struct { rand bit [3:0] x; rand mode_t mode; } cfg_t;
  class free_c; rand cfg_t s; endclass
endpackage
module top;
  import p::*;
  initial begin
    free_c f = new();
    int bad = 0;
    repeat (40) begin
      if (!f.randomize()) bad++;
      else if ((f.s.mode inside {M_A, M_B}) !== 1'b1) bad++;
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
    check(
        r#"
package p;
  typedef enum bit [2:0] { M_A = 1, M_B = 5 } mode_t;
  typedef struct { rand bit [3:0] x; rand mode_t mode; } cfg_t;
  class free_p; rand cfg_t s; endclass
endpackage
package q;
  typedef enum bit [2:0] { M_A = 0, M_B = 7 } mode_t;
  typedef struct { rand bit [3:0] x; rand mode_t mode; } cfg_t;
  class free_q; rand cfg_t s; endclass
endpackage
module top;
  initial begin
    p::free_p f = new();
    q::free_q g = new();
    int bad = 0;
    repeat (40) begin
      if (!f.randomize()) bad++;
      else if ((f.s.mode inside {p::M_A, p::M_B}) !== 1'b1) bad++;
      if (!g.randomize()) bad++;
      else if ((g.s.mode inside {q::M_A, q::M_B}) !== 1'b1) bad++;
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

#[test]
fn associative_enum_keys_join_array_sum_solve_848() {
    check(
        r#"
typedef enum bit [3:0] { E0=0, E3=3, E8=8, E12=12 } cause_t;
class cfg_c;
  rand int main_cnt;
  rand int sub_cnt[];
  rand bit flag[cause_t];
  constraint c {
    sub_cnt.size() == 5;
    main_cnt inside {[10:10000]};
    foreach (sub_cnt[i]) sub_cnt[i] inside {[10:10000]};
    main_cnt + sub_cnt.sum() == 10000;
    flag[E12] == 0;
  }
  function new();
    flag[E0]=1; flag[E3]=1; flag[E8]=1; flag[E12]=1;
  endfunction
endclass
module top;
  initial begin
    cfg_c c = new();
    int bad=0;
    repeat (5) begin
      if (!c.randomize()) bad++;
      else if (c.main_cnt+c.sub_cnt.sum()!=10000 || c.flag[E12]!==0) bad++;
      if (c.flag.num()!=4 || !c.flag.exists(E0) || !c.flag.exists(E3)
          || !c.flag.exists(E8) || !c.flag.exists(E12)) bad++;
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

#[test]
fn associative_sparse_signed_keys_are_preserved_848() {
    check(
        r#"
class cfg_c;
  rand int main_cnt;
  rand int a[int];
  constraint c {
    foreach (a[k]) a[k] inside {[100:200]};
    main_cnt inside {[100:10000]};
    main_cnt + a.sum() == 10000;
    a[-4] == a[99];
  }
  function new(); a[-4]=1; a[7]=2; a[99]=3; endfunction
endclass
module top;
  initial begin
    cfg_c c = new();
    int bad=0;
    repeat (5) begin
      if (!c.randomize()) bad++;
      else if (c.main_cnt+c.a.sum()!=10000 || c.a[-4]!=c.a[99]) bad++;
      if (c.a.num()!=3 || !c.a.exists(-4) || !c.a.exists(7)
          || !c.a.exists(99) || c.a.exists(0)) bad++;
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

#[test]
fn child_dynamic_array_joins_parent_sum_solve_860() {
    check(
        r#"
class sub_c;
  rand bit [7:0] r[];
  function new(); r=new[4]; endfunction
  constraint s_c { foreach (r[i]) r[i] > 8'd10; }
endclass
class cfg_c;
  rand sub_c sub;
  rand int main_cnt;
  rand int part[];
  int total=10000;
  constraint k {
    part.size()==5;
    main_cnt inside {[10:10000]};
    foreach (part[i]) part[i] inside {[10:10000]};
    main_cnt+part.sum()==total;
  }
  function new(); sub=new(); endfunction
endclass
module top;
  initial begin
    cfg_c c=new();
    int bad=0;
    repeat (5) begin
      if (!c.randomize()) bad++;
      else begin
        if (c.main_cnt+c.part.sum()!=10000 || c.sub.r.size()!=4) bad++;
        foreach (c.sub.r[i]) if (c.sub.r[i]<=10) bad++;
      end
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

#[test]
fn child_array_constraints_remain_joint_with_parent_860() {
    check(
        r#"
class sub_c;
  rand int r[];
  rand int value;
  function new(); r=new[2]; endfunction
  constraint s_c {
    r.size()==4;
    value inside {[50:100]};
    foreach(r[i]) r[i] inside {[11:30]};
    r.sum()==value;
  }
endclass
class cfg_c;
  rand sub_c sub;
  rand int main_cnt;
  rand int part[5];
  constraint k {
    main_cnt==sub.value;
    foreach(part[i]) part[i] inside {[10:10000]};
    main_cnt+part.sum()==10000;
  }
  function new(); sub=new(); endfunction
endclass
module top;
  initial begin
    cfg_c c=new();
    int bad=0;
    repeat(5) begin
      if(!c.randomize()) bad++;
      else begin
        if(c.main_cnt+c.part.sum()!=10000 || c.sub.r.size()!=4
           || c.main_cnt!=c.sub.value || c.sub.r.sum()!=c.sub.value) bad++;
        foreach(c.sub.r[i]) if(c.sub.r[i]<11 || c.sub.r[i]>30) bad++;
      end
    end
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}

/// §18.6: a failed enclosing solve restores child collections as well as
/// scalar values, including a dynamic array's original length.
#[test]
fn unsatisfiable_joint_child_array_solve_restores_children_860() {
    check(
        r#"
class sub_c;
  rand int fixed_values[2];
  rand int dynamic_values[];
  constraint c {
    dynamic_values.size()==4;
    foreach(dynamic_values[i]) dynamic_values[i] inside {[11:30]};
    foreach(fixed_values[i]) fixed_values[i] inside {[11:30]};
  }
  function new();
    fixed_values[0]=41; fixed_values[1]=42;
    dynamic_values=new[2];
    dynamic_values[0]=7; dynamic_values[1]=8;
  endfunction
endclass
class cfg_c;
  rand sub_c sub;
  rand int marker=123;
  constraint impossible { marker==321; sub.fixed_values.sum()==1; }
  function new(); sub=new(); endfunction
endclass
module top;
  initial begin
    cfg_c c=new();
    sub_c original=c.sub;
    int bad=0;
    if(c.randomize()!=0) bad++;
    if(c.marker!=123 || c.sub!=original) bad++;
    if(c.sub.fixed_values[0]!=41 || c.sub.fixed_values[1]!=42) bad++;
    if(c.sub.dynamic_values.size()!=2) bad++;
    else if(c.sub.dynamic_values[0]!=7 || c.sub.dynamic_values[1]!=8) bad++;
    $display("RESULT bad=%0d", bad);
  end
endmodule
"#,
    );
}
