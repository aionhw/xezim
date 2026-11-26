library ieee;
use ieee.std_logic_1164.all;

entity packed_record_arrays is end;
architecture test of packed_record_arrays is
  type row is record
    value : std_logic_vector(7 downto 0);
    valid : std_logic;
  end record;
  type rows is array (2 downto -1) of row;
  type packed_rows_t is array (2 downto -1) of row;
  constant initial_row : row := (x"00", '0');
  signal packed_rows : packed_rows_t;
  signal delayed_rows, split_rows : rows;
  signal initial_rows : rows := (others => initial_row);
begin
  process
  begin
    for i in 2 downto -1 loop
      packed_rows(i).value <= x"A5";
      packed_rows(i).valid <= '1';
    end loop;
    packed_rows(-1).value(0) <= '0';
    packed_rows(2).value(7) <= '0';
    delayed_rows(-1).value <= x"12" after 3 ns;
    delayed_rows(0).valid <= '1' after 1 ns;
    split_rows(-1).value <= x"34";
    wait for 0 ns;
    assert packed_rows(-1).value = x"A4" and packed_rows(2).value = x"25"
      and packed_rows(0).value = x"A5" and packed_rows(0).valid = '1'
      report "packed field update or index order" severity failure;
    assert initial_rows(2) = initial_row and initial_rows(-1) = initial_row
      report "aggregate initialization" severity failure;
    assert split_rows(-1).value = x"34" and split_rows(2).value = x"56"
      report "disjoint drivers" severity failure;
    wait for 2 ns;
    assert delayed_rows(0).valid = '1' and delayed_rows(-1).value = "UUUUUUUU"
      report "first delayed field" severity failure;
    wait for 2 ns;
    assert delayed_rows(-1).value = x"12" and delayed_rows(0).valid = '1'
      report "independent projected waveforms" severity failure;
    report "PASS packed record arrays";
    std.env.finish;
  end process;
  split_rows(2).value <= x"56";
end;
