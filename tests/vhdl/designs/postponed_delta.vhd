-- A postponed process must not cause a delta cycle (§11.3): GHDL stops
-- with an error.
entity postponed_delta is end entity;
architecture a of postponed_delta is
  signal s, t : integer := 0;
begin
  process begin wait for 1 ns; s <= 1; wait; end process;
  postponed process (s) begin
    report "s=" & integer'image(s);
    if s = 1 then t <= 2; end if;
  end process;
end architecture;
