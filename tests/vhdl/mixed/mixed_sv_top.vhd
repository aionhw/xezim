-- A VHDL entity instantiated from a SystemVerilog testbench
-- (mixed_sv_top.sv): generics from SV parameters, every port type that
-- crosses the language boundary, and an internal signal read through a
-- hierarchical name.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity acc is
  generic (
    width : positive := 4;
    init  : natural := 0;
    name  : string := "acc";
    neg   : boolean := false;
    scale : real := 1.0;
    pat   : std_logic_vector(3 downto 0) := "0000");
  port (
    clk, rst : in std_logic;
    din      : in std_logic_vector(width - 1 downto 0);
    asc      : in std_logic_vector(0 to 3);
    q        : out std_logic_vector(width - 1 downto 0);
    sum      : out unsigned(7 downto 0);
    carry    : out std_logic;
    zero     : out std_logic;
    nine     : out std_logic_vector(8 downto 0);
    seen     : out std_logic_vector(3 downto 0);
    b_in     : in bit;
    b_out    : out boolean;
    i_in     : in integer;
    i_out    : out integer;
    r_in     : in real;
    r_out    : out real);
end entity;

architecture rtl of acc is
  signal count : integer := 0;
  signal acc_r : unsigned(width downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        acc_r <= to_unsigned(init, width + 1);
        count <= 0;
      else
        acc_r <= ('0' & acc_r(width - 1 downto 0)) + unsigned(din);
        count <= count + 1;
      end if;
    end if;
  end process;
  q <= std_logic_vector(acc_r(width - 1 downto 0));
  carry <= acc_r(width);
  sum <= resize(acc_r, 8);
  zero <= '1' when acc_r(width - 1 downto 0) = 0 else '0';
  -- The nine values, to see how each leaves for SV.
  nine <= "UX01ZWLH-";
  -- The values SV drives, as VHDL sees them.
  seen <= asc;
  b_out <= (b_in = '1') xor neg;
  i_out <= i_in * 2 - 1;
  r_out <= r_in * scale;
end architecture;
