-- A VHDL testbench with a SystemVerilog module (mixed_vhdl_top.sv) bound
-- to a component of its name; output through TEXTIO.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

entity mixed_vhdl_top is
end entity;

architecture sim of mixed_vhdl_top is
  component counter is
    generic (width : integer := 4; name : string := "c"; step : real := 1.0);
    port (
      clk, rst : in std_logic;
      load     : in std_logic_vector(width - 1 downto 0);
      count    : out std_logic_vector(width - 1 downto 0);
      wrap     : out std_logic;
      steps    : out integer;
      scaled   : out real);
  end component;
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal cnt : std_logic_vector(5 downto 0);
  signal w : std_logic;
  signal st : integer;
  signal sc : real;
  signal half : std_logic_vector(3 downto 0);
  signal running : boolean := true;

  procedure say(s : string) is
    variable l : line;
  begin
    write(l, s);
    writeline(output, l);
  end procedure;
begin
  u0 : counter
    generic map (width => 6, name => "six", step => 0.25)
    port map (clk => clk, rst => rst, load => "111100", count => cnt, wrap => w,
              steps => st, scaled => sc);
  -- A second instance on parts of signals.
  u1 : counter
    generic map (width => 2)
    port map (clk => clk, rst => rst, load => "10", count => half(2 downto 1),
              wrap => half(0), steps => open, scaled => open);
  half(3) <= '0';
  clk <= not clk after 5 ns when running;
  process
  begin
    wait for 12 ns;
    rst <= '0';
    for i in 1 to 6 loop
      wait until rising_edge(clk);
      wait for 1 ns;
      say(integer'image(now / 1 ns) & " ns cnt=" & to_hstring(cnt) & " wrap="
          & std_logic'image(w) & " steps=" & integer'image(st) & " scaled*100="
          & integer'image(integer(sc * 100.0)) & " half=" & to_string(half));
    end loop;
    say("done");
    running <= false;
    wait;
  end process;
  process (cnt)
  begin
    say(integer'image(now / 1 ns) & " ns cnt event " & to_string(cnt));
  end process;
end architecture;
