//! §7.10: `$` in a queue select is the last index of that queue. A select
//! holding `$` is evaluated whole and a write through one is made whole, so
//! reads, conditions and writes through element, bit, part, slice and member
//! selects, in a module block or a method, and an index that casts `$` or
//! passes it to a call, all see the queue's own bound.

use xezim::simulate;

#[test]
fn last_index_selects_in_a_compiled_block() {
    let src = r#"
module child;
  int q[$] = '{4, 5, 6};
endmodule
module top;
  typedef struct packed { logic [7:0] f, g; } pair_t;
  int q[$] = '{10, 20, 30};
  int e[$];
  int aq[2][$];
  logic [7:0] bq[$] = '{8'h00, 8'h04};
  pair_t sq[$] = '{16'h0102, 16'h0304};
  int r_last, r_prev, r_sum, r_e, r_u, r_g, r_aq;
  logic [7:0] r_sqg;
  bit r_bq2, hit_if, hit_case;
  logic clk = 0;
  child u();
  if (1) begin : g
    int gq[$] = '{40, 50};
  end
  initial begin
    aq[1].push_back(60);
    aq[1].push_back(70);
  end
  always @(posedge clk) begin
    r_last = q[$];
    r_prev = q[$-1];
    r_sum = 0;
    for (int i = 0; i < 3; i++) r_sum += q[$-i];
    r_e = e[$];
    r_u = u.q[$];
    r_g = g.gq[$];
    r_aq = aq[1][$];
    r_sqg = sq[$].g;
    r_bq2 = bq[$][2];
    hit_if = 0;
    if (q[$] == 30) hit_if = 1;
    case (q[$])
      30: hit_case = 1;
      default: hit_case = 0;
    endcase
    bq[$][3] = 1'b1;
    bq[$-1][7:4] = 4'hf;
    sq[$].g = 8'h55;
    sq[$-1].f <= 8'h66;
  end
  initial begin
    #1 clk = 1;
    #1 $display("R|%0d %0d %0d %0d %0d %0d %0d %h %0d %0d %0d|%h %h|%h %h",
                r_last, r_prev, r_sum, r_e, r_u, r_g, r_aq, r_sqg, r_bq2, hit_if, hit_case,
                bq[0], bq[1], sq[0], sq[1]);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert!(
        sim.output
            .iter()
            .any(|o| o.message == "R|30 20 60 0 6 50 70 04 1 1 1|f0 0c|6602 0355"),
        "{:?}",
        sim.output
    );
}

#[test]
fn last_index_in_a_method_and_through_casts() {
    let src = r#"
module top;
  class C;
    int q[$] = '{1, 2, 3};
    function int last_two(int t);
      this.q[$-t] = 9;
      return this.q[$] + q[$-t];
    endfunction
  endclass
  int q[$] = '{10, 20, 30};
  C c;
  int r;
  initial begin
    c = new;
    r = c.last_two(1);
    q[int'($)] = 5;
    q[signed'($) - 1] = 6;
    q[$+1] = 7;
    $display("M|%0d %p", r, q);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "M|12 '{10, 6, 5, 7}"),
        "{:?}",
        sim.output
    );
}

#[test]
fn last_index_in_compiled_writes_calls_and_slices() {
    let src = r#"
module top;
  int q[$] = '{10, 20, 30};
  int s[$];
  int n = 0, r_call, r_in, r_nest, r_cast, r_sgn, r_size;
  logic clk = 0;
  function automatic int idf(int i);
    n++;
    return i;
  endfunction
  always @(posedge clk) begin
    q[idf($)] = 7;
    q[{1{$}} - 1] = 8;
    r_call = q[idf($)];
    r_in = q[$ inside {2} ? 0 : 1];
    r_nest = q[q[$] == 7 ? 0 : 1];
    r_cast = q[int'($)];
    r_sgn = q[$signed($) - 1];
    s = q[1:$];
    q[$+1] = 40;
    q[$]++;
    r_size = q.size();
  end
  initial begin
    #1 clk = 1;
    #1 $display("W|%p %p %0d %0d %0d %0d %0d %0d n=%0d",
                q, s, r_call, r_in, r_nest, r_cast, r_sgn, r_size, n);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert!(
        sim.output
            .iter()
            .any(|o| o.message == "W|'{10, 8, 7, 41} '{8, 7} 7 10 10 7 8 4 n=2"),
        "{:?}",
        sim.output
    );
}

#[test]
fn last_index_in_compiled_compound_and_nonblocking_writes() {
    // §10.4.2: the nonblocking target's index is taken when the statement
    // executes, before the `push_back`.
    let src = r#"
module top;
  int q[$] = '{1, 2, 3};
  int z[$] = '{5, 6, 7};
  logic clk = 0;
  always @(posedge clk) begin
    q[$] += 5;
    q[$-1] -= 1;
    z[$] <= 99;
    z.push_back(8);
  end
  initial begin
    #1 clk = 1;
    #1 $display("C|%p %p", q, z);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert!(
        sim.output
            .iter()
            .any(|o| o.message == "C|'{1, 1, 8} '{5, 6, 99, 8}"),
        "{:?}",
        sim.output
    );
}
