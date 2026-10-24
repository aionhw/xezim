-- Range checks of subtype-constrained objects (§14.7.3.1): a value
-- outside the subtype is a bound check failure at the assignment.
entity checks is end entity;
architecture a of checks is
  subtype digit is integer range 0 to 9;
begin
  process
    variable d : digit := 0;
  begin
    for i in 0 to 12 loop
      d := i;
      report "d=" & integer'image(d);
    end loop;
    wait;
  end process;
end architecture;
