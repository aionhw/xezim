//! IEEE 1800-2023 §§7.4.2, 7.4.4, 7.4.5, 10.10: a selected fixed row
//! contributes each element in declared order to an unpacked concatenation.

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
fn blocking_row_and_delayed_snapshot() {
    // §9.4.5: the delayed assignment reads the row and scalar before waiting.
    check(
        r#"
module top;
  timeunit 1ns;
  timeprecision 1ns;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int dst[3];
  initial begin
    #1;
    dst = {src[1], src[0][1]};
    $display("T|direct %0d %0d %0d", dst[0], dst[1], dst[2]);
    dst = #4 {src[1], src[0][1]};
    $display("T|delay %0d %0d %0d", dst[0], dst[1], dst[2]);
    $finish;
  end
  initial begin
    #2;
    src[1][0] = 66;
    src[0][1] = 55;
  end
endmodule
"#,
        &["T|direct 6 7 5", "T|delay 6 7 5"],
    );
}

#[test]
fn bounds_rank_and_selector_evaluation() {
    // §§6.20, 7.4.2, 7.4.4: fixed bounds can be descending and negative,
    // and a row can be selected from a rank-three source.
    // §7.6: a descending destination starts at its left bound.
    check(
        r#"
module top;
  timeunit 1ns;
  timeprecision 1ns;
  parameter int ROW = -1;
  int src[-2:-1][-1:-2] = '{'{4, 5}, '{6, 7}};
  int cube[2][2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}};
  int whole[2] = '{6, 7};
  int dst[3], down[5:3], repeated[4];
  int calls;
  int k;
  function automatic int select_row();
    calls++;
    return 1;
  endfunction
  initial begin
    #1;
    dst = {src[ROW], src[-2][-2]};
    $display("T|parameter %0d %0d %0d", dst[0], dst[1], dst[2]);
    k = -1;
    repeated = {src[k], src[k]};
    $display("T|repeated %0d %0d %0d %0d",
             repeated[0], repeated[1], repeated[2], repeated[3]);
    down = {src[k], src[-2][-2]};
    $display("T|descending target %0d %0d %0d", down[5], down[4], down[3]);
    down = {6, 7, 5};
    $display("T|descending scalars %0d %0d %0d", down[5], down[4], down[3]);
    down = {whole, src[-2][-2]};
    $display("T|descending whole %0d %0d %0d", down[5], down[4], down[3]);
    dst = {cube[select_row()][0], cube[1][1][1]};
    $display("T|rank three %0d %0d %0d calls=%0d", dst[0], dst[1], dst[2], calls);
    $finish;
  end
endmodule
"#,
        &[
            "T|parameter 6 7 5",
            "T|repeated 6 7 6 7",
            "T|descending target 6 7 5",
            "T|descending scalars 6 7 5",
            "T|descending whole 6 7 5",
            "T|rank three 5 6 8 calls=1",
        ],
    );
}

#[test]
fn packed_struct_elements() {
    // §7.2.1: each packed structure contributes one value.
    check(
        r#"
module top;
  timeunit 1ns;
  timeprecision 1ns;
  typedef struct packed { int v; } item_t;
  item_t src[2][2];
  item_t dst[3];
  initial begin
    #1;
    src[1][0].v = 6;
    src[1][1].v = 7;
    src[0][1].v = 5;
    dst = {src[1], src[0][1]};
    $display("T|%0d %0d %0d", dst[0].v, dst[1].v, dst[2].v);
    $finish;
  end
endmodule
"#,
        &["T|6 7 5"],
    );
}

