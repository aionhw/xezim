-- Signals of record types driven element by element (§14.7.2), record
-- elements as port actuals, and events of single elements.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package bus_pkg is
  type inner_t is record
    a : std_logic;
    b : integer;
  end record;
  type req_t is record
    addr : std_ulogic_vector(7 downto 0);
    data : std_logic_vector(7 downto 0);
    stb  : std_ulogic;
    clk  : std_logic;
    sub  : inner_t;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.bus_pkg.all;

entity slave is
  port (stb : in std_ulogic; addr : in std_ulogic_vector(7 downto 0);
        data : out std_logic_vector(7 downto 0); ack : out std_ulogic);
end entity;

architecture rtl of slave is
begin
  process (stb)
  begin
    if stb = '1' then
      data <= not addr after 1 ns;
      ack <= '1' after 1 ns;
    else
      ack <= '0' after 1 ns;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bus_pkg.all;

entity record_signals is
end entity;

architecture sim of record_signals is
  signal r : req_t := (addr => x"00", data => (others => 'Z'), stb => '0', clk => '0',
                       sub => (a => '0', b => 0));
  signal ack : std_ulogic;
  signal copy : req_t;
  signal edges : natural := 0;
begin
  -- Different processes drive different elements.
  r.clk <= not r.clk after 5 ns when now < 60 ns;
  u : entity work.slave port map (stb => r.stb, addr => r.addr, data => r.data, ack => ack);
  -- A second driver of a resolved element.
  r.sub.a <= 'H';

  stim : process
  begin
    wait for 2 ns;
    r.addr <= x"3c";
    r.stb <= '1';
    wait until ack = '1';
    report "data=" & to_hstring(r.data) & " a=" & std_logic'image(r.sub.a);
    r.stb <= '0';
    r.sub.b <= 7;
    wait for 3 ns;
    r.addr <= x"a5";
    r.stb <= '1';
    wait for 2 ns;
    report "data=" & to_hstring(r.data) & " b=" & integer'image(r.sub.b);
    wait;
  end process;

  -- An edge of one element, not of the others.
  edge : process (r.clk)
  begin
    if rising_edge(r.clk) then
      edges <= edges + 1;
      report "edge " & integer'image(edges) & " addr=" & to_hstring(r.addr);
    end if;
  end process;

  -- The whole record, read and assigned.
  copy <= r;
  watch : process (copy)
  begin
    report "copy stb=" & std_logic'image(copy.stb) & " clk=" & std_logic'image(copy.clk);
  end process;
end architecture;
