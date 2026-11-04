// ============================================================================
// MWE — xezim vs xrun: writes through a select to a NON-EXISTENT
// associative-array element are mishandled in xezim.
//
// A register shadow-model style builder creates an int-indexed associative
// array of 32-bit words using ONLY part-select writes to initially-absent
// elements (`words[addr][msb:lsb] = value`), inside a class method, returned
// by value. Under xezim the elements are never created, so a later
// `exists()` probe always fails and the reader returns a sentinel value.
//
// Expected (LRM 1800-2017 §7.8/§10.8.1): `w[k][msb:lsb] = v` reads the
// absent element as its default (0 for 2-state, X for 4-state), replaces
// the selected bits, and writes the element back — CREATING it.
// ============================================================================

`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
  int failures = 0;

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end

`endif

module test_assoc_partselect;

  `SVTEST_INIT

  // Register shadow-model style element type: 32-bit words keyed by int.
  typedef bit [31:0] word_map_t [int];

  // ---- function return-by-value link (the builder returns the array)
  function automatic word_map_t build_whole();
    word_map_t w;
    w[200] = 32'hcafef00d;              // whole-element write only
    return w;
  endfunction

  function automatic word_map_t build_part();
    word_map_t w;
    w[100][20:0] = 21'h123456;          // part-select write only
    return w;
  endfunction

  // ---- part-writes-present factor: part + whole writes in one function
  function automatic word_map_t build_mixed();
    word_map_t w;
    w[100][20:0] = 21'h123456;          // part-select write (absent)
    w[200]       = 32'hcafef00d;        // whole-element write
    return w;
  endfunction

  // ---- class-method factor (the production builder is a class method)
  virtual class Base;
    virtual function word_map_t build();
      word_map_t w;
      return w;
    endfunction
  endclass

  class Derived extends Base;
    virtual function word_map_t build();
      word_map_t w;
      w[200] = 32'hcafef00d;            // whole-element write only
      return w;
    endfunction
  endclass

  initial begin
    // A1 — CORE BUG: part-select write to an ABSENT assoc element
    begin
      word_map_t w;
      w[100][20:0] = 21'h123456;
      `SVTEST_CHECK(w.exists(100) && w[100] == 32'h00123456 && w.size() == 1,
                    "A1: part-select write to absent assoc element dropped (want EXISTS val=00123456 size=1)")
    end

    // A2 — control: whole-element write (known-good in xezim)
    begin
      word_map_t w;
      w[100] = 32'hcafef00d;
      `SVTEST_CHECK(w.exists(100) && w[100] == 32'hcafef00d && w.size() == 1,
                    "A2: whole-element write to assoc element failed")
    end

    // A3 — cousin: part-select write to an EXISTING element (RMW must
    //      preserve unselected bits)
    begin
      word_map_t w;
      w[100] = 32'hffffffff;
      w[100][20:0] = 21'h0;
      `SVTEST_CHECK(w.exists(100) && w[100] == 32'hffe00000,
                    "A3: part-select RMW on existing assoc element wrong (want ffe00000)")
    end

    // A4 — cousin: single-BIT-select write to an ABSENT element
    begin
      word_map_t w;
      w[100][3] = 1'b1;
      `SVTEST_CHECK(w.exists(100) && w[100] == 32'h00000008,
                    "A4: bit-select write to absent assoc element: element created but bit dropped (want 00000008)")
    end

    // A5 — cousin: indexed part-select (+:) write to an ABSENT element
    begin
      word_map_t w;
      w[100][8 +: 8] = 8'hab;
      `SVTEST_CHECK(w.exists(100) && w[100] == 32'h0000ab00,
                    "A5: indexed part-select (+:) write to absent assoc element dropped (want 0000ab00)")
    end

    // A6 — cousin: part-select READ of an ABSENT element (must yield the
    //      default and must NOT create the element)
    begin
      word_map_t w;
      bit [20:0] rd;
      rd = w[999][20:0];
      `SVTEST_CHECK(!w.exists(999) && rd == 21'h0 && w.size() == 0,
                    "A6: part-select read of absent assoc element must not create it (rd must be 0)")
    end

    // A7 — cousin: 4-state element type (unwritten bits must be X)
    begin
      logic [31:0] la [int];
      la[100][20:0] = 21'h123456;
      `SVTEST_CHECK(la.exists(100) && la[100][20:0] === 21'h123456 && la[100][31:21] === 11'bx,
                    "A7: part-select write to absent 4-state assoc element dropped (want low bits 123456, high bits X)")
    end

    // A8 — cousin: dynamic-array element (exists after new[], zeroed)
    begin
      bit [31:0] da [];
      da = new[2];
      da[0][20:0] = 21'h123456;
      `SVTEST_CHECK(da.size() == 2 && da[0] == 32'h00123456 && da[1] == 32'h00000000,
                    "A8: part-select write to dynamic-array element wrong (want 00123456 / 00000000)")
    end

    // A9 — cousin: queue element RMW
    begin
      bit [31:0] q [$];
      q.push_back(32'hffffffff);
      q[0][20:0] = 21'h0;
      `SVTEST_CHECK(q.size() == 1 && q[0] == 32'hffe00000,
                    "A9: part-select RMW on queue element wrong (want ffe00000)")
    end

    // B1 — plain function return-by-value, whole-element write only
    begin
      word_map_t w;
      w = build_whole();
      `SVTEST_CHECK(w.exists(200) && w[200] == 32'hcafef00d && w.size() == 1,
                    "B1: assoc array returned from plain function lost whole-element write")
    end

    // B2 — plain function return-by-value, part-select write only
    begin
      word_map_t w;
      w = build_part();
      `SVTEST_CHECK(w.exists(100) && w[100] == 32'h00123456 && w.size() == 1,
                    "B2: assoc array returned from plain function lost part-select write")
    end

    // B3 — class VIRTUAL method, whole-element write only
    begin
      Derived d;
      Base    b;
      word_map_t w;
      d = new();
      b = d;
      w = b.build();
      `SVTEST_CHECK(w.exists(200) && w[200] == 32'hcafef00d && w.size() == 1,
                    "B3: assoc array returned from class virtual method lost whole-element write")
    end

    // B4 — plain function, part-select + whole writes both present
    begin
      word_map_t w;
      w = build_mixed();
      `SVTEST_CHECK(w.exists(200) && w[200] == 32'hcafef00d && w.exists(100) && w[100] == 32'h00123456 && w.size() == 2,
                    "B4: mixed part+whole writes: part-select element lost from returned assoc array")
    end

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule

