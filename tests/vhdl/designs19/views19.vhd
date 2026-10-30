-- VHDL-2019 mode views (§6.5.2): a record port whose elements have their
-- own modes, the converse view, a view of a record element, an array of
-- views, association of a port's elements one by one, and a signal
-- parameter with a view.
library ieee;
use ieee.std_logic_1164.all;
package bus_pkg is
  type bus_t is record
    req  : std_logic;
    data : std_logic_vector(7 downto 0);
    ack  : std_logic;
  end record;
  view master of bus_t is
    req, data : out;
    ack : in;
  end view;
  type pair_t is record
    fwd : bus_t;
    bwd : bus_t;
    en  : std_logic;
  end record;
  view pair_master of pair_t is
    fwd : view master;
    bwd : view master'converse;
    en  : out;
  end view;
  type bus_array is array (natural range <>) of bus_t;
  procedure respond(signal b : view master'converse; value : std_logic);
end package;

package body bus_pkg is
  procedure respond(signal b : view master'converse; value : std_logic) is
  begin
    b.ack <= value;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.bus_pkg.all;
entity producer is
  generic (base : natural);
  port (clk : in std_logic; m : view master);
end entity;
architecture a of producer is
  signal n : natural := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if m.ack = '1' then
        m.req <= '0';
      else
        m.req <= '1';
        m.data <= std_logic_vector(to_unsigned(base + n, 8));
        n <= n + 1;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity consumer is
  port (clk : in std_logic; s : view master'converse; got : out std_logic_vector(7 downto 0));
end entity;
architecture a of consumer is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      respond(s, s.req);
      if s.req = '1' then
        got <= s.data;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity pair_side is
  port (clk : in std_logic; p : view pair_master);
end entity;
architecture a of pair_side is
begin
  p.en <= '1';
  p.fwd.req <= clk;
  p.fwd.data <= x"5A";
  p.bwd.ack <= p.bwd.req;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.bus_pkg.all;
entity array_side is
  port (ms : view (master) of bus_array(0 to 1));
end entity;
architecture a of array_side is
begin
  ms(0).req <= '1';
  ms(0).data <= x"11";
  ms(1).req <= ms(1).ack;
  ms(1).data <= x"22";
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
use work.bus_pkg.all;
entity views19 is end entity;
architecture a of views19 is
  signal clk : std_logic := '0';
  signal b1 : bus_t;
  signal got1, got2 : std_logic_vector(7 downto 0);
  signal r2, a2 : std_logic;
  signal d2 : std_logic_vector(7 downto 0);
  signal pr : pair_t;
  signal arr : bus_array(0 to 1);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  p1 : entity work.producer generic map (base => 16) port map (clk => clk, m => b1);
  c1 : entity work.consumer port map (clk => clk, s => b1, got => got1);
  -- The elements of a view port associated one by one, each with its mode.
  p2 : entity work.producer generic map (base => 64)
    port map (clk => clk, m.req => r2, m.data => d2, m.ack => a2);
  c2 : entity work.consumer
    port map (clk => clk, s.req => r2, s.data => d2, s.ack => a2, got => got2);
  ps : entity work.pair_side port map (clk => clk, p => pr);
  pr.bwd.req <= pr.fwd.req;
  as : entity work.array_side port map (ms => arr);
  arr(0).ack <= arr(0).req;
  arr(1).ack <= '1';
  process
    variable l : line;
  begin
    for i in 1 to 6 loop
      wait until rising_edge(clk);
      wait for 1 ns;
      write(l, "t=" & time'image(now)
        & " b1 req=" & std_logic'image(b1.req) & " ack=" & std_logic'image(b1.ack)
        & " got1=" & to_hstring(got1)
        & " | r2=" & std_logic'image(r2) & " a2=" & std_logic'image(a2)
        & " got2=" & to_hstring(got2)
        & " | pair en=" & std_logic'image(pr.en) & " fwd=" & to_hstring(pr.fwd.data)
        & " bwd.ack=" & std_logic'image(pr.bwd.ack)
        & " | arr " & std_logic'image(arr(0).ack) & to_hstring(arr(0).data)
        & " " & std_logic'image(arr(1).req) & to_hstring(arr(1).data));
      writeline(output, l);
    end loop;
    done <= true;
    wait;
  end process;
end architecture;
