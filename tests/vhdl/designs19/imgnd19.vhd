-- VHDL-2019 §16.2.2 'IMAGE, TO_STRING and 'VALUE of two-dimensional arrays: an
-- aggregate of the rows' aggregates (a row of character literals as a
-- string literal). nvc 1.23.1 prints only part of the first row, so the
-- golden is written from the LRM (imgnd19.lrm).
use std.textio.all;
entity imgnd19 is end entity;
architecture a of imgnd19 is
  type grid is array (1 to 2, 1 to 3) of integer;
  type bgrid is array (0 to 1, 0 to 1) of bit;
begin
  process
    variable l : line;
    variable g : grid := ((1, 2, 3), (4, 5, 6));
    variable b : bgrid := ("01", "10");
  begin
    write(l, grid'image(g) & " " & bgrid'image(b) & " " & to_string(g));
    writeline(output, l);
    g := grid'value(" ((6, 5, 4), (3, 2, 1)) ");
    b := bgrid'value("(""11"", ""00"")");
    write(l, grid'image(g) & " " & bgrid'image(b));
    writeline(output, l);
    wait;
  end process;
end architecture;
