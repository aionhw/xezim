//! §7.6, §7.2, §13.4.1 (#1057): an unpacked array whose elements are
//! unpacked structs copies element by element, member by member, between
//! differently shaped arrays — fixed-size, dynamic and queue, variables and
//! class properties — and through a function's return, also a local queue
//! or dynamic array of structs. The elements have member leaves and no value
//! of their own, so a copy of each element's (nonexistent) value left every
//! member at its default. Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

fn expect(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:#?}");
    }
    assert!(
        !msgs.iter().any(|m| m.starts_with("FAIL")),
        "unexpected FAIL in {msgs:#?}"
    );
}

/// The reporter's register-table program: the class-member fixed array
/// returned as a dynamic array, plain copies, the element-type matrix, the
/// in-frame shape controls (a field-by-field copy into a local dynamic array
/// too) and the loader that hung when the table arrived as zeros.
#[test]
fn register_table_survives_every_copy() {
    let msgs = lines(
        r##"
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) $display("TEST_PASS"); \
  else               $display("TEST_FAIL count=%0d", failures);

`endif

package cfgmap_pkg;

  typedef struct {
    bit [15:0] addr;
    bit [31:0] data;
  } reg_slot_t;

  typedef reg_slot_t reg_slot_q[];

  typedef struct {
    bit [31:0] rd;
    bit [31:0] wr;
  } acc_slot_t;

  typedef acc_slot_t acc_slot_q[];

  virtual class AbsRegMap;
    reg_slot_t slot_table[];                     // dynamic; shadowed by the derived
    virtual function reg_slot_q make_slots();
    endfunction
    virtual function acc_slot_q make_access();
    endfunction
  endclass

endpackage

package lane_pkg;

  import cfgmap_pkg::*;

  parameter int NSLOT   = 7;
  parameter int BUS_W   = 32;
  parameter int STALL_K = 400;                 // watchdog horizon, clock cycles

  virtual class AbsLaneRegs extends AbsRegMap;

    rand bit [1:0]   R_MODE;
    rand bit [1:0]   R_DEPTH;
    rand bit [31:0]  R_PAIR;
    rand bit [271:0] R_SHIFT_LO;
    rand bit [239:0] R_SHIFT_HI;

    reg_slot_t slot_table[NSLOT];               // fixed; shadows the base member

    function reg_slot_q make_slots();
    begin
      slot_table[0].addr      = 0;
      slot_table[0].data      = 32'h0000_0005;   // mode 1, depth 1
      slot_table[1].addr      = 1;
      slot_table[1].data      = 32'h0A0A_0A01;
      slot_table[2].addr      = 2;
      slot_table[2].data      = 32'h0B0B_0B02;
      slot_table[3].addr      = 3;
      slot_table[3].data      = 32'h0C0C_0C03;
      slot_table[4].addr      = 4;
      slot_table[4].data      = 32'h0D0D_0D04;
      slot_table[5].addr      = 5;
      slot_table[5].data      = 32'h0E0E_0E05;
      slot_table[6].addr      = 6;
      slot_table[6].data      = 32'h0F0F_0F06;
      return slot_table;                        // fixed-size unpacked -> dynamic
    end
    endfunction

    function acc_slot_q make_access();
    begin
      acc_slot_t acc[NSLOT];
      foreach (acc[i]) begin
        acc[i].rd = {BUS_W{1'b1}};
        acc[i].wr = {BUS_W{1'b1}};
      end
      return acc;                               // same shape, second struct type
    end
    endfunction

  endclass

  class LaneRegs extends AbsLaneRegs;
    int user_mode;
    function new();
      user_mode = 1;
    endfunction
    constraint c_mode { R_MODE == user_mode[1:0]; R_DEPTH == 2'd1; }
  endclass

  class CfgBus;
    bit        clk;
    bit        rst_n;
    bit        load_enable;
    bit        slot_strobe;
    bit [15:0] slot_addr;
    bit [31:0] slot_data;
  endclass

  class SlotLoader;
    CfgBus bus;

    task run(input reg_slot_q slots, input acc_slot_q accs);
    begin
      bus.load_enable = 1;
      @(posedge bus.clk);
      if (slots.size() != 0) begin
        foreach (slots[i]) begin
          bus.slot_strobe = 1;
          bus.slot_addr   = slots[i].addr;
          bus.slot_data   = slots[i].data;
          @(posedge bus.clk);
        end
      end
      bus.slot_strobe = 0;
      @(posedge bus.clk);
      bus.load_enable = 0;
      @(posedge bus.clk);
    end
    endtask
  endclass

  class StreamFeeder;
    CfgBus bus;
    int    stream_id;
    bit    beat_valid;
    bit    beat_last;
    bit    feed_ok;                    // driven from the unit side
    int    beats_sent;
    bit    stalled;

    task run(input int width, input int height);
    begin
      beats_sent = 0;
      stalled    = 0;
      for (int y = 0; y < height; y++) begin
        for (int x = 0; x < width; x++) begin
          beat_valid = 1;
          beat_last  = (y == height-1) && (x == width-1);
          begin
            int guard = 0;
            while (feed_ok !== 1'b1) begin
              @(posedge bus.clk);
              guard++;
              if (guard > STALL_K) begin
                stalled = 1;
                return;
              end
            end
          end
          beat_valid = 0;
          beats_sent++;
          @(posedge bus.clk);
        end
      end
    end
    endtask
  endclass

endpackage

package probe_pkg;

  typedef struct { bit [15:0] a; bit [31:0] d; } urec2;
  typedef urec2 urec2_q[];
  typedef struct { bit [31:0] v; } urec1;
  typedef urec1 urec1_q[];
  typedef struct { int a; int d; } ureci;
  typedef ureci ureci_q[];
  typedef struct { bit [3:0][7:0] p; } urecpa;
  typedef urecpa urecpa_q[];
  typedef struct { urec2 inner; bit [7:0] t; } urecn;
  typedef urecn urecn_q[];
  typedef struct { logic [31:0] l; bit [7:0] b; } urec4;
  typedef urec4 urec4_q[];
  typedef struct { bit [7:0] a; string s; } urecstr;
  typedef urecstr urecstr_q[];

  typedef struct packed { bit [7:0] x; bit [7:0] y; } prec;
  typedef prec prec_q[];
  typedef enum bit [2:0] {EA, EB, EC} etyp;
  typedef etyp etyp_q[];
  typedef bit [15:0] styp;
  typedef styp styp_q[];
  typedef int ityp_q[];

  class Obj; int id; function new(int i); id = i; endfunction endclass
  typedef Obj obj_q[];

  typedef urec2 urec2_qq[$];                                 // queue destination
  typedef urec2 urec2_fx[3];                                 // fixed destination

  function automatic urec2_q b_urec2();
    urec2 t[3];
    t[0] = '{1,10}; t[1] = '{2,20}; t[2] = '{3,30};
    return t;
  endfunction

  function automatic urec1_q b_urec1();
    urec1 t[3];
    t[0].v = 32'h11; t[1].v = 32'h22; t[2].v = 32'h33;
    return t;
  endfunction

  function automatic ureci_q b_ureci();
    ureci t[3];
    t[0] = '{7,8}; t[1] = '{9,10}; t[2] = '{11,12};
    return t;
  endfunction

  function automatic urecpa_q b_urecpa();
    urecpa t[2];
    t[0].p = 32'hAAAA_1111; t[1].p = 32'hBBBB_2222;
    return t;
  endfunction

  function automatic urecn_q b_urecn();
    urecn t[2];
    t[0].inner.a = 1; t[0].inner.d = 1; t[0].t = 8'hAA;
    t[1].inner.a = 3; t[1].inner.d = 4; t[1].t = 8'hBB;
    return t;
  endfunction

  function automatic urec4_q b_urec4();
    urec4 t[2];
    t[0] = '{32'h1111_1111, 8'h22};
    t[1] = '{32'h3333_3333, 8'h44};
    return t;
  endfunction

  function automatic urecstr_q b_urecstr();
    urecstr t[2];
    t[0] = '{8'h11, "aa"}; t[1] = '{8'h22, "bb"};
    return t;
  endfunction

  function automatic prec_q b_prec();
    prec t[2];
    t[0] = '{8'h11, 8'h22}; t[1] = '{8'h33, 8'h44};
    return t;
  endfunction

  function automatic etyp_q b_etyp();
    etyp t[3];
    t[0] = EA; t[1] = EB; t[2] = EC;
    return t;
  endfunction

  function automatic styp_q b_styp();
    styp t[3];
    t[0] = 16'h1111; t[1] = 16'h2222; t[2] = 16'h3333;
    return t;
  endfunction

  function automatic ityp_q b_ityp();
    int t[3];
    t[0] = 30; t[1] = 31; t[2] = 32;
    return t;
  endfunction

  function automatic obj_q b_obj();
    Obj t[2];
    t[0] = new(1); t[1] = new(2);
    return t;
  endfunction

  function automatic bit ctl_fixed_to_fixed();
    urec2 s[3];
    urec2 d[3];
    s[0] = '{1,10}; s[1] = '{2,20}; s[2] = '{3,30};
    d = s;
    return (d[0].a == 1 && d[1].a == 2 && d[2].a == 3 && d[2].d == 30);
  endfunction

  function automatic bit ctl_fieldwise();              // the workaround
    urec2   s[3];
    urec2_q d;
    s[0] = '{1,10}; s[1] = '{2,20}; s[2] = '{3,30};
    d = new[3];
    foreach (s[i]) begin
      d[i].a = s[i].a;
      d[i].d = s[i].d;
    end
    return (d[0].a == 1 && d[1].a == 2 && d[2].a == 3 && d[2].d == 30);
  endfunction

  function automatic bit ctl_perelem();
    urec2   s[3];
    urec2_q d;
    s[0] = '{1,10}; s[1] = '{2,20}; s[2] = '{3,30};
    d = new[3];
    foreach (s[i]) d[i] = s[i];
    return (d[0].a == 1 && d[1].a == 2 && d[2].a == 3 && d[2].d == 30);
  endfunction

  function automatic bit ctl_dyn_to_dyn();
    urec2_q s;
    urec2_q d;
    s = new[3];
    s[0] = '{1,10}; s[1] = '{2,20}; s[2] = '{3,30};
    d = s;
    return (d[0].a == 1 && d[1].a == 2 && d[2].a == 3 && d[2].d == 30);
  endfunction

  function automatic bit ctl_int_elem();
    int    s[3];
    ityp_q d;
    s[0] = 10; s[1] = 20; s[2] = 30;
    d = s;
    return (d[0] == 10 && d[1] == 20 && d[2] == 30);
  endfunction

  class Holder;
    urec2 fx[3];
    urec2 da[];

    function void fill();
      fx[0] = '{1,10};
      fx[1] = '{2,20};
      fx[2] = '{3,30};
    endfunction

    function void whole_copy();                        // the defect
      da = fx;
    endfunction

    function void fieldwise_copy();                    // the workaround
      da = new[3];
      foreach (fx[i]) begin
        da[i].a = fx[i].a;
        da[i].d = fx[i].d;
      end
    endfunction

    function urec2_q ret_fixed();                      // the defect
      return fx;
    endfunction

    function void fill_da();                           // control source
      da = new[3];
      da[0] = '{4,40};
      da[1] = '{5,50};
      da[2] = '{6,60};
    endfunction

    function urec2_q ret_da();                         // control
      return da;
    endfunction
  endclass

endpackage

package noshadow_pkg;

  typedef struct { bit [15:0] addr; bit [31:0] data; } slot_t;
  typedef slot_t slot_q[];

  virtual class AbsNoShadow;                    // no slot_table member here
    virtual function slot_q make_slots();
    endfunction
  endclass

  virtual class AbsMap extends AbsNoShadow;
    slot_t slot_table[7];                       // fixed; nothing shadowed
    function slot_q make_slots();
    begin
      slot_table[0].addr = 0; slot_table[0].data = 32'h0A0A_0A00;
      slot_table[1].addr = 1; slot_table[1].data = 32'h0B0B_0B01;
      slot_table[2].addr = 2; slot_table[2].data = 32'h0C0C_0C02;
      slot_table[3].addr = 3; slot_table[3].data = 32'h0D0D_0D03;
      slot_table[4].addr = 4; slot_table[4].data = 32'h0E0E_0E04;
      slot_table[5].addr = 5; slot_table[5].data = 32'h0F0F_0F05;
      slot_table[6].addr = 6; slot_table[6].data = 32'h1010_1006;
      return slot_table;
    end
    endfunction
  endclass

  class Map extends AbsMap;
  endclass

endpackage

module tb;

  import cfgmap_pkg::*;
  import lane_pkg::*;
  import probe_pkg::*;
  import noshadow_pkg::*;

  int failures = 0;
  int stalls   = 0;

  function automatic bit table_ok(input reg_slot_q got, input int n,
                                  input int exp_a[], input bit [31:0] exp_d[]);
    if (got.size() != n) begin
      $display("    size=%0d expected=%0d", got.size(), n);
      return 0;
    end
    foreach (got[i])
      if (got[i].addr !== exp_a[i][15:0] || got[i].data !== exp_d[i]) begin
        $display("    [%0d] got %0d/%h expected %0d/%h",
                 i, got[i].addr, got[i].data, exp_a[i], exp_d[i]);
        return 0;
      end
    return 1;
  endfunction

  function automatic bit slot_ok(input noshadow_pkg::slot_q got, input int n,
                                 input int exp_a[], input bit [31:0] exp_d[]);
    if (got.size() != n) begin
      $display("    size=%0d expected=%0d", got.size(), n);
      return 0;
    end
    foreach (got[i])
      if (got[i].addr !== exp_a[i][15:0] || got[i].data !== exp_d[i]) begin
        $display("    [%0d] got %0d/%h expected %0d/%h",
                 i, got[i].addr, got[i].data, exp_a[i], exp_d[i]);
        return 0;
      end
    return 1;
  endfunction

  task automatic section_S1();
    LaneRegs   r;
    reg_slot_q got;
    int        exp_a[7];
    bit [31:0] exp_d[7];
    $display("== S1: register table through the production (shadowed) shape ==");
    exp_a = '{0,1,2,3,4,5,6};
    exp_d = '{32'h0000_0005, 32'h0A0A_0A01, 32'h0B0B_0B02, 32'h0C0C_0C03,
              32'h0D0D_0D04, 32'h0E0E_0E05, 32'h0F0F_0F06};
    r = new;
    void'(r.randomize());
    got = r.make_slots();
    `SVTEST_CHECK(table_ok(got, 7, exp_a, exp_d),
        "S1 make_slots() must return the table the member array holds")
    `SVTEST_CHECK(r.slot_table[6].addr === 16'd6 &&
                  r.slot_table[6].data === 32'h0F0F_0F06,
        "S1 control: the member array itself must be filled correctly")
  endtask

  task automatic section_S2();
    noshadow_pkg::Map    m;
    noshadow_pkg::slot_q got;
    int        exp_a[7];
    bit [31:0] exp_d[7];
    $display("== S2: same shape without the base-class shadow ==");
    exp_a = '{0,1,2,3,4,5,6};
    exp_d = '{32'h0A0A_0A00, 32'h0B0B_0B01, 32'h0C0C_0C02, 32'h0D0D_0D03,
              32'h0E0E_0E04, 32'h0F0F_0F05, 32'h1010_1006};
    m = new;
    got = m.make_slots();
    `SVTEST_CHECK(slot_ok(got, 7, exp_a, exp_d),
        "S2 make_slots() must return the table the member array holds")
  endtask

  task automatic section_S3();
    urec2     src[3];
    urec2     back[3];
    urec2_q   dst;
    urec2_q   presized;
    urec2_q   man;
    urec2_qq  q;
    $display("== S3: plain copies, no classes involved ==");
    src[0] = '{1,10}; src[1] = '{2,20}; src[2] = '{3,30};

    dst = src;
    `SVTEST_CHECK(dst.size() == 3 && dst[0].a == 1 && dst[0].d == 10 &&
                              dst[1].a == 2 && dst[1].d == 20 &&
                              dst[2].a == 3 && dst[2].d == 30,
        "S3a fixed -> dynamic assignment must carry the elements")

    presized = new[3];
    presized = src;
    `SVTEST_CHECK(presized[2].a == 3 && presized[2].d == 30,
        "S3b fixed -> presized dynamic must carry the elements")

    q = src;
    `SVTEST_CHECK(q.size() == 3 && q[0].a == 1 && q[2].a == 3 && q[2].d == 30,
        "S3c fixed -> queue must carry the elements")

    man = new[3];
    man[0] = '{4,40}; man[1] = '{5,50}; man[2] = '{6,60};
    back = man;
    `SVTEST_CHECK(back[0].a == 4 && back[2].a == 6 && back[2].d == 60,
        "S3d dynamic -> fixed must carry the elements")
  endtask

  task automatic section_S4();
    urec2_q   r1;
    urec1_q   r2;
    ureci_q   r3;
    urecpa_q  r4;
    urecn_q   r5;
    urec4_q   r6;
    urecstr_q r7;
    prec_q    p1;
    etyp_q    e1;
    styp_q    s1;
    ityp_q    i1;
    obj_q     o1;
    $display("== S4: element-type matrix (fixed local array -> dynamic array) ==");

    r1 = b_urec2();
    `SVTEST_CHECK(r1.size() == 3 && r1[0].a == 1 && r1[2].a == 3 && r1[2].d == 30,
        "S4a two 2-state struct fields")
    r2 = b_urec1();
    `SVTEST_CHECK(r2.size() == 3 && r2[1].v == 32'h22,
        "S4b single struct field")
    r3 = b_ureci();
    `SVTEST_CHECK(r3.size() == 3 && r3[0].a == 7 && r3[2].d == 12,
        "S4c int struct fields")
    r4 = b_urecpa();
    `SVTEST_CHECK(r4.size() == 2 && r4[1].p == 32'hBBBB_2222,
        "S4d packed-array struct field")
    r5 = b_urecn();
    `SVTEST_CHECK(r5.size() == 2 && r5[1].inner.d == 4 && r5[1].t == 8'hBB,
        "S4e nested struct field")
    r6 = b_urec4();
    `SVTEST_CHECK(r6.size() == 2 && r6[1].l == 32'h3333_3333 && r6[0].b == 8'h22,
        "S4f 4-state struct field")
    r7 = b_urecstr();
    `SVTEST_CHECK(r7.size() == 2 && r7[0].a == 8'h11 && r7[1].s == "bb",
        "S4g string struct field")

    p1 = b_prec();
    `SVTEST_CHECK(p1.size() == 2 && p1[1].x == 8'h33,
        "S4h control: packed struct element")
    e1 = b_etyp();
    `SVTEST_CHECK(e1.size() == 3 && e1[2] == EC,
        "S4i control: enum element")
    s1 = b_styp();
    `SVTEST_CHECK(s1.size() == 3 && s1[1] == 16'h2222,
        "S4j control: scalar typedef element")
    i1 = b_ityp();
    `SVTEST_CHECK(i1.size() == 3 && i1[2] == 32,
        "S4k control: int element")
    o1 = b_obj();
    `SVTEST_CHECK(o1.size() == 2 && o1[1].id == 2,
        "S4l control: class handle element")
  endtask

  task automatic section_S5();
    $display("== S5: shape controls (unpacked struct element, checked in place) ==");
    `SVTEST_CHECK(ctl_fixed_to_fixed(),
        "S5a control: fixed -> fixed must carry the elements")
    `SVTEST_CHECK(ctl_fieldwise(),
        "S5b field-by-field copy out of a fixed local array into a local dynamic array")
    `SVTEST_CHECK(ctl_perelem(),
        "S5c control: per-element whole-struct copy must carry the elements")
    `SVTEST_CHECK(ctl_dyn_to_dyn(),
        "S5d control: dynamic -> dynamic must carry the elements")
    `SVTEST_CHECK(ctl_int_elem(),
        "S5e control: int element, fixed -> dynamic must carry the elements")
  endtask

  task automatic section_S6();
    Holder  h;
    urec2_q g;
    $display("== S6: class member forms ==");
    h = new;

    h.fill();
    h.whole_copy();
    `SVTEST_CHECK(h.da.size() == 3 && h.da[0].a == 1 && h.da[1].a == 2 &&
                              h.da[2].a == 3 && h.da[2].d == 30,
        "S6a fixed class member -> dynamic member must carry the elements")

    h.fill();
    g = h.ret_fixed();
    `SVTEST_CHECK(g.size() == 3 && g[0].a == 1 && g[1].a == 2 &&
                             g[2].a == 3 && g[2].d == 30,
        "S6b fixed class member -> dynamic return must carry the elements")

    h.fill();
    h.fieldwise_copy();
    `SVTEST_CHECK(h.da.size() == 3 && h.da[0].a == 1 && h.da[1].a == 2 &&
                              h.da[2].a == 3 && h.da[2].d == 30,
        "S6c control: field-by-field copy of a fixed class member (the workaround)")

    h.fill_da();
    g = h.ret_da();
    `SVTEST_CHECK(g.size() == 3 && g[0].a == 4 && g[2].a == 6 && g[2].d == 60,
        "S6d control: dynamic class member -> dynamic return")
  endtask

  bit clk;
  bit rst_n;

  CfgBus       bus;
  LaneRegs     regs;
  SlotLoader   loader;
  StreamFeeder fdnr[3];

  bit [1:0] tick;              // free-running; ready is high 2 cycles in 3
  bit [1:0] lane_mode;         // what the unit was actually programmed with

  function automatic bit unit_feed_ok(input int s);
    case (s)
      0:       return (tick != 2'd2);
      1:       return (tick != 2'd2) && (lane_mode == 2'd1);
      2:       return (tick != 2'd2) && (lane_mode == 2'd0);
      default: return 1'b0;
    endcase
  endfunction

  always @(tick or lane_mode or rst_n) begin
    if (bus != null)
      for (int s = 0; s < 3; s++)
        fdnr[s].feed_ok = unit_feed_ok(s);
  end

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) tick <= 2'd0;
    else        tick <= (tick == 2'd2) ? 2'd0 : (tick + 2'd1);
  end

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) lane_mode <= 2'd0;
    else if (bus.slot_strobe && (bus.slot_addr == 16'd0))
      lane_mode <= bus.slot_data[1:0];
  end

  function automatic bit stream_active(input int s);
    case (s)
      0:       return 1'b1;
      1:       return (regs.R_MODE == 2'd1);
      2:       return (regs.R_MODE == 2'd0);
      default: return 1'b0;
    endcase
  endfunction

  task automatic section_S7();
    reg_slot_q tbl;
    acc_slot_q acc;
    $display("== S7: loader programs the unit from the returned table ==");
    tbl = regs.make_slots();
    acc = regs.make_access();
    loader.run(tbl, acc);
    `SVTEST_CHECK(lane_mode == regs.R_MODE,
        "S7 the unit must be programmed with the mode the table carries")
    `SVTEST_CHECK(acc.size() == NSLOT && acc[0].rd == 32'hFFFF_FFFF,
        "S7 the access table must survive the same copy")
  endtask

  task automatic section_S8();
    $display("== S8: feed the streams the testbench started ==");
    for (int s = 0; s < 3; s++) begin
      if (stream_active(s)) begin
        fdnr[s].run(4, 3);
        if (fdnr[s].stalled) begin
          stalls++;
          $display("TEST_INCOMPLETE section=S8 stream=%0d reason=feed_ok never rose within %0d cycles (lane_mode=%0d R_MODE=%0d)",
                   s, STALL_K, lane_mode, regs.R_MODE);
        end else begin
          $display("    stream %0d drained %0d beats", s, fdnr[s].beats_sent);
        end
      end else begin
        $display("    stream %0d not started (mode %0d)", s, regs.R_MODE);
      end
    end
    `SVTEST_CHECK(stalls == 0, "S8 every started stream must complete")
  endtask

  initial begin
    bus    = new;
    regs   = new;
    loader = new;
    loader.bus     = bus;
    for (int s = 0; s < 3; s++) begin
      fdnr[s]           = new;
      fdnr[s].bus       = bus;
      fdnr[s].stream_id = s;
    end
    clk       = 1'b0;
    rst_n     = 1'b0;
    bus.clk   = 1'b0;
    bus.rst_n = 1'b0;
    tick      = 2'd0;
    lane_mode = 2'd0;

    void'(regs.randomize());

    section_S1();
    section_S2();
    section_S3();
    section_S4();
    section_S5();
    section_S6();

    rst_n = 1'b1;                       // release reset before programming
    section_S7();
    section_S8();

    `SVTEST_PASSFAIL
    $finish;
  end

  initial begin
    #1;
    forever #5 clk = ~clk;
  end

  always @(clk)   if (bus != null) bus.clk   = clk;
  always @(rst_n) if (bus != null) bus.rst_n = rst_n;

endmodule
"##,
    );
    expect(&msgs, &["TEST_PASS", "    stream 1 drained 12 beats"]);
}

/// Module-scope and task-local copies fixed to dynamic, fixed to queue,
/// dynamic to fixed and queue to fixed.
#[test]
fn fixed_dynamic_and_queue_copies() {
    let msgs = lines(
        r##"
module top;
  typedef struct { bit [15:0] a; bit [31:0] d; } urec2;
  typedef urec2 urec2_q[];
  typedef urec2 urec2_qq[$];
  urec2 msrc[3];
  urec2_q mdst;
  urec2_qq mq;
  urec2 mback[3];
  task automatic t();
    urec2 src[3]; urec2_q dst; urec2_qq q; urec2 back[3]; urec2_q man;
    src[0] = '{1,10}; src[1] = '{2,20}; src[2] = '{3,30};
    dst = src;
    $display("task fixed->dyn  size=%0d %0d %0d %0d", dst.size(), dst[0].a, dst[2].a, dst[2].d);
    q = src;
    $display("task fixed->q    size=%0d %0d %0d %0d", q.size(), q[0].a, q[2].a, q[2].d);
    man = new[3]; man[0] = '{4,40}; man[1] = '{5,50}; man[2] = '{6,60};
    back = man;
    $display("task dyn->fixed  %0d %0d %0d", back[0].a, back[2].a, back[2].d);
    back = q;
    $display("task q->fixed    %0d %0d %0d", back[0].a, back[2].a, back[2].d);
  endtask
  initial begin
    msrc[0] = '{1,10}; msrc[1] = '{2,20}; msrc[2] = '{3,30};
    mdst = msrc;
    $display("mod fixed->dyn   size=%0d %0d %0d %0d", mdst.size(), mdst[0].a, mdst[2].a, mdst[2].d);
    mq = msrc;
    $display("mod fixed->q     size=%0d %0d %0d %0d", mq.size(), mq[0].a, mq[2].a, mq[2].d);
    mdst[1].a = 77;
    mback = mdst;
    $display("mod dyn->fixed   %0d %0d %0d", mback[0].a, mback[1].a, mback[2].d);
    t();
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "mod fixed->dyn   size=3 1 3 30",
            "mod fixed->q     size=3 1 3 30",
            "mod dyn->fixed   1 77 30",
            "task fixed->dyn  size=3 1 3 30",
            "task fixed->q    size=3 1 3 30",
            "task dyn->fixed  4 6 60",
            "task q->fixed    1 3 30",
        ],
    );
}

/// Class properties: fixed to dynamic and queue inside a method, the fixed
/// member returned as a dynamic array, dynamic to fixed, and copies between a
/// property and a variable in both directions.
#[test]
fn class_property_copies() {
    let msgs = lines(
        r##"
module top;
  typedef struct { bit [15:0] a; bit [31:0] d; } urec2;
  typedef urec2 urec2_q[];
  typedef urec2 urec2_qq[$];
  class Holder;
    urec2 fx[3];
    urec2 da[];
    urec2 qq[$];
    function void fill(); fx[0] = '{1,10}; fx[1] = '{2,20}; fx[2] = '{3,30}; endfunction
    function void whole_copy(); da = fx; endfunction
    function void q_copy(); qq = fx; endfunction
    function void back_copy(); fx = da; endfunction
    function urec2_q ret_fixed(); return fx; endfunction
  endclass
  initial begin
    Holder h = new;
    urec2_q g;
    urec2 loc[3];
    h.fill();
    h.whole_copy();
    $display("S6a da=fx size=%0d %0d %0d %0d", h.da.size(), h.da[0].a, h.da[2].a, h.da[2].d);
    h.q_copy();
    $display("qq=fx size=%0d %0d %0d %0d", h.qq.size(), h.qq[0].a, h.qq[2].a, h.qq[2].d);
    g = h.ret_fixed();
    $display("S6b ret size=%0d %0d %0d %0d", g.size(), g[0].a, g[2].a, g[2].d);
    h.da[1].a = 99;
    h.back_copy();
    $display("fx=da %0d %0d", h.fx[1].a, h.fx[2].d);
    g = h.fx;
    $display("g=h.fx size=%0d %0d %0d", g.size(), g[1].a, g[2].d);
    loc = h.da;
    $display("loc=h.da %0d %0d", loc[1].a, loc[2].d);
    h.da = loc;
    $display("h.da=loc size=%0d %0d", h.da.size(), h.da[1].a);
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "S6a da=fx size=3 1 3 30",
            "qq=fx size=3 1 3 30",
            "S6b ret size=3 1 3 30",
            "fx=da 99 30",
            "g=h.fx size=3 99 30",
            "loc=h.da 99 30",
            "h.da=loc size=3 99",
        ],
    );
}

/// A local queue or dynamic array of structs handed out of a function
/// frame, assigned and printed with `%p`.
#[test]
fn returned_local_struct_collections() {
    let msgs = lines(
        r##"
module top;
  typedef struct { int a; byte b; } s_t;
  typedef s_t sq_t[$];
  typedef s_t sd_t[];
  function automatic sq_t sqf(byte l); sq_t r; r.push_back('{1, 8'd5}); r.push_back('{2, l}); return r; endfunction
  function automatic sd_t sdf(byte l); sd_t r = new[2]; r[0] = '{1, 8'd5}; r[1] = '{2, l}; return r; endfunction
  initial begin
    sq_t q; sd_t d;
    q = sqf(6);
    $display("q size=%0d %0d %0d %0d", q.size(), q[0].a, q[1].a, q[1].b);
    d = sdf(7);
    $display("d size=%0d %0d %0d %0d", d.size(), d[0].a, d[1].a, d[1].b);
    $display("%p", sqf(8));
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "q size=2 1 2 6",
            "d size=2 1 2 7",
            "'{'{a:1, b:5}, '{a:2, b:8}}",
        ],
    );
}
