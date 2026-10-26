-- A VHDL counter and an SV counter on one clock: each samples the other's
-- register on the same edge and must see the value from before the edge,
-- as two SV flip-flops do. The decoded output is combinational.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  port (clk, rst : in std_logic;
        sv_q : in std_logic_vector(3 downto 0);
        q, seen, dec : out std_logic_vector(3 downto 0));
end entity;

architecture rtl of counter is
  signal cnt : unsigned(3 downto 0);
begin
  process (clk, rst)
  begin
    if rst = '1' then
      cnt <= (others => '0');
      seen <= (others => '0');
    elsif rising_edge(clk) then
      cnt <= cnt + 1;
      seen <= sv_q;
    end if;
  end process;
  q <= std_logic_vector(cnt);
  dec <= not std_logic_vector(cnt);
end architecture;
