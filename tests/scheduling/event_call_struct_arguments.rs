//! IEEE 1800-2023 §9.4.2: unpacked struct arguments are allowed in event
//! expressions that reduce to a singular value. Wake on a change of the result.

const AGGREGATE_ARGUMENTS: &str = r#"
`timescale 1ns/1ps
package event_types;
  typedef struct {real v;} S;
  nettype S N;
endpackage

module receiver(input var event_types::S port_value);
  int hits=0;
  function automatic real project(input event_types::S a); return a.v; endfunction
  initial begin #0.5; forever @(project(port_value)) hits++; end
endmodule

module top;
  import event_types::*;
  S drive, sv, arr[2];
  N n;
  int net_hits=0, var_hits=0, array_hits=0;
  function automatic real project(input S a); return a.v; endfunction
  assign n=drive;
  receiver r(sv);

  initial begin #0.5; forever @(project(n)) net_hits++; end
  initial begin #0.5; forever @(project(sv)) var_hits++; end
  initial begin #0.5; forever @(project(arr[1])) array_hits++; end

  initial begin
    drive='{1.25}; sv='{1.25}; arr[0]='{1.25}; arr[1]='{1.25};
    #1 drive.v=2.5; sv.v=2.5; arr[1].v=2.5;
    #1 arr[0].v=3.75; arr[1].v=2.5;
    #1 $display("counts=%0d,%0d,%0d,%0d",net_hits,var_hits,r.hits,array_hits);
    $finish;
  end
endmodule
"#;

const RESULT_GUARDS: &str = r#"
`timescale 1ns/1ps
module top;
  typedef struct {real v; int i; logic l;} S;
  typedef struct packed {int i; logic l;} P;
  S sv;
  P packed_value;
  int ignored=0, packed_hits=0, member_hits=0, qualified=0;
  bit enable=0;
  function automatic int struct_i(input S a); return a.i; endfunction
  function automatic int packed_i(input P a); return a.i; endfunction
  function automatic real twice(input real a); return 2.0*a; endfunction
  function automatic bit above_threshold(input S a); return a.v > 1; endfunction

  initial begin #0.5; forever @(struct_i(sv)) ignored++; end
  initial begin #0.5; forever @(packed_i(packed_value)) packed_hits++; end
  initial begin #0.5; forever @(twice(sv.v)) member_hits++; end
  always @(posedge above_threshold(sv) iff enable) qualified++;

  initial begin
    sv='{0.0,7,1'b0}; packed_value='{3,1'b0};
    #1 sv.v=2.5; packed_value.l=1;
    #1 sv.v=0.0; packed_value.i=4; enable=1;
    #1 sv.v=2.5; sv.l=1;
    #1 $display("counts=%0d,%0d,%0d,%0d",ignored,packed_hits,member_hits,qualified);
    $finish;
  end
endmodule
"#;

const SCOPED_ALWAYS: &str = r#"
`timescale 1ns/1ps
package event_types;
  typedef struct {real v;} S;
  typedef struct {S inner; bit tag;} W;
endpackage

module receiver(input var event_types::S port_value);
  event_types::W local_value;
  int local_hits=0, port_hits=0, wrapped_local_hits=0;
  function automatic real project(input event_types::S a); return a.v; endfunction
  always @(project(local_value.inner)) if ($time > 0) local_hits++;
  always @(project(local_value.inner) > 2.0) if ($time > 0) wrapped_local_hits++;
  always @(project(port_value)) if ($time > 0) port_hits++;

  initial begin
    local_value='{'{1.25},0};
    #2 local_value.inner.v=2.5;
    #1 local_value.tag=1;
  end
endmodule

module top;
  event_types::S sv;
  receiver r(sv);
  int top_hits=0, wrapped_hits=0;
  function automatic real project(input event_types::S a); return a.v; endfunction
  always @(project(sv)) if ($time > 0) top_hits++;
  always @(project(sv) > 2.0) if ($time > 0) wrapped_hits++;

  initial begin
    sv='{1.25};
    #1 sv.v=2.5;
    #3 $display("counts=%0d,%0d,%0d,%0d,%0d",
                r.local_hits,r.port_hits,top_hits,wrapped_hits,r.wrapped_local_hits);
    $finish;
  end
endmodule
"#;

const WRAPPED_CALLS: &str = r#"
`timescale 1ns/1ps
module top;
  typedef struct {real v; int unused_member;} S;
  S sv;
  bit en=1;
  int sum_hits=0, compare_hits=0, unary_hits=0, mux_hits=0, concat_hits=0;
  function automatic real project(input S a); return a.v; endfunction
  function automatic bit above(input S a); return a.v > 2.0; endfunction

  initial begin #0.5; forever @(project(sv) + 1.0) sum_hits++; end
  initial begin #0.5; forever @(project(sv) > 2.0) compare_hits++; end
  initial begin #0.5; forever @(-project(sv)) unary_hits++; end
  initial begin #0.5; forever @(en ? project(sv) : 0.0) mux_hits++; end
  initial begin #0.5; forever @({above(sv)}) concat_hits++; end

  initial begin
    sv='{1.25,0};
    #1 sv.v=2.5;
    #1 sv.v=3.75;
    #1 sv.unused_member=1;
    #1 $display("counts=%0d,%0d,%0d,%0d,%0d",sum_hits,compare_hits,unary_hits,mux_hits,concat_hits);
    $finish;
  end
