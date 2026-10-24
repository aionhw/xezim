library ieee;
use ieee.std_logic_1164.all;

package respkg is
  type int_vec is array (natural range <>) of integer;
  function sum (v : int_vec) return integer;
  subtype rint is sum integer;
end package;

package body respkg is
  function sum (v : int_vec) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := s + v(i);
    end loop;
    return s;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;

entity cnt is
  port (clk : in std_ulogic; q : out bit_vector(3 downto 0); tc : out bit);
end entity;

architecture rtl of cnt is
  signal r : bit_vector(3 downto 0) := "0000";
begin
  process (clk)
  begin
    if rising_edge(clk) then
      case r is
        when "1111" => r <= "0000";
        when "0000" => r <= "0001";
        when "0001" => r <= "0010";
        when "0010" => r <= "0011";
        when others => r <= not r;
      end case;
    end if;
  end process;
  q <= r;
  tc <= r(3) and r(2);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.respkg.all;

entity hierarchy is
end entity;

architecture sim of hierarchy is
  signal clk : bit := '0';
  signal uclk : std_ulogic := '0';
  signal q : bit_vector(3 downto 0);
  signal tc : bit;
  signal parts : bit_vector(0 to 3);
  signal total : rint := 0;
begin
  uclk <= '1' when clk = '1' else '0';
  u0 : entity work.cnt port map (clk => uclk, q => q, tc => tc);
  parts(0) <= '1' after 2 ns;
  parts(1 to 2) <= "11" after 3 ns;
  p3 : process
  begin
    parts(3) <= '1';
    wait for 4 ns;
    parts(3) <= '0';
    wait;
  end process;
  total <= 5 after 1 ns;
  total <= 7 after 2 ns;
  clkgen : process
  begin
    for i in 1 to 12 loop
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
    end loop;
    wait;
  end process;
  mon : process (q, tc)
  begin
    report "q=" & to_string(q) & " tc=" & bit'image(tc);
  end process;
  mon2 : process (parts, total)
  begin
    report "parts=" & to_string(parts) & " total=" & integer'image(total);
  end process;
end architecture;