#[test]
fn nonblocking_and_event_capture() {
    // §§9.4.5, 10.4.2: the RHS values are captured before scheduling the store.
    check(
        r#"
module top;
  timeunit 1ns;
  timeprecision 1ns;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int nba[3], delayed[3], on_event[3];
  event e;
  initial begin
    #1;
    nba <= {src[1], src[0][1]};
    delayed <= #4 {src[1], src[0][1]};
    on_event = @(e) {src[1], src[0][1]};
    #2;
    $display("T|nba %0d %0d %0d", nba[0], nba[1], nba[2]);
    $display("T|delayed %0d %0d %0d", delayed[0], delayed[1], delayed[2]);
    $display("T|event %0d %0d %0d", on_event[0], on_event[1], on_event[2]);
    $finish;
  end
  initial begin
    #2;
    src[1][0] = 66;
    src[0][1] = 55;
    #2 -> e;
  end
endmodule
"#,
        &["T|nba 6 7 5", "T|delayed 6 7 5", "T|event 6 7 5"],
    );
}

#[test]
fn collection_destinations_and_operand_guards() {
    // §§7.6, 7.10, 10.10: collection destinations resize to the RHS length.
    // §§7.6, 10.10: a compatible array-valued item contributes one element.
    // §10.8: array input arguments supply the target type for the concat.
    // §9.4.5: a delayed whole-array copy retains the captured values.
    check(
        r#"
module top;
  timeunit 1ns;
  timeprecision 1ns;
  int src[2][2] = '{'{4, 5}, '{6, 7}};
  int whole[1:0] = '{6, 7};
  int q[$], d[];
  int qq[$][$], dd[][];
  int dst[3], saved[3] = '{6, 7, 5};
  logic [7:0] packed_items[2];
  string words[2][2] = '{'{"A", "B"}, '{"C", "D"}}, joined[3];
  int calls;
  class source;
    int rows[2][2];
    int result[3];
    function new();
      rows[1][0] = 6;
      rows[1][1] = 7;
    endfunction
  endclass
  source obj;
  function automatic int select_row();
    calls++;
    return 1;
  endfunction
  function automatic int fixed_arg(input int a[3]);
    return a[0]*100 + a[1]*10 + a[2];
  endfunction
  function automatic int queue_arg(input int a[$]);
    return a.sum();
  endfunction
  initial begin
    #1;
    q = {src[select_row()], src[0][1]};
    d = {src[1], src[0][1]};
    $display("T|queue %0d: %0d %0d %0d calls=%0d", q.size(), q[0], q[1], q[2], calls);
    $display("T|dynamic %0d: %0d %0d %0d", d.size(), d[0], d[1], d[2]);
    qq = {src[1], src[0]};
    dd = {src[1], src[0]};
    $display("T|array elements %0d %0d", qq.size(), dd.size());
    $display("T|fixed arg %0d", fixed_arg({src[1], src[0][1]}));
    $display("T|queue arg %0d", queue_arg({src[1], src[0][1]}));
    obj = new;
    obj.result = {obj.rows[1], src[0][1]};
    $display("T|class row %0d %0d %0d", obj.result[0], obj.result[1], obj.result[2]);
    joined = {words[1], words[0][1]};
    $display("T|string row %s %s %s", joined[0], joined[1], joined[2]);
    dst = {whole, src[0][1]};
    $display("T|whole %0d %0d %0d", dst[0], dst[1], dst[2]);
    dst = {6, 7, 5};
    $display("T|scalars %0d %0d %0d", dst[0], dst[1], dst[2]);
    packed_items = {8'({4'd6, 4'd7}), 8'd5};
    $display("T|packed concat %h %h", packed_items[0], packed_items[1]);
    q = {whole, 5};
    d = {q, 9};
    $display("T|queue operand %0d: %0d %0d %0d %0d", d.size(), d[0], d[1], d[2], d[3]);
    q = {d, 8};
    $display("T|dynamic operand %0d: %0d %0d %0d %0d %0d",
             q.size(), q[0], q[1], q[2], q[3], q[4]);
    saved = #4 saved;
    $display("T|self-copy %0d %0d %0d", saved[0], saved[1], saved[2]);
    $finish;
  end
  initial begin
    #2;
    saved[0] = 66;
    saved[2] = 55;
  end
endmodule
"#,
        &[
            "T|queue 3: 6 7 5 calls=1",
            "T|dynamic 3: 6 7 5",
            "T|array elements 2 2",
            "T|fixed arg 675",
            "T|queue arg 18",
            "T|class row 6 7 5",
            "T|string row C D B",
            "T|whole 6 7 5",
            "T|scalars 6 7 5",
            "T|packed concat 67 05",
            "T|queue operand 4: 6 7 5 9",
            "T|dynamic operand 5: 6 7 5 9 8",
            "T|self-copy 6 7 5",
        ],
    );
}

