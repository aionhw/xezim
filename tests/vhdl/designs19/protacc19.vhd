-- VHDL-2019 §5.4.1, §5.6.2: an access type of a protected type (not allowed
-- before 2019), a protected type completing an incomplete type
-- declaration, objects of it allocated by access values (new), and method
-- calls through an access value, with and without .all (§8.3). nvc 1.23.1
-- rejects the incomplete type seen through a use clause, so the golden is
-- written from the LRM (protacc19.lrm).
package pacc_pkg is
  type counter_pt;
  type counter is access counter_pt;
  type counter_pt is protected
    procedure inc(n : integer);
    impure function get return integer;
  end protected;
end package;

package body pacc_pkg is
  type counter_pt is protected body
    variable count : integer := 0;
    procedure inc(n : integer) is
    begin
      count := count + n;
    end procedure;
    impure function get return integer is
    begin
      return count;
    end function;
  end protected body;
end package body;

use std.textio.all;
use work.pacc_pkg.all;
entity protacc19 is end entity;
architecture a of protacc19 is
begin
  process
    variable a, b : counter;
    variable l : line;
  begin
    a := new counter_pt;
    b := new counter_pt;
    a.inc(2);
    a.all.inc(5);
    b.inc(1);
    write(l, integer'image(a.get) & " " & integer'image(b.all.get) & " " & boolean'image(a = b));
    writeline(output, l);
    b := a;
    b.inc(10);
    write(l, integer'image(a.get));
    writeline(output, l);
    wait;
  end process;
end architecture;
