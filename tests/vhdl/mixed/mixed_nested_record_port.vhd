-- A three-dimensional array of records leaving VHDL as a port, copied out of
-- a record field (PeakRDL-etana's `hwif_out_r1 <= hwif_out.r1`), and one of
-- its elements read by a concurrent assignment, whose sensitivity test
-- needs a temporary of its own (see mixed_nested_record_port.lrm).
library ieee;
use ieee.std_logic_1164.all;
package rp_pkg is
  type fld_t is record
    value : std_logic_vector(2 to 2);
    swmod : std_logic;
  end record;
  type fld_arr is array (natural range <>, natural range <>, natural range <>) of fld_t;
  type hw_t is record
    r1 : fld_arr(0 to 1, 0 to 1, 0 to 2);
    en : std_logic;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.rp_pkg.all;
entity regs is
  port (clk : in std_logic; d : in std_logic;
        hwif_out : out hw_t);
end entity;
architecture rtl of regs is
  signal st : hw_t := (r1 => (others => (others => (others => (value => "0", swmod => '0')))), en => '0');
begin
  process (clk) begin
    if rising_edge(clk) then
      st.r1(1, 0, 2).value <= (2 => d);
      st.r1(0, 1, 1).swmod <= not st.r1(0, 1, 1).swmod;
      st.en <= d;
    end if;
  end process;
  hwif_out <= st;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.rp_pkg.all;
entity wrap is
  port (clk : in std_logic; d : in std_logic;
        hwif_out_r1 : out fld_arr(0 to 1, 0 to 1, 0 to 2);
        en : out std_logic; v102 : out std_logic);
end entity;
architecture rtl of wrap is
  signal hwif_out : hw_t;
begin
  u : entity work.regs port map (clk => clk, d => d, hwif_out => hwif_out);
  hwif_out_r1 <= hwif_out.r1;
  en <= hwif_out.en;
  v102 <= hwif_out.r1(1, 0, 2).value(2);
end architecture;