endmodule
"#;

const FIXED_ARRAY_ARGUMENTS: &str = r#"
`timescale 1ns/1ps
package event_indices;
  localparam int IDX=1;
endpackage

module top;
  parameter int PARAM_IDX=1;
  localparam int LOCAL_IDX=1;
  localparam bit [1:0] WIDTH_TWO=2'd1;
  typedef struct {real v;} S;
  S arr[2], grid[2][2], signed_range[-1:2];
  int param_hits=0, local_hits=0, pkg_hits=0, signed_hits=0, unsigned_hits=0, expr_hits=0;
  int grid_hits=0, whole_hits=0;
  int div_hits=0, mod_hits=0, shift_hits=0, cond_hits=0, concat_hits=0;
  int clog_hits=0, bits_hits=0, cast_hits=0, neg_hits=0;
  function automatic real project(input S a); return a.v; endfunction
  function automatic real total(input S a[2]); return a[0].v + a[1].v; endfunction

  initial begin #0.5; forever @(project(arr[PARAM_IDX])) param_hits++; end
  initial begin #0.5; forever @(project(arr[LOCAL_IDX])) local_hits++; end
  initial begin #0.5; forever @(project(arr[event_indices::IDX])) pkg_hits++; end
  initial begin #0.5; forever @(project(arr[$signed(1)])) signed_hits++; end
  initial begin #0.5; forever @(project(arr[$unsigned(1)])) unsigned_hits++; end
  initial begin #0.5; forever @(project(arr[LOCAL_IDX+0])) expr_hits++; end
  initial begin #0.5; forever @(project(grid[1][0])) grid_hits++; end
  initial begin #0.5; forever @(total(arr)) whole_hits++; end

  initial begin #0.5; forever @(project(signed_range[4/2])) div_hits++; end
  initial begin #0.5; forever @(project(signed_range[5%3])) mod_hits++; end
  initial begin #0.5; forever @(project(signed_range[1<<LOCAL_IDX])) shift_hits++; end
  initial begin #0.5; forever @(project(signed_range[LOCAL_IDX ? 2 : 0])) cond_hits++; end
  initial begin #0.5; forever @(project(signed_range[{1'b1,1'b0}])) concat_hits++; end
  initial begin #0.5; forever @(project(signed_range[$clog2(4)])) clog_hits++; end
  initial begin #0.5; forever @(project(signed_range[$bits(WIDTH_TWO)])) bits_hits++; end
  initial begin #0.5; forever @(project(signed_range[int'(2'b10)])) cast_hits++; end
  initial begin #0.5; forever @(project(signed_range[-1])) neg_hits++; end

  initial begin
    arr='{'{1.25},'{1.25}};
    grid='{'{'{1.25},'{1.25}},'{'{1.25},'{1.25}}};
    #1 arr[1].v=2.5; grid[1][0].v=2.5;
    signed_range[2].v=2.5; signed_range[-1].v=2.5;
    #1 arr[0].v=3.75; grid[0][0].v=3.75; grid[1][1].v=3.75;
    signed_range[0].v=3.75; signed_range[1].v=3.75;
    #1 $display("counts=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d",
                param_hits,local_hits,pkg_hits,signed_hits,unsigned_hits,expr_hits,grid_hits,whole_hits);
    $display("indices=%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d,%0d",
             div_hits,mod_hits,shift_hits,cond_hits,concat_hits,clog_hits,bits_hits,cast_hits,neg_hits);
    $finish;
  end
endmodule
"#;

const COLLECTION_ALWAYS: &str = r#"
`timescale 1ns/1ps
module child;
  typedef struct {real v;} S;
  function automatic real project(input S a); return a.v; endfunction
  S child_d[];
  S child_q[$];
  S child_aa[int];
  int runs=0;
  // These three always blocks guard progress to $finish.
  always @(project(child_d[0])) if ($time > 0) runs++;
  always @(project(child_q[0])) if ($time > 0) runs++;
  always @(project(child_aa[1])) if ($time > 0) runs++;
  initial begin
    child_d=new[1]; child_d[0]='{1.25}; child_q.push_back('{1.25}); child_aa[1]='{1.25};
    #1 child_d[0].v=2.5; child_q[0].v=2.5; child_aa[1].v=2.5;
  end
