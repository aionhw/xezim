-- A design analysed as VHDL-2019 (--std=19): processes, signals, waits,
-- records and TEXTIO output, the base the 2019 features build on.
use std.textio.all;
entity basics19 is end entity;
architecture a of basics19 is
  type pair is record
    x, y : integer;
  end record;
  signal clk : bit := '0';
  signal p : pair := (0, 0);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk)
  begin
    if clk'event and clk = '1' then
      p <= (p.x + 1, p.y + p.x);
    end if;
  end process;
  process
    variable l : line;
  begin
    for i in 1 to 4 loop
      wait until clk = '1';
      write(l, string'("t=") & time'image(now) & " x=" & integer'image(p.x) & " y=" & integer'image(p.y));
      writeline(output, l);
    end loop;
    done <= true;
    wait;
  end process;
end architecture;
