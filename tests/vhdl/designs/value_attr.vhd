-- §16.2.2 'VALUE of scalar types: decimal, based and exponent integer
-- literals, reals, enumeration literals (case-insensitive identifiers,
-- character literals) and physical literals; blanks around the text are
-- ignored; text that is no value fails (GHDL: bound check failure).
entity value_attr is end entity;
architecture a of value_attr is
  type color is (red, green, blue);
  type dist is range 0 to 1000000 units mm; m = 1000 mm; end units;
begin
  process
  begin
    report integer'image(integer'value(" -12_3 ")) & " " & integer'image(integer'value("16#FF#")) & " " & integer'image(integer'value("1E3"));
    report real'image(real'value(" 2.5 ")) & " " & real'image(real'value("1.0e-2"));
    report color'image(color'value(" GREEN ")) & " " & character'image(character'value("'x'")) & " " & boolean'image(boolean'value("TRUE"));
    report dist'image(dist'value("3 m")) & " " & dist'image(dist'value("7 MM"));
    report integer'image(integer'value("12x"));
    wait;
  end process;
end architecture;