endmodule

module top;
  child u();
  typedef struct {real v;} S;
  function automatic real project(input S a); return a.v; endfunction
  S d[];
  S q[$];
  S aa[int];
  int runs=0;
  // These three always blocks also guard progress to $finish.
  always @(project(d[0])) if ($time > 0) runs++;
  always @(project(q[0])) if ($time > 0) runs++;
  always @(project(aa[1])) if ($time > 0) runs++;
  initial begin
    d=new[1]; d[0]='{1.25}; q.push_back('{1.25}); aa[1]='{1.25};
    #1 d[0].v=2.5; q[0].v=2.5; aa[1].v=2.5;
    #1 $display("reached finish");
    $finish;
  end
endmodule
"#;

#[test]
fn struct_net_variable_port_and_array_arguments_wake() {
    // §6.6.7, §7.2, §7.4.2, §13.5.1 and §23.2.2.3: each argument
    // supplies its member value. The unrelated array element cannot wake it.
    let sim = xezim::simulate(AGGREGATE_ARGUMENTS, 100_000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "counts=1,1,1,1"),
        "{:?}",
        sim.output
    );
}

#[test]
fn struct_argument_events_preserve_result_and_edge_guards() {
    // §7.2.1, §9.4.2 and §9.4.2.3: only a change of the call result
    // qualifies. On the edge term the rising result also needs a true iff.
    // The first three counts are guards that pass on main.
    let sim = xezim::simulate(RESULT_GUARDS, 100_000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "counts=0,1,3,1"),
        "{:?}",
        sim.output
    );
}

#[test]
fn level_sensitive_always_watches_scoped_structs_and_ports() {
    // §9.2.2.1, §9.4.2 and §23.2.2.3: the child local and input
    // variable each change the projection once after initialization.
    let sim = xezim::simulate(SCOPED_ALWAYS, 100_000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "counts=1,1,1,1,1"),
        "{:?}",
        sim.output
    );
}

#[test]
fn enclosing_operators_wake_on_their_result_change() {
    // §9.4.2 and §11.4: both real writes change the arithmetic results.
    // Only the first write crosses the threshold. The other member is unused.
    let sim = xezim::simulate(WRAPPED_CALLS, 100_000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "counts=2,1,2,2,1"),
        "{:?}",
        sim.output
    );
}

#[test]
fn fixed_arrays_allow_constant_selects_and_whole_arguments() {
    // §6.20, §6.24.1, §7.4.2, §9.4.2, §11.2.1, §20.5, §20.6.2, §20.8
    // and §26.3: each constant select names one element. The whole-array
    // call observes both elements.
    let sim = xezim::simulate(FIXED_ARRAY_ARGUMENTS, 100_000).expect("simulate");
    assert!(
        sim.output
            .iter()
            .any(|o| o.message == "counts=1,1,1,1,1,1,1,2"),
        "{:?}",
        sim.output
    );
    assert!(
        sim.output
            .iter()
            .any(|o| o.message == "indices=1,1,1,1,1,1,1,1,1"),
        "{:?}",
        sim.output
    );
}

#[test]
fn always_on_struct_collection_elements_reaches_finish() {
    // §7.5, §7.8, §7.10 and §9.4.2: the six always blocks are termination
    // guards. Assert only that the process reaches the explicit $finish.
    let sim = xezim::simulate(COLLECTION_ALWAYS, 100_000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "reached finish"),
        "{:?}",
        sim.output
    );
}
