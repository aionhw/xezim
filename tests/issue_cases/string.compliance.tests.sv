// ============================================================================
// Testbench: sv_string_lrm_compliance_tb
// LRM Section: IEEE Std 1800-2017, Section 6.16 (Strings)
// Description: Exhaustive self-checking test for all string subsections.
// ============================================================================

module sv_string_lrm_compliance_tb;

  bit [15:0] error_count;

  // Helper macro for strict string validation
  `define CHECK_STR(test_name, actual, expected) \
if ((actual) != (expected)) begin \
$display("[ERROR] %s failed! Expected: \"%s\", Got: \"%s\"", test_name, expected, actual); \
error_count++; \
end else begin \
$display("[PASS] %s passed.", test_name); \
end

  // Helper macro for integer/byte results from string methods
  `define CHECK_VAL(test_name, actual, expected) \
if ((actual) !== (expected)) begin \
$display("[ERROR] %s failed! Expected: %0d, Got: %0d", test_name, expected, actual); \
error_count++; \
end else begin \
$display("[PASS] %s passed.", test_name); \
end

  // Helper macro for boolean expressions mapped to integer logic
  `define CHECK_BOOL(t_name, act, exp) `CHECK_VAL(t_name, int'(act), int'(exp))

  initial begin
    $display("==========================================================");
    $display("Starting SystemVerilog LRM Section 6.16 Compliance Test");
    $display("==========================================================");

    // --------------------------------------------------------------------
    // 6.16.1: String Operators (Equality, Relational, Concatenation, Replication)
    // --------------------------------------------------------------------
    begin
      string concat_res;
      string repl_res;
      string s1, s2, s3;
      s1 = "apple";
      s2 = "banana";
      s3 = "apple";
                                                                                                                                                                                                                                                                              
      $display("\n--- Testing 6.16.1: String Operators ---");

      `CHECK_BOOL("Equality (==)", (s1 == s3), 1'b1)
      `CHECK_BOOL("Inequality (!=)", (s1 != s2), 1'b1)
      `CHECK_BOOL("Comparison (<)", (s1 < s2), 1'b1)  // "apple" comes before "banana"
      `CHECK_BOOL("Comparison (>)", (s2 > s1), 1'b1)

      concat_res = {s1, " ", s2};
      `CHECK_STR("Concatenation ({s1, s2})", concat_res, "apple banana")

      repl_res = {3{"A"}};
      `CHECK_STR("Replication ({3{\"A\"}})", repl_res, "AAA")

      // Empty replication behavior (LRM: results in empty string "")
      repl_res = {0{"A"}};
      `CHECK_STR("Zero Replication ({0{\"A\"}})", repl_res, "")
    end

    // --------------------------------------------------------------------
    // 6.16.2: String Methods - len()
    // --------------------------------------------------------------------
    begin
      string s, emptys;
      s = "Hello World";
      emptys = "";
      $display("\n--- Testing 6.16.2: len() method ---");
      `CHECK_VAL("len() on active string", s.len(), 11)
      `CHECK_VAL("len() on empty string", emptys.len(), 0)
    end

    // --------------------------------------------------------------------
    // 6.16.3: String Methods - putc()
    // --------------------------------------------------------------------
    begin
      string s;
      s = "Clear";
      $display("\n--- Testing 6.16.3: putc() method ---");

      s.putc(0, "B");  // Mutate index 0
      `CHECK_STR("putc() legal modification", s, "Blear")

      // LRM: Out of bounds index or empty string mutation shall not change the string
      s.putc(99, "X");
      `CHECK_STR("putc() out-of-bounds mutation (ignored)", s, "Blear")
      s.putc(-1, "Z");
      `CHECK_STR("putc() negative index mutation (ignored)", s, "Blear")
    end

    // --------------------------------------------------------------------
    // 6.16.4: String Methods - getc()
    // --------------------------------------------------------------------
    begin
      string s;
      s = "Verify";
      $display("\n--- Testing 6.16.4: getc() method ---");
      `CHECK_VAL("getc() legal index 0", s.getc(0), 86)  // ASCII 'V'
      `CHECK_VAL("getc() legal index 1", s.getc(1), 101)  // ASCII 'e'

      // LRM: Out of bounds index returns 0
      `CHECK_VAL("getc() out-of-bounds positive", s.getc(20), 0)
      `CHECK_VAL("getc() out-of-bounds negative", s.getc(-5), 0)
    end

    // --------------------------------------------------------------------
    // 6.16.5: String Methods - toupper() and tolower()
    // --------------------------------------------------------------------
    begin
      string lower, upper;
      lower = "mixed123";
      upper = "MIXED123";
      
      $display("\n--- Testing 6.16.5: toupper() and tolower() methods ---");
      `CHECK_STR("toupper() conversions", lower.toupper(), "MIXED123")
      `CHECK_STR("tolower() conversions", upper.tolower(), "mixed123")

      // LRM: String itself remains unchanged unless reassigned
      `CHECK_STR("Original immutable lowercase check", lower, "mixed123")
    end

    // --------------------------------------------------------------------
    // 6.16.6: String Methods - compare() and icompare()
    // --------------------------------------------------------------------
    begin
      string s1, s2;
      s1 = "SystemVerilog";
      s2 = "systemverilog";
      $display("\n--- Testing 6.16.6: compare() and icompare() methods ---");

      // compare() is case-sensitive (ASCII evaluation)
      `CHECK_VAL("compare() matching", s1.compare(s1), 0)
      if (s1.compare(s2) == 0) begin
        $display("[ERROR] compare() was incorrectly case-insensitive.");
        error_count++;
      end else begin
        $display("[PASS] compare() case-sensitivity validated.");
      end

      // icompare() is case-insensitive
      `CHECK_VAL("icompare() matching mixed-case", s1.icompare(s2), 0)
    end

    // --------------------------------------------------------------------
    // 6.16.7: String Methods - substr()
    // --------------------------------------------------------------------
    begin
      string s;
      s = "Compliance";
      $display("\n--- Testing 6.16.7: substr() method ---");

      `CHECK_STR("substr() valid mid-range", s.substr(2, 5), "mpli")
      `CHECK_STR("substr() single char range", s.substr(0, 0), "C")

      // LRM: If i > j or indices are out of bounds, returns empty string ""
      `CHECK_STR("substr() reversed indices (i > j)", s.substr(5, 2), "")
      `CHECK_STR("substr() out of bounds high", s.substr(2, 50), "")
      `CHECK_STR("substr() out of bounds low", s.substr(-1, 3), "")
    end

    // --------------------------------------------------------------------
    // 6.16.8: String Methods - atoi(), atohex(), atooct(), atobin()
    // --------------------------------------------------------------------
    begin
      string s_dec, s_hex, s_oct, s_bin, s_invalid;
      s_dec = "12345";
      s_hex = "7f";
      s_oct = "77";
      s_bin = "1011";
      s_invalid = "abc";

      $display("\n--- Testing 6.16.8: Conversion methods (ato*) ---");
      `CHECK_VAL("atoi() decimal conversion", s_dec.atoi(), 12345)
      `CHECK_VAL("atohex() hex conversion", s_hex.atohex(), 127)
      `CHECK_VAL("atooct() octal conversion", s_oct.atooct(), 63)
      `CHECK_VAL("atobin() binary conversion", s_bin.atobin(), 11)

      // LRM: Non-digit characters return 0
      `CHECK_VAL("atoi() processing non-digits", s_invalid.atoi(), 0)
    end

    // --------------------------------------------------------------------
    // 6.16.9: String Methods - itoa(), hextoa(), octtoa(), bintoa()
    // --------------------------------------------------------------------
    begin
      string s;
      $display("\n--- Testing 6.16.9: Format conversion methods (*toa) ---");

      s.itoa(9876);
      `CHECK_STR("itoa() integer representation", s, "9876")

      s.hextoa(255);
      `CHECK_STR("hextoa() hex representation", s, "ff")

      s.octtoa(64);
      `CHECK_STR("octtoa() octal representation", s, "100")

      s.bintoa(5);
      `CHECK_STR("bintoa() binary representation", s, "101")
    end

    // --------------------------------------------------------------------
    // 6.16.10: String Methods - realtoa(), atoreal()
    // --------------------------------------------------------------------
    begin
      string s;
      real   r_out;
      real   r;

      r = 3.14159;

      $display("\n--- Testing 6.16.10: Real conversion methods ---");

      s.realtoa(r);
      // Check basic prefix string matching to account for varying simulator rounding behavior
      if (s.substr(0, 3) != "3.14") begin
        $display("[ERROR] realtoa() conversion failed! Got string: %s", s);
        error_count++;
      end else begin
        $display("[PASS] realtoa() validated.");
      end

      s = "2.71828";
      r_out = s.atoreal();
      if (r_out < 2.718 || r_out > 2.719) begin
        $display("[ERROR] atoreal() conversion failed! Got real: %f", r_out);
        error_count++;
      end else begin
        $display("[PASS] atoreal() validated.");
      end
    end

    // --------------------------------------------------------------------
    // Final Compliance Summary
    // -------------------------------------------------------------------
    $display("\n==========================================================");
    if (error_count == 0) begin
      $display("TEST PASSED: Simulator string engine is fully compliant with Section 6.16");
    end else begin
      $display("TEST FAILED: %0d string LRM compliance errors detected.", error_count);
    end
    $display("==========================================================");
    $finish;
  end
endmodule
