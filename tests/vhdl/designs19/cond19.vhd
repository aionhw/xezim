-- VHDL-2019: conditional analysis (§24.2), conditional return statements
-- (§10.13) and the classes of generic types (§6.5.3).
use std.textio.all;
entity cond19 is end entity;
architecture a of cond19 is
  function clamp(x, lo, hi : integer) return integer is
  begin
    return lo when x < lo;
    return hi when x > hi else x;
  end function;
  -- An unaffected arm returns nothing: execution goes on.
  function sign(x : integer) return integer is
  begin
    return -1 when x < 0 else 1 when x > 0 else unaffected;
    return 0;
  end function;
  procedure bump(n : integer; variable c : inout integer) is
  begin
    c := c + 1;
    return when n < 3;
    c := c + 10;
  end procedure;
  function larger generic (type t is (<>)) parameter (a, b : t) return t is
  begin
    return a when a > b else b;
  end function;
  function twice generic (type t is range <>) parameter (x : t) return t is
  begin
    return x + x;
  end function;
  function larger_int is new larger generic map (t => integer);
  function larger_char is new larger generic map (t => character);
  function twice_int is new twice generic map (t => integer);
`if VHDL_VERSION < "2019" then
  constant revision : string := "before 2019";
`elsif VHDL_VERSION = "2019" then
  constant revision : string := "2019";
`else
  constant revision : string := "after 2019";
`end if
begin
  process
    variable l : line;
    variable c : integer := 0;
  begin
    write(l, "revision " & revision);
    writeline(output, l);
`if TOOL_TYPE = "SIMULATION" and not (TOOL_NAME = "") then
    write(l, string'("a simulator"));
`else
    write(l, string'("not a simulator"));
`end
    writeline(output, l);
    for i in -2 to 12 loop
      write(l, integer'image(i) & ": clamp " & integer'image(clamp(i, 0, 10))
        & " sign " & integer'image(sign(i - 5)));
      writeline(output, l);
    end loop;
    for n in 1 to 5 loop
      bump(n, c);
    end loop;
    write(l, "bump " & integer'image(c));
    writeline(output, l);
    write(l, "larger " & integer'image(larger_int(3, 7)) & " " & larger_char('q', 'b')
      & " twice " & integer'image(twice_int(21)));
    writeline(output, l);
    wait;
  end process;
end architecture;
