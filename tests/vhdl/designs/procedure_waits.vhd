-- Procedures that wait and assign signals (§4.2.2.3, §10.2): signal
-- parameters of modes in, out and inout denote the actual signals (also
-- elements), in a package and nested in a process, called from several
-- places; wait on, until and for in procedures, timeouts and 'event of a
-- signal parameter.
library ieee; use ieee.std_logic_1164.all;
package tbutil is
  procedure wait_cycles(signal clk : in std_logic; n : natural);
  procedure pulse(signal s : out std_logic; t : time);
  procedure drive_bit(signal v : out std_logic; x : std_logic);
  procedure handshake(signal req : out std_logic; signal ack : in std_logic; tmo : time;
                      ok : out boolean);
end package;
package body tbutil is
  procedure wait_cycles(signal clk : in std_logic; n : natural) is
  begin
    for i in 1 to n loop
      wait until rising_edge(clk);
    end loop;
  end procedure;
  procedure pulse(signal s : out std_logic; t : time) is
  begin
    s <= '1'; wait for t; s <= '0';
  end procedure;
  procedure drive_bit(signal v : out std_logic; x : std_logic) is
  begin
    v <= x;
  end procedure;
  procedure handshake(signal req : out std_logic; signal ack : in std_logic; tmo : time;
                      ok : out boolean) is
  begin
    req <= '1';
    wait until ack = '1' for tmo;
    ok := ack = '1';
    if ack'event then report "ack event at " & time'image(now); end if;
    req <= '0';
  end procedure;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.tbutil.all;
entity procedure_waits is end entity;
architecture a of procedure_waits is
  signal clk : std_logic := '0';
  signal req, ack, p : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := "0000";
  signal done : boolean := false;
  signal t : integer := 0;
  procedure put(signal x : inout integer; d : integer) is
  begin
    x <= x + d after 1 ns;
    wait on x;
  end procedure;
begin
  clk <= not clk after 5 ns when not done;
  -- Acknowledges the first request only.
  process begin
    wait until req = '1';
    wait for 12 ns; ack <= '1';
    wait until req = '0'; ack <= '0';
    wait;
  end process;
  process
    variable ok : boolean;
    variable n : integer := 0;
    procedure step(signal x : out std_logic; k : natural) is
    begin
      wait_cycles(clk, k);
      pulse(x, 3 ns);
    end procedure;
    procedure bump is begin n := n + 1; t <= t + 1; end procedure;
  begin
    wait_cycles(clk, 2);
    report "after 2 cycles " & time'image(now);
    step(p, 1);
    report "after step " & time'image(now) & " p=" & std_logic'image(p);
    wait for 1 ns;
    drive_bit(v(2), '1'); drive_bit(v(0), 'H');
    wait for 0 ns;
    report "v=" & to_string(v);
    handshake(req, ack, 50 ns, ok);
    report "hs1 ok=" & boolean'image(ok) & " at " & time'image(now);
    wait for 10 ns;
    handshake(req, ack, 20 ns, ok);
    report "hs2 ok=" & boolean'image(ok) & " at " & time'image(now);
    bump; wait for 0 ns; bump; wait for 0 ns;
    report "n=" & integer'image(n) & " t=" & integer'image(t);
    put(t, 7); report "t=" & integer'image(t) & " at " & time'image(now);
    put(t, 3); report "t=" & integer'image(t) & " at " & time'image(now);
    done <= true;
    wait;
  end process;
  process (p) begin report "p=" & std_logic'image(p) & " at " & time'image(now); end process;
end architecture;
