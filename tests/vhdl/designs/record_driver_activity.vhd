library ieee;
use ieee.std_logic_1164.all;
entity record_driver_activity is end;
architecture test of record_driver_activity is
  type pair is record a, b : bit; end record;
  type bus_state is record
    data : pair;
    valid : bit;
    ack : std_logic;
  end record;
  signal bus_q : bus_state := (('0', '1'), '1', 'Z');
begin
  process
  begin
    bus_q.data.a <= '1';
    bus_q.data.b <= '0';
    bus_q.ack <= '1';
    wait for 0 ns;
    assert bus_q.data = ('1', '0') and bus_q.valid = '1' and bus_q.ack = '1'
      report "simultaneous field update" severity failure;
    bus_q.data.a <= '1';
    wait for 0 ns;
    assert bus_q.data.a'active and not bus_q.data.a'event and not bus_q.data.b'active
      report "equal-value field activity" severity failure;
    wait for 2 ns;
    assert bus_q.data = ('1', '0') and bus_q.valid = '0' and bus_q.ack = 'X'
      report "idle fields or shared resolution changed" severity failure;
    report "PASS record driver activity";
    std.env.finish;
  end process;
  process
  begin
    bus_q.ack <= 'Z';
    wait for 1 ns;
    bus_q.ack <= '0';
    wait;
  end process;
  bus_q.valid <= '0' after 1 ns;
end;
