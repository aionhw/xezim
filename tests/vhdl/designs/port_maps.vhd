library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity unit is
  port (
    a   : in  integer;
    v   : in  std_logic_vector(3 downto 0);
    y   : out integer range 0 to 255;
    w   : out std_logic_vector(7 downto 0);
    bus_io : inout std_logic;
    oe  : in  std_logic;
    cnt : buffer natural := 0);
end entity;
architecture rtl of unit is
begin
  y <= a + to_integer(unsigned(v));
  w <= v & not v;
  bus_io <= '1' when oe = '1' else 'Z';
  process (a) begin cnt <= cnt + 1; end process;
end architecture;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity port_maps is end entity;
architecture tb of port_maps is
  signal u : unsigned(7 downto 0) := x"05";
  signal lo, hi : std_logic_vector(1 downto 0) := "01";
  signal r : unsigned(7 downto 0);
  signal wl, wh : std_logic_vector(3 downto 0);
  signal b : std_logic := 'L';
  signal en : std_logic := '0';
  signal c : natural;
  function to_u8(i : integer) return unsigned is begin return to_unsigned(i, 8); end function;
begin
  dut: entity work.unit port map (
    a => to_integer(u),
    v(1 downto 0) => lo, v(3 downto 2) => hi,
    to_u8(y) => r,
    w(7 downto 4) => wh, w(3 downto 0) => wl,
    bus_io => b, oe => en, cnt => c);
  b <= 'H';
  process
  begin
    wait for 1 ns;
    report "r=" & to_string(r) & " wh=" & to_string(wh) & " wl=" & to_string(wl) & " b=" & std_logic'image(b) & " c=" & integer'image(c);
    u <= x"10"; hi <= "10"; en <= '1';
    wait for 0 ns;
    report "d1 r=" & to_string(r) & " wh=" & to_string(wh) & " b=" & std_logic'image(b);
    wait for 0 ns;
    report "d2 r=" & to_string(r) & " wh=" & to_string(wh) & " b=" & std_logic'image(b);
    wait for 0 ns;
    report "d3 r=" & to_string(r) & " wh=" & to_string(wh) & " b=" & std_logic'image(b) & " c=" & integer'image(c);
    wait for 1 ns;
    lo <= "11"; en <= '0';
    wait for 1 ns;
    report "r=" & to_string(r) & " wh=" & to_string(wh) & " wl=" & to_string(wl) & " b=" & std_logic'image(b) & " c=" & integer'image(c);
    wait;
  end process;
end architecture;
