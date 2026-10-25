-- Slices and elements as actuals of out and inout parameters (§4.2.2.2):
-- the procedure's values are stored back into them.
entity slice_actuals is end entity;
architecture a of slice_actuals is
  procedure set(o : out bit_vector; good : out boolean) is
  begin
    o := (o'range => '1');
    good := true;
  end procedure;
  procedure inc(x : inout integer) is begin x := x + 1; end procedure;
begin
  process
    variable v : bit_vector(0 to 7) := (others => '0');
    variable a : integer_vector(0 to 3) := (others => 0);
    variable g : boolean;
  begin
    set(v(4 to 7), g);
    report to_string(v);
    for i in 0 to 1 loop set(v(2*i to 2*i), g); end loop;
    report to_string(v);
    inc(a(2)); inc(a(2));
    report integer'image(a(2));
    wait;
  end process;
end architecture;
