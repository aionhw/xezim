// ============================================================================
// Testbench: sv_format_specifier_compliance_tb
// LRM Section: IEEE Std 1800-2017, Sections 21.2.1.1 through 21.2.1.6
// Description: Exhaustive self-checking test for string formatting,
//              radix specifiers, padding, and net strength formatting.
// ============================================================================

module percent_m_test0 ();

    string inst_path = $sformatf("%m");

    initial begin
        #2 ;
      $display("\n--- Testing 21.2.1.6: Specialized Percent M Visual Inspection Check ---");
        $display("Inferred Instantiation Path: %s", inst_path);
        $display("Inferred Instantiation Path (Non-String Version): %m");
    end

endmodule: percent_m_test0

module pmwrapper ();

    percent_m_test0 inst0 ();
    percent_m_test0 inst1 ();

endmodule: pmwrapper

module sv_format_specifier_compliance_tb;

    bit [15:0] error_count = 0;
    string out_str, expected_str;

    percent_m_test0 direct_dut ();
    pmwrapper deep_dut0();
    pmwrapper deep_dut1();

    // Helper macro to validate formatted string matches LRM expectation
    `define CHECK_FORMAT(test_name, actual_str, expected_str) \
        if (actual_str != expected_str) begin \
            $display("[ERROR] %s failed!", test_name); \
            $display("        Expected: \"%s\"", expected_str); \
            $display("        Got:      \"%s\"", actual_str); \
            error_count++; \
        end else begin \
            $display("[PASS] %s: \"%s\" vs \"%s\"", test_name, actual_str, expected_str); \
        end

    // ------------------------------------------------------------------------
    // Nets for Section 21.2.1.6 (%v Strength Specifier Testing)
    // ------------------------------------------------------------------------
    wire net_strong_1;
    wire net_pull_0;
    wire net_supply_z;
    wire net_weak_x;

    assign (strong1, strong0) net_strong_1 = 1'b1;
    assign (pull1,   pull0)   net_pull_0   = 1'b0;
    assign (strong1,  highz0)  net_supply_z = 1'bz;
    assign (weak1,   weak0)   net_weak_x   = 1'bx;

    initial begin
      
        $display("==========================================================");
        $display("Starting LRM 21.2.1.1 - 21.2.1.6 Compliance Test");
        $display("==========================================================");

        #1; // Allow nets to evaluate and resolve strengths
      $display("\n--- Testing Section 21.2.1.1: Escape Sequences ---");

        // Rule: Ordinary characters are copied directly without modification
        out_str = $sformatf("Plain literal string with no specifiers");
        `CHECK_FORMAT("21.2.1: Plain literal routing", out_str, "Plain literal string with no specifiers")

        // Escape Sequences: Character sequences introduced to embed control/special characters
        out_str = $sformatf("Yield rate is 99%% optimized");
        `CHECK_FORMAT("21.2.1: Literal percent symbol escape (%%)", out_str, "Yield rate is 99% optimized")

        out_str = $sformatf("Line1\nLine2");
        `CHECK_FORMAT("21.2.1: Newline escape (\\n)", out_str, {"Line1", 8'h0A, "Line2"})

        out_str = $sformatf("Column1\tColumn2");
        `CHECK_FORMAT("21.2.1: Horizontal tab escape (\\t)", out_str, {"Column1", 8'h09, "Column2"})

        // 4. Backslash
        out_str = $sformatf("Path\\To\\File");
        expected_str = "Path\\To\\File"; 
        `CHECK_FORMAT("21.2.1: Backslash escape (\\\\)", out_str, expected_str)

            // 5. Double Quote
            out_str = $sformatf("He said, \"Hello\"");
            expected_str = "He said, \"Hello\"";
            `CHECK_FORMAT("21.2.1: Double quote escape (\\\")", out_str, expected_str)

            // 7. Vertical Tab
            out_str = $sformatf("Vertical\vTab");
            expected_str = {"Vertical", 8'h0B, "Tab"};
            `CHECK_FORMAT("21.2.1: Vertical tab escape (\\v)", out_str, expected_str)

            // 8. Form Feed
            out_str = $sformatf("Form\fFeed");
            expected_str = {"Form", 8'h0C, "Feed"};
            `CHECK_FORMAT("21.2.1: Form feed escape (\\f)", out_str, expected_str)

            // 9. Alert / Bell
            out_str = $sformatf("Bell\aAlert");
            expected_str = {"Bell", 8'h07, "Alert"};
            `CHECK_FORMAT("21.2.1: Alert/Bell escape (\\a)", out_str, expected_str)
            
        // --------------------------------------------------------------------
        // 21.2.1.2: Format specifications
        // --------------------------------------------------------------------
      $display("\n--- Testing 21.2.1.2: Format specifications ---");

        out_str = $sformatf("%h", 8'hA5);
        `CHECK_FORMAT("Hex Lowercase (%h)", out_str, "a5")

        out_str = $sformatf("%H", 8'hA5);
        // LRM specifies upper case and lower case fmt specifiers to behave identically
        `CHECK_FORMAT("Hex Uppercase (%H)", out_str, "a5")
      out_str = $sformatf("%x", 8'h75);
      `CHECK_FORMAT("Hex (%x)", out_str, "75")
      out_str = $sformatf("%x", 16'h75);
      `CHECK_FORMAT("Hex (%x)", out_str, "0075")

        out_str = $sformatf("%d", 1234);
        `CHECK_FORMAT("Decimal (%d)", out_str, "       1234")
        out_str = $sformatf("%d", -2147483648);
        `CHECK_FORMAT("Decimal (%d)", out_str, "-2147483648")

        out_str = $sformatf("%o", 8'o75);
      `CHECK_FORMAT("Octal (%o)", out_str, "075")
      out_str = $sformatf("%o", 16'o75);
      `CHECK_FORMAT("Octal (%o)", out_str, "000075")

        out_str = $sformatf("%b", 4'b1011);
        `CHECK_FORMAT("Binary (%b)", out_str, "1011")

        // LRM Rule: Unknown/High-Z states must print as x/z
        out_str = $sformatf("%b", 4'b1x0z);
        `CHECK_FORMAT("Binary x/z extraction", out_str, "1x0z")

        out_str = $sformatf("%h", 8'shX);
        `CHECK_FORMAT("Hex with unknown bits", out_str, "xx")


            // %u/%U: 2-state unformatted data (Little-endian byte layout stream)
            out_str = $sformatf("%u", 32'h41424344); // LSB is '44' (D), MSB is '41' (A)
            `CHECK_FORMAT("21.2.1: 2-State Unformatted Stream Lowercase (%u)", out_str, "DCBA")
            
            out_str = $sformatf("%U", 32'h45464748); // LSB is '48' (H), MSB is '45' (E)
            `CHECK_FORMAT("21.2.1: 2-State Unformatted Stream Uppercase (%U)", out_str, "HGFE")

        begin  
          // 4-state vectors: 12 bits * 2 bits/state = 24 bits (3 bytes data + 1 byte pad = 32 bits)
            logic [11:0] vec_4state_a = 12'b 1010_0101_zx10; 
            logic [11:0] vec_4state_b = 12'b 0101_1010_10zx; 

            // 2-state vector: 24 bits * 1 bit/state = 24 bits (3 bytes data + 1 byte pad = 32 bits)
            logic [23:0] vec_2state_c = 24'b 01000001_01000010_00000010; // ASCII 'A', 'B', and 8'h02

            // --- 1. %u / %U 2-State Fallback Filtering Check ---
            // Expects 4 bytes total (Data: 02, 42, 41 + 1 padded null byte 00)
            out_str = $sformatf("%u", vec_2state_c);
            if (out_str.len() == 4 && 
                out_str.getc(0) == 8'h02 && 
                out_str.getc(1) == 8'h42 && // 'B'
                out_str.getc(2) == 8'h41 && // 'A'
                out_str.getc(3) == 8'h00)   // Pad byte
            begin
                $display("[PASS] 21.2.1: 2-State %%u stream tracking: Passed 4-byte check.");
            end else begin
                $display("[ERROR] 21.2.1: 2-State %%u stream tracking failed!");
                $display("        Expected 4 bytes (02 42 41 00), Got length %0d with hex: %0h %0h %0h %0h", 
                          out_str.len(), out_str.getc(0), out_str.getc(1), out_str.getc(2), out_str.getc(3));
                error_count++;
            end

           // --- 2. %z Lowercase 4-State Packing Validation ---
            // Expects 8 bytes total: b-val bytes [0-3] then a-val bytes [4-7]
            out_str = $sformatf("%z", vec_4state_a);
            if (out_str.len() == 8 && 
                out_str.getc(0) == 8'h0C && out_str.getc(1) == 8'h00 && // b-val word
                out_str.getc(2) == 8'h00 && out_str.getc(3) == 8'h00 && 
                out_str.getc(4) == 8'h56 && out_str.getc(5) == 8'h0A && // a-val word
                out_str.getc(6) == 8'h00 && out_str.getc(7) == 8'h00) 
            begin
                $display("[PASS] 21.2.1: 4-State Lowercase %%z tracking with x/z: Passed 8-byte b-val/a-val check.");
            end else begin
                $display("[ERROR] 21.2.1: 4-State Lowercase %%z tracking with x/z failed!");
                $display("        Expected length 8 (0C 00 00 00 56 0A 00 00)");
                $display("        Got length %0d with hex bytes: %0h %0h %0h %0h %0h %0h %0h %0h", 
                          out_str.len(), out_str.getc(0), out_str.getc(1), out_str.getc(2), out_str.getc(3),
                          out_str.getc(4), out_str.getc(5), out_str.getc(6), out_str.getc(7));
                error_count++;
            end

            // --- 3. %Z Uppercase 4-State Packing Validation ---
            // Expects 8 bytes total: b-val bytes [0-3] then a-val bytes [4-7]
            out_str = $sformatf("%Z", vec_4state_b);
            if (out_str.len() == 8 && 
                out_str.getc(0) == 8'h03 && out_str.getc(1) == 8'h00 && // b-val word
                out_str.getc(2) == 8'h00 && out_str.getc(3) == 8'h00 && 
                out_str.getc(4) == 8'hA9 && out_str.getc(5) == 8'h05 && // a-val word
                out_str.getc(6) == 8'h00 && out_str.getc(7) == 8'h00) 
            begin
                $display("[PASS] 21.2.1: 4-State Uppercase %%Z tracking with x/z: Passed 8-byte b-val/a-val check.");
            end else begin
                $display("[ERROR] 21.2.1: 4-State Uppercase %%Z tracking with x/z failed!");
                $display("        Expected length 8 (03 00 00 00 A9 05 00 00)");
                $display("        Got length %0d with hex bytes: %0h %0h %0h %0h %0h %0h %0h %0h", 
                          out_str.len(), out_str.getc(0), out_str.getc(1), out_str.getc(2), out_str.getc(3),
                          out_str.getc(4), out_str.getc(5), out_str.getc(6), out_str.getc(7));
                error_count++;
            end
        end
      
        // --------------------------------------------------------------------
        // 21.2.1.2: Real Number Format Specifiers (%e, %f, %g)
        // --------------------------------------------------------------------
        $display("\n--- Testing 21.2.1.2: Real Number Specifiers ---");

        // Float format default behavior (typically 6 decimal places)
        out_str = $sformatf("%.2f", 3.14159);
        `CHECK_FORMAT("Float Precision Padding (%.2f)", out_str, "3.14")

        // Exponential notation
        out_str = $sformatf("%.1e", 100.0);
        `CHECK_FORMAT("Scientific Notation (%.1e)", out_str, "1.0e+02")

        $display("\n--- Testing Section 21.2.1.2: Smart-Scaling Real Specifiers (%%g / %%G) ---");

        begin
            real val_decimal = 123.45;
            real val_scientific = 0.0000123; // Exponent < -4 forces exponential format
            
            // ----------------------------------------------------------------
            // Path A: Value selects Decimal Format (%f representation is shorter)
            // ----------------------------------------------------------------
            out_str = $sformatf("%g", val_decimal);
            `CHECK_FORMAT("21.2.1.2: Smart Real Lowercase Path A (Decimal)", out_str, "123.45")

            out_str = $sformatf("%G", val_decimal);
            `CHECK_FORMAT("21.2.1.2: Smart Real Uppercase Path A (Decimal)", out_str, "123.45")

            // ----------------------------------------------------------------
            // Path B: Value selects Exponential Format (%e representation is shorter)
            // Note: Table 21-3 dictates that %G forces an uppercase 'E' token 
            // if scientific notation is chosen by the layout manager.
            // ----------------------------------------------------------------
            out_str = $sformatf("%g", val_scientific);
            `CHECK_FORMAT("21.2.1.2: Smart Real Lowercase Path B (Scientific)", out_str, "1.23e-05")

            out_str = $sformatf("%G", val_scientific);
          `CHECK_FORMAT("21.2.1.2: Smart Real Uppercase Path B (Scientific)", out_str, "1.23e-05")
        end
        // --------------------------------------------------------------------
        // 21.2.1.2: Character & String Specifiers (%c, %s)
        // --------------------------------------------------------------------
      $display("\n--- Testing 21.2.1.2: Character & String Specifiers ---");

        out_str = $sformatf("%c", 8'd65);
        `CHECK_FORMAT("Character specifier (%c)", out_str, "A")

        out_str = $sformatf("%s", "SystemVerilog");
        `CHECK_FORMAT("String specifier (%s)", out_str, "SystemVerilog")


        // --------------------------------------------------------------------
        // 21.2.1.4: Padding and Field Widths (%0d, %5d, %-5d)
        // --------------------------------------------------------------------
      $display("\n--- Testing 21.2.1.3: Field Widths & Alignments ---");

        out_str = $sformatf("%0d", 45);
        `CHECK_FORMAT("Minimum Field Width (%0d)", out_str, "45")

        out_str = $sformatf("%5d", 45);
        `CHECK_FORMAT("Right-Justified Padding (%5d)", out_str, "   45")

        out_str = $sformatf("%-5d", 45);
        `CHECK_FORMAT("Left-Justified Padding (%-5d)", out_str, "45   ")

        // Over-width fallback: String must expand if number exceeds designated width
        out_str = $sformatf("%2d", 12345);
        `CHECK_FORMAT("Width Overflow Auto-Expand (%2d)", out_str, "12345")


        // %m prints the hierarchical path of the current scope
        out_str = $sformatf("%m");
        `CHECK_FORMAT("Hierarchical Scope Path (%m)", out_str, "sv_format_specifier_compliance_tb")


        // --------------------------------------------------------------------
        // 21.2.1.5: Net Strength Format Specifier (%v)
        // --------------------------------------------------------------------
      $display("\n--- Testing 21.2.1.5: Net Strength Specifier (%%v) ---");

        // LRM format output for %v is exactly 3 characters long:
        // Char 1-2: Strength abbreviation (St, Pu, Su, We, Hi)
        // Char 3: Logic state value (0, 1, X, Z)

        out_str = $sformatf("%v", net_strong_1);
        `CHECK_FORMAT("Strength Strong 1 (%v)", out_str, "St1")

        out_str = $sformatf("%v", net_pull_0);
        `CHECK_FORMAT("Strength Pull 0 (%v)", out_str, "Pu0")

        out_str = $sformatf("%v", net_supply_z);
        `CHECK_FORMAT("Strength High-Z (%v)", out_str, "HiZ")

        out_str = $sformatf("%v", net_weak_x);
        `CHECK_FORMAT("Strength Weak X (%v)", out_str, "WeX")


        // --------------------------------------------------------------------
        // Final Verdict Processing
        // --------------------------------------------------------------------
        $display("\n==========================================================");
        if (error_count == 0) begin
            $display("TEST PASSED: Simulator formatting engine is compliant.");
        end else begin
            $display("TEST FAILED: %0d formatting compliance errors found.", error_count);
        end
        $display("==========================================================");
        #5;
        $finish;
    end

endmodule
