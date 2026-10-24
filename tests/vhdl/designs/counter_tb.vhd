library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  generic (WIDTH : positive := 8);
  port (
    clk   : in  std_logic;
    rst   : in  std_logic;
    en    : in  std_logic;
    load  : in  std_logic;
    din   : in  std_logic_vector(WIDTH - 1 downto 0);
    count : out std_logic_vector(WIDTH - 1 downto 0);
    wrap  : out std_logic
  );
end entity;

architecture rtl of counter is
  signal cnt : unsigned(WIDTH - 1 downto 0);
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        cnt <= (others => '0');
        wrap <= '0';
      elsif load = '1' then
        cnt <= unsigned(din);
      elsif en = '1' then
        if cnt = 2 ** WIDTH - 1 then
          wrap <= '1';
        else
          wrap <= '0';
        end if;
        cnt <= cnt + 1;
      end if;
    end if;
  end process;
  count <= std_logic_vector(cnt);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tristate is
  port (oe : in std_logic; d : in std_logic_vector(3 downto 0); bus_io : inout std_logic_vector(3 downto 0));
end entity;

architecture rtl of tristate is
begin
  bus_io <= d when oe = '1' else (others => 'Z');
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;
use std.env.all;

entity counter_tb is
end entity;

architecture tb of counter_tb is
  constant PERIOD : time := 10 ns;
  signal clk : std_logic := '0';
  signal rst, en, load : std_logic := '0';
  signal din, count : std_logic_vector(3 downto 0);
  signal wrap : std_logic;
  signal oe1, oe2 : std_logic := '0';
  signal d1 : std_logic_vector(3 downto 0) := "1010";
  signal d2 : std_logic_vector(3 downto 0) := "0110";
  signal bus_s : std_logic_vector(3 downto 0);
  signal pull : std_logic_vector(3 downto 0) := "LLHH";
  signal done : boolean := false;
begin
  clk <= not clk after PERIOD / 2 when not done;

  dut : entity work.counter
    generic map (WIDTH => 4)
    port map (clk => clk, rst => rst, en => en, load => load, din => din, count => count, wrap => wrap);

  t1 : entity work.tristate port map (oe => oe1, d => d1, bus_io => bus_s);
  t2 : entity work.tristate port map (oe => oe2, d => d2, bus_io => bus_s);
  bus_s <= pull;

  stim : process
    variable l : line;
    variable u : unsigned(7 downto 0);
    variable s : signed(7 downto 0);
  begin
    rst <= '1';
    din <= "0000";
    wait until rising_edge(clk);
    wait until rising_edge(clk);
    rst <= '0';
    en <= '1';
    for i in 0 to 17 loop
      wait until rising_edge(clk);
      write(l, string'("t="));
      write(l, now);
      write(l, string'(" count="));
      write(l, count);
      write(l, string'(" wrap="));
      write(l, wrap);
      write(l, string'(" int="));
      write(l, to_integer(unsigned(count)));
      writeline(output, l);
    end loop;
    load <= '1';
    din <= x"C";
    wait until rising_edge(clk);
    load <= '0';
    wait until falling_edge(clk);
    assert count = x"C" report "load failed: " & to_hstring(count) severity error;
    report "after load: " & to_hstring(count) & " " & to_string(count);
    -- Bus resolution
    oe1 <= '1';
    wait for 1 ns;
    report "bus oe1: " & to_string(bus_s);
    oe2 <= '1';
    wait for 1 ns;
    report "bus both: " & to_string(bus_s);
    oe1 <= '0';
    wait for 1 ns;
    report "bus oe2: " & to_string(bus_s);
    oe2 <= '0';
    pull <= "ZZZZ";
    wait for 1 ns;
    report "bus none: " & to_string(bus_s);
    -- numeric_std arithmetic
    u := to_unsigned(200, 8);
    s := to_signed(-56, 8);
    report "u+77=" & integer'image(to_integer(u + 77)) & " u*2=" & to_hstring(u * 2)
      & " s=" & integer'image(to_integer(s)) & " s-100=" & integer'image(to_integer(s - 100))
      & " shl=" & to_string(shift_left(u, 3)) & " sra=" & to_string(shift_right(s, 2))
      & " resize=" & to_string(resize(s, 12)) & " cmp=" & boolean'image(u > 150)
      & " div=" & integer'image(to_integer(u / 7)) & " mod=" & integer'image(to_integer(u mod 7));
    report "std_match: " & boolean'image(std_match(unsigned'("1010"), unsigned'("1-1-")));
    report "x: " & to_string(u + unsigned'("0X01"));
    done <= true;
    wait for PERIOD;
    finish;
  end process;
end architecture;