#[test]
fn invalid_row_select_has_fixed_size() {
    // §7.4.5 Table 7-1: an invalid fixed-array entry is defaulted elementwise.
    // §§6.19.2, 7.2.1, 7.3.1: the enum base and packed members determine state.
    check(
        r#"
module top;
  timeunit 1ns;
  timeprecision 1ns;
  typedef logic [7:0] vector_t;
  typedef struct packed { logic [7:0] a; bit [7:0] b; } packed_t;
  typedef union packed { logic [15:0] a; bit [15:0] b; } union_t;
  typedef enum logic [1:0] { E0, E1 } enum_t;
  logic [7:0] src[2][2] = '{'{1, 2}, '{3, 4}};
  logic [7:0] dst[3];
  int numbers[2][2] = '{'{1, 2}, '{3, 4}};
  int fallback[3];
  vector_t v[2][2] = '{'{1, 2}, '{3, 4}}, vd[3];
  packed_t p[2][2] = '{'{16'h1111, 16'h2222}, '{16'h3333, 16'h4444}}, pd[3];
  union_t u[2][2] = '{'{16'h1111, 16'h2222}, '{16'h3333, 16'h4444}}, ud[3];
  enum_t e[2][2] = '{'{E0, E1}, '{E1, E0}}, ed[3];
  logic [95:0] k;
  int i;
  initial begin
    #1;
    k = 96'h1_0000_0000_0000_0001;
    dst = {src[k], 8'd5};
    if ((dst[0] === 8'hxx) && (dst[1] === 8'hxx) && (dst[2] === 8'd5))
      $display("T|wide invalid index");
    k = 'x;
    dst = {src[k], 8'd5};
    if ((dst[0] === 8'hxx) && (dst[1] === 8'hxx) && (dst[2] === 8'd5))
      $display("T|unknown index");
    i = 2;
    fallback = {numbers[i], 5};
    $display("T|two-state %0d %0d %0d", fallback[0], fallback[1], fallback[2]);
    vd = {v[i], 8'h05};
    pd = {p[i], 16'h0505};
    ud = {u[i], 16'h0505};
    ed = {e[i], E1};
    $display("T|typedef %h %h %h", vd[0], vd[1], vd[2]);
    $display("T|packed struct %h %h %h", pd[0], pd[1], pd[2]);
    $display("T|packed union %h %h %h", ud[0], ud[1], ud[2]);
    $display("T|enum %b %b %b", ed[0], ed[1], ed[2]);
    $finish;
  end
endmodule
"#,
        &[
            "T|wide invalid index",
            "T|unknown index",
            "T|two-state 0 0 5",
            "T|typedef xx xx 05",
            "T|packed struct xxxx xxxx 0505",
            "T|packed union xxxx xxxx 0505",
            "T|enum xx xx 01",
        ],
    );
}

#[test]
fn class_property_is_the_only_multidimensional_array() {
    check(
        r#"
module top;
  class holder;
    int rows[2][2];
  endclass
  holder obj;
  int dst[3];
  initial begin
    obj = new;
    obj.rows[1][0] = 31;
    obj.rows[1][1] = 42;
    dst = {obj.rows[1], 53};
    $display("T|class-only %0d %0d %0d", dst[0], dst[1], dst[2]);
    $finish;
  end
endmodule
"#,
        &["T|class-only 31 42 53"],
    );
}
