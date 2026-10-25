-- Actuals of constrained array parameters take the formal's bounds
-- (§4.2.2.1); a length mismatch is a bound check failure.
entity constrained_params is
end entity;

architecture a of constrained_params is
  function low_bit(d : bit_vector(3 downto 0)) return bit is
  begin
    return d(0);
  end function;
  function left_index(d : bit_vector(3 downto 0)) return integer is
  begin
    return d'left;
  end function;
  procedure show(d : in bit_vector(0 to 3)) is
  begin
    report "d(0)=" & bit'image(d(0)) & " left=" & integer'image(d'left);
  end procedure;
begin
  process
    variable x : bit_vector(31 downto 0) := x"5000_0001";
    variable y : bit_vector(7 downto 0) := x"0f";
    variable k : integer := 5;
  begin
    report "low=" & bit'image(low_bit(x(31 downto 28))) & " left=" & integer'image(left_index(x(31 downto 28)));
    show(x(31 downto 28));
    report "low=" & bit'image(low_bit(y(4 downto 1)));
    report "mismatch=" & bit'image(low_bit(y(k downto 1)));
    wait;
  end process;
end architecture;
