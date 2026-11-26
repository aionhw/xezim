library ieee;
use ieee.std_logic_1164.all;

entity disjoint_record_events is end;
architecture test of disjoint_record_events is
  type row is record
    data : std_logic_vector(7 downto 0);
    valid : bit;
  end record;
  type rows is array (0 to 1) of row;
  type state is record
    values : rows;
    end_mark : bit;
  end record;
  constant initial_state : state := (((x"00", '0'), (x"00", '0')), '0');
  constant first_state : state := (((x"A5", '0'), (x"5A", '1')), '0');
  signal r : state := initial_state;
begin
  process
  begin
    r.values(0).data <= x"A5";
    wait for 0 ns;
    assert r'event and r = first_state and r'last_value = initial_state
      report "disjoint writes must record one event and keep the old value" severity failure;
    r.values(0).data <= x"A5";
    wait for 0 ns;
    assert r'active and not r'event and r'last_value = initial_state
      report "a repeated transaction must not advance last_value" severity failure;
    wait for 3 ns;
    assert r.values = first_state.values and r.end_mark = '1'
      and r'last_value = first_state
      report "a later event must preserve idle fields" severity failure;
    report "PASS disjoint record events";
    std.env.finish;
  end process;

  process
  begin
    r.values(1).data <= x"5A";
    r.values(1).valid <= '1';
    wait for 2 ns;
    r.end_mark <= '1';
    wait;
  end process;
end;
