//! A queue, dynamic array or associative array declared in a sub-instance
//! is registered under its instance path, but a bare reference inside the
//! instance (`q.push_back(t)`, `q[0].f`, `aa["k"]`) resolved to the leaf
//! name only: two instances of the same module shared one lazily created
//! top-level queue, and the element type keyed by the instance path was
//! missed, so a packed-struct element's fields read 0. A UVM-style BFM
//! request queue (10 per-client instances) never granted a request.
use xezim::simulate;

fn messages_until(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn messages(src: &str) -> Vec<String> {
    messages_until(src, 1_000)
}

#[test]
fn collections_in_sibling_instances_stay_separate_and_typed() {
    let msgs = messages(
        "package p; typedef struct packed { logic [31:0] wait_cnt; logic [7:0] src_id; } req_t; endpackage
module unit(input logic push, input logic clk, input logic [7:0] id); import p::*;
  req_t q[$]; int qs; int aa[string]; int da[];
  always_comb qs = q.size();
  function void push_one(input logic [7:0] i); req_t t; t.src_id = i; t.wait_cnt = 5; q.push_back(t); endfunction
  always @(posedge clk) if (push) begin
    push_one(id); q[0].wait_cnt = q[0].wait_cnt - 1;
    aa[\"k\"] = aa.exists(\"k\") ? aa[\"k\"] + 1 : 1;
    da = new[da.size() + 1]; da[da.size() - 1] = id;
    $display(\"%m size=%0d wait=%0d src=%0d aa=%0d da=%0d\", q.size(), q[0].wait_cnt, q[0].src_id, aa[\"k\"], da.size());
  end
endmodule
module wrap(input logic clk, input logic p2, input logic p7);
  unit u2(.push(p2), .clk(clk), .id(8'd2)); unit u7(.push(p7), .clk(clk), .id(8'd7));
endmodule
module tb; logic clk = 0; always #5 clk = ~clk; logic p2 = 0, p7 = 0; wrap env(.clk(clk), .p2(p2), .p7(p7));
  initial begin
    @(negedge clk); p7 = 1; @(negedge clk); p7 = 0; @(negedge clk); p2 = 1; @(negedge clk); p2 = 0; #1;
    $display(\"hier u2=%0d/%0d u7=%0d/%0d\", env.u2.q.size(), env.u2.qs, env.u7.q.size(), env.u7.qs);
    $finish;
  end
endmodule",
    );
    for want in [
        "tb.env.u7 size=1 wait=4 src=7 aa=1 da=1",
        "tb.env.u2 size=1 wait=4 src=2 aa=1 da=1",
        "hier u2=1/1 u7=1/1",
    ] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}

/// The receiver of a collection method call is cached per source span and
/// scope hint. A `for` body runs with no hint, and every sibling's inlined
/// copy of the line shares the span, so `q.size()` inside the loop read the
/// first sibling's queue: a BFM saw its own queue as empty and never
/// decremented the wait counters. The head identifier (already prefixed
/// per instance) is part of the key now.
#[test]
fn collection_method_inside_loop_body_reads_own_instance() {
    let msgs = messages(
        "package p; typedef struct packed { logic [31:0] w; logic [7:0] s; } req_t; endpackage
module unit(input logic clk, input logic go); import p::*;
  req_t q[$]; logic [7:0][15:0] pend;
  function void push1(input logic [7:0] n); req_t t; t.s = n; t.w = 5; q.push_back(t); endfunction
  always @(posedge clk) if (go) push1(3);
  always @(posedge clk) begin
    for (int i = 0; i < 2; i++) begin
      pend[i] <= pend[i] + 1;
      if (q.size()) $display(\"%m in i=%0d size=%0d\", i, q.size());
    end
    if (q.size()) begin
      q[0].w = q[0].w - 1;
      $display(\"%m after size=%0d w0=%0d\", q.size(), q[0].w);
    end
  end
endmodule
module tb; logic clk = 0, go = 0; always #5 clk = ~clk;
  unit u0(.clk(clk), .go(1'b0)); unit u1(.clk(clk), .go(go));
  initial begin #12 go = 1; #10 go = 0; #10 $finish; end
endmodule",
    );
    for want in [
        "tb.u1 in i=0 size=1",
        "tb.u1 in i=1 size=1",
        "tb.u1 after size=1 w0=4",
        "tb.u1 after size=1 w0=3",
    ] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
    assert!(!msgs.iter().any(|m| m.starts_with("tb.u0 ")), "u0 has no queue: {msgs:?}");
}

/// The full shape of the user's report: ten per-client BFM wrappers, each
/// holding a request queue of packed structs; requests are pushed from a
/// function, wait counters are decremented per cycle through `q[i].field`
/// writes inside a `for`, grants fire on the negedge, and a consumer drains
/// the held queue. Every ingredient above was broken in an instance.
#[test]
fn bfm_request_queues_grant_and_drain_in_ten_sibling_wrappers() {
    let msgs = messages_until(r#"`timescale 1ns/1ps

package slc_pkg;
  typedef struct packed {
    logic [255:0] payload;
    logic [31:0]  wait_cnt;
    logic [7:0]   src_id;
  } slc_req_t;
endpackage

module slc_bfm_req_q (
  input logic clk,
  input logic rst_l,
  input tri0  [7:0] cli_req_vld,
  input logic [7:0][255:0] cli_req_dat,
  output logic [7:0] cli_req_gnt,
  input  logic out_read,
  output logic out_nonempty,
  output logic [255:0] out_payload
);
  import slc_pkg::*;
  parameter MAX_HELD = 32;

  slc_req_t held_q[$];
  slc_req_t new_q[$];
  slc_req_t current_rx;
  logic [7:0] cli_req_vld_d1;
  int gnt_dly_rng, req_dly_rng;
  int stat_min_r2g, stat_min_g2s;
  logic [7:0][15:0] pend_cnt;
  int held_size, new_size;
  int size_snap;

  assign out_payload = (current_rx.src_id << 128) | current_rx.payload;

  always_comb begin
    held_size = held_q.size();
    new_size  = new_q.size();
  end

  // second, independent view of the queue size (posedge-process read)
  always @(posedge clk) size_snap <= new_q.size();

  initial begin
    gnt_dly_rng = 7;    // wait counters 1..8: exercises the per-cycle
    req_dly_rng = 0;    // decrement of queued elements before a grant
  end

  always @(posedge clk) begin
    for (int i = 0; i < 8; i++) begin
      if (rst_l !== 1'b1)
        pend_cnt[i] <= 0;
      else if (cli_req_vld[i] && !cli_req_gnt[i])
        pend_cnt[i] <= pend_cnt[i] + 1;
      else if (!cli_req_vld[i] && cli_req_gnt[i])
        pend_cnt[i] <= pend_cnt[i] - 1;
    end
    if (!rst_l) begin
      cli_req_vld_d1 <= 0;
    end else if (rst_l === 1) begin
      cli_req_vld_d1 <= cli_req_vld;
    end
    // (guard mirrors the upstream consumer protocol: read only when a
    // held entry is present)
    if (out_read && held_q.size())
      void'(held_q.pop_front());
    if (new_q.size() && (held_size < MAX_HELD)) begin
      if (new_q[0].wait_cnt > 0)
        new_q[0].wait_cnt = new_q[0].wait_cnt - 1;
      for (int i = 1; i < new_q.size(); i++) begin
        if (new_q[i].wait_cnt > 1)
          new_q[i].wait_cnt = new_q[i].wait_cnt - 1;
      end
    end
    for (int i = 0; i < 8; i++)
      if (cli_req_vld_d1[i])
        rx_push_new(cli_req_dat[i], i);
    if (|cli_req_gnt) begin
      slc_req_t temp_req;
      stat_min_g2s = 8;
      temp_req = new_q.pop_front();
      temp_req.wait_cnt = $urandom % (req_dly_rng + 1);
      held_q.push_back(temp_req);
    end
  end

  always @(negedge clk) begin
    if (new_q.size() && (held_size < MAX_HELD)) begin
      for (int i = 0; i < 8; i++)
        cli_req_gnt[i] = (new_q[0].wait_cnt == 1 && new_q[0].src_id == i) ? 1'b1 : 1'b0;
    end else begin
      cli_req_gnt = 0;
    end
    if (held_q.size()) begin
      out_nonempty = (held_q[0].wait_cnt == 0);
      current_rx = held_q[0];
    end else begin
      out_nonempty = 0;
      current_rx = '0;
    end
  end

  // internal self-observer: prints only cycles with queue activity
  always @(posedge clk)
    if (rst_l === 1'b1 && (cli_req_vld_d1 != 0 || new_q.size() != 0 || held_q.size() != 0))
      $display("%0t: %m: obs new=%0d held=%0d neww0=%0d pend0=%0d gnt=%b d1=%b",
               $time, new_q.size(), held_q.size(), (new_q.size() ? new_q[0].wait_cnt : 0),
               pend_cnt[0], cli_req_gnt, cli_req_vld_d1);

  function void rx_push_new(input logic [255:0] request_arg, input logic [7:0] req_number);
    slc_req_t temp_req;
    stat_min_r2g = 4;
    temp_req.payload  = request_arg;
    temp_req.src_id   = req_number;
    temp_req.wait_cnt = $urandom % (gnt_dly_rng + 1) + 1;
    new_q.push_back(temp_req);
  endfunction
endmodule
// ---PART2---
module slc_bfm (
  input logic clk,
  input logic rst_l,
  input tri0  [7:0] cli_req_vld,
  input logic [7:0][255:0] cli_req_dat,
  output logic [7:0] cli_req_gnt,
  input  logic out_read,
  output logic out_nonempty,
  output logic [255:0] out_payload
);
  slc_bfm_req_q u_q (
    .clk(clk), .rst_l(rst_l),
    .cli_req_vld(cli_req_vld), .cli_req_dat(cli_req_dat),
    .cli_req_gnt(cli_req_gnt), .out_read(out_read),
    .out_nonempty(out_nonempty), .out_payload(out_payload)
  );
endmodule

// Ten per-client wrappers in one environment, as in the production
// hierarchy. Clients 2 and 7 are driven; the rest sit idle.
module slc_env (
  input logic clk,
  input logic rst_l,
  input tri0  c2_vld,
  input tri0  c7_vld,
  input logic [255:0] c2_dat,
  input logic [255:0] c7_dat,
  input  logic c2_read,
  input  logic c7_read,
  output logic [7:0] c2_gnt,
  output logic [7:0] c7_gnt,
  output logic c2_nonempty,
  output logic c7_nonempty,
  output logic [255:0] c2_payload,
  output logic [255:0] c7_payload
);
  slc_bfm u_c0 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c1 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c2 (.clk(clk), .rst_l(rst_l), .cli_req_vld(c2_vld), .cli_req_dat({{7{256'h0}}, c2_dat}),
                .cli_req_gnt(c2_gnt), .out_read(c2_read), .out_nonempty(c2_nonempty), .out_payload(c2_payload));
  slc_bfm u_c3 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c4 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c5 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c6 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c7 (.clk(clk), .rst_l(rst_l), .cli_req_vld(c7_vld), .cli_req_dat({{7{256'h0}}, c7_dat}),
                .cli_req_gnt(c7_gnt), .out_read(c7_read), .out_nonempty(c7_nonempty), .out_payload(c7_payload));
  slc_bfm u_c8 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
  slc_bfm u_c9 (.clk(clk), .rst_l(rst_l), .cli_req_vld(1'b0), .cli_req_dat({8{256'h0}}), .cli_req_gnt(), .out_read(1'b0), .out_nonempty(), .out_payload());
endmodule
// ---PART3---
module tb_top;
  logic clk = 0;
  logic rst_l = 0;
  logic c2_vld = 0, c7_vld = 0;
  logic [255:0] c2_dat = '0, c7_dat = '0;
  wire  [7:0] c2_gnt, c7_gnt;
  wire  c2_nonempty, c7_nonempty;
  wire  [255:0] c2_payload, c7_payload;
  logic drain = 0;
  wire  c2_read, c7_read;
  assign c2_read = drain & c2_nonempty;
  assign c7_read = drain & c7_nonempty;
  int errors = 0;
  int g2_cnt = 0, g7_cnt = 0;
  int ne7_seen = 0, pl2_seen = 0;

  slc_env env (
    .clk(clk), .rst_l(rst_l),
    .c2_vld(c2_vld), .c7_vld(c7_vld), .c2_dat(c2_dat), .c7_dat(c7_dat),
    .c2_read(c2_read), .c7_read(c7_read),
    .c2_gnt(c2_gnt), .c7_gnt(c7_gnt),
    .c2_nonempty(c2_nonempty), .c7_nonempty(c7_nonempty),
    .c2_payload(c2_payload), .c7_payload(c7_payload)
  );

  always #5 clk = ~clk;

  // grant/nonempty/payload are negedge-driven in the DUT, so sampling
  // them at the posedge is race-free; each grant-high posedge pops
  // exactly one queued request (1:1 with grants)
  always @(posedge clk) begin
    if (c2_gnt[0]) g2_cnt = g2_cnt + 1;
    if (c7_gnt[0]) g7_cnt = g7_cnt + 1;
    if (c7_nonempty) ne7_seen = 1;
    if (c2_nonempty && (c2_payload != '0)) pl2_seen = 1;
  end

  task automatic drive_req(input int c, input logic [255:0] p);
    begin
      if (c == 7) begin
        @(negedge clk); c7_vld = 1'b1; c7_dat = p;
        @(negedge clk); c7_vld = 1'b0;
      end else begin
        @(negedge clk); c2_vld = 1'b1; c2_dat = p;
        @(negedge clk); c2_vld = 1'b0;
      end
    end
  endtask

  initial begin
    rst_l = 0;
    repeat (2) @(negedge clk);
    rst_l = 1'b1;
    repeat (2) @(negedge clk);
    // client 7: one early request; client 2: three later requests
    drive_req(7, 256'h0000_0000_0000_0000_0000_0000_0000_00A5);
    repeat (2) @(negedge clk);
    drive_req(2, 256'h0000_0000_0000_0000_0000_0000_0000_00B1);
    repeat (2) @(negedge clk);
    drive_req(2, 256'h0000_0000_0000_0000_0000_0000_0000_00B2);
    repeat (2) @(negedge clk);
    drive_req(2, 256'h0000_0000_0000_0000_0000_0000_0000_00B3);
    // wait counters are 1..8 with a per-cycle decrement: all grants
    // must complete well inside this window
    repeat (60) @(negedge clk);
    // consumer drain phase
    drain = 1'b1;
    repeat (12) @(negedge clk);
    drain = 1'b0;
    repeat (2) @(negedge clk);

    $display("RESULT grants: c2=%0d (want 3)  c7=%0d (want 1)", g2_cnt, g7_cnt);
    $display("RESULT sizes: c2 new=%0d snap=%0d held=%0d | c7 new=%0d snap=%0d held=%0d (want all 0)",
             env.u_c2.u_q.new_size, env.u_c2.u_q.size_snap, env.u_c2.u_q.held_size,
             env.u_c7.u_q.new_size, env.u_c7.u_q.size_snap, env.u_c7.u_q.held_size);
    $display("RESULT consumer: c7 nonempty seen=%0d  c2 payload seen=%0d  stat: c2 r2g=%0d g2s=%0d",
             ne7_seen, pl2_seen, env.u_c2.u_q.stat_min_r2g, env.u_c2.u_q.stat_min_g2s);

    if (g7_cnt != 1) begin
      errors++;
      $display("ERROR: c7 grant count %0d (want 1) - request never granted", g7_cnt);
    end
    if (g2_cnt != 3) begin
      errors++;
      $display("ERROR: c2 grant count %0d (want 3) - requests never granted (or lost/misplaced)", g2_cnt);
    end
    if (!ne7_seen) begin
      errors++;
      $display("ERROR: c7 consumer never saw a non-empty response");
    end
    if (!pl2_seen) begin
      errors++;
      $display("ERROR: c2 consumer never saw a response payload");
    end
    if ((env.u_c2.u_q.new_size != 0) || (env.u_c2.u_q.size_snap != 0) ||
        (env.u_c7.u_q.new_size != 0) || (env.u_c7.u_q.size_snap != 0)) begin
      errors++;
      $display("ERROR: request queues not drained: c2 new=%0d snap=%0d  c7 new=%0d snap=%0d",
               env.u_c2.u_q.new_size, env.u_c2.u_q.size_snap,
               env.u_c7.u_q.new_size, env.u_c7.u_q.size_snap);
    end
    if ((env.u_c2.u_q.held_size != 0) || (env.u_c7.u_q.held_size != 0)) begin
      errors++;
      $display("ERROR: held queues not drained: c2=%0d c7=%0d",
               env.u_c2.u_q.held_size, env.u_c7.u_q.held_size);
    end

    if (errors == 0) $display("TEST_PASS");
    else             $display("TEST_FAIL: %0d error(s)", errors);
    $finish;
  end
endmodule
"#, 100_000);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{msgs:?}");
    assert!(msgs.iter().any(|m| m == "RESULT grants: c2=3 (want 3)  c7=1 (want 1)"), "{msgs:?}");
}

/// Sibling audit: queue, associative array, associative array of packed
/// structs and array-of-queues element fields, read inside a `for` body and
/// written from a task, across ten generate siblings with distinct IDs.
#[test]
fn collection_shapes_stay_per_instance_across_ten_generate_siblings() {
    let msgs = messages_until(r#"package p; typedef struct packed { logic [31:0] w; logic [7:0] s; } req_t; endpackage
module unit #(parameter int ID = 0) (input logic clk, input logic go);
  import p::*;
  req_t q[$]; int aa[int]; req_t aq[int]; req_t g2[2][$]; int errs = 0;
  task automatic tpush(input int n); req_t t; t.s = n; t.w = ID; q.push_back(t); aa[n] = ID; aq[n].w = ID; aq[n].s = n; g2[1].push_back(t); endtask
  always @(posedge clk) if (go) tpush(3);
  always @(posedge clk) begin
    for (int i = 0; i < 2; i++) begin
      if (q.size() != 0 && q[0].w != ID) begin errs++; $display("%m E1 q[0].w=%0d", q[0].w); end
      if (aa.exists(3) && aa[3] != ID) begin errs++; $display("%m E2 aa[3]=%0d", aa[3]); end
      if (aa.exists(3) && aq[3].w != ID) begin errs++; $display("%m E3 aq[3].w=%0d", aq[3].w); end
      if (g2[1].size() != q.size()) begin errs++; $display("%m E4 g2[1].size=%0d q.size=%0d", g2[1].size(), q.size()); end
      if (g2[1].size() != 0 && g2[1][0].w != ID) begin errs++; $display("%m E5 g2[1][0].w=%0d", g2[1][0].w); end
    end
    if (q.size() > 2) begin void'(q.pop_front()); void'(g2[1].pop_front()); aa.delete(3); end
  end
  final if (errs == 0) $display("%m OK q=%0d aa=%0d g2=%0d", q.size(), aa.num(), g2[1].size()); else $display("%m FAIL errs=%0d", errs);
endmodule
module tb_top; logic clk = 0, go = 0; always #5 clk = ~clk;
  genvar k; generate for (k = 0; k < 10; k++) begin : g unit #(.ID(k+1)) u(.clk(clk), .go(go)); end endgenerate
  initial begin #12 go = 1; #40 go = 0; #30 $finish; end
endmodule
"#, 100_000);
    for k in 0..10 {
        let want = format!("tb_top.g[{k}].u OK q=2 aa=0 g2=2");
        assert!(msgs.iter().any(|m| m == &want), "missing {want}: {msgs:?}");
    }
    assert!(!msgs.iter().any(|m| m.contains(" E") || m.contains("FAIL")), "{msgs:?}");
}

/// Element field of a queue, fixed array and dynamic array of packed structs
/// inside an instance, against the same code at the top level.
#[test]
fn packed_struct_element_fields_in_instance_containers() {
    let msgs = messages_until(r#"package p; typedef struct packed { logic [31:0] w; logic [7:0] s; } req_t; endpackage
module unit(input logic go); import p::*;
  req_t q[$]; req_t arr[2]; req_t sc; req_t da[];
  always @(posedge go) begin
    req_t t; t.s = 7; t.w = 5; q.push_back(t); arr[0] = t; sc = t; da = new[1]; da[0] = t;
    $display("%m: queue q[0].w=%0d | fixed arr[0].w=%0d | scalar sc.w=%0d | dyn da[0].w=%0d (want 5 5 5 5)", q[0].w, arr[0].w, sc.w, da[0].w);
  end
endmodule
module tb_top; import p::*; logic go = 0; unit u(.go(go));
  req_t q[$]; req_t arr[2]; req_t sc; req_t da[];
  initial begin req_t t; t.s = 7; t.w = 5; q.push_back(t); arr[0] = t; sc = t; da = new[1]; da[0] = t;
    $display("top: queue q[0].w=%0d | fixed arr[0].w=%0d | scalar sc.w=%0d | dyn da[0].w=%0d (want 5 5 5 5)", q[0].w, arr[0].w, sc.w, da[0].w);
    #1 go = 1; #1 $finish; end
endmodule
"#, 1_000);
    for want in [
        "top: queue q[0].w=5 | fixed arr[0].w=5 | scalar sc.w=5 | dyn da[0].w=5 (want 5 5 5 5)",
        "tb_top.u: queue q[0].w=5 | fixed arr[0].w=5 | scalar sc.w=5 | dyn da[0].w=5 (want 5 5 5 5)",
    ] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}
