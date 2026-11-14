-- An element or a slice of an attribute's value as an operand of an
-- overloaded operator (IEEE 1076-2008 §8.4, §8.5, §12.5).
entity image_index_concat is end;
architecture x of image_index_concat is
  type st_t is (idle, run, done);
begin
  process
    variable s : st_t := run;
    variable c : character;
    variable b : boolean := true;
  begin
    c := st_t'image(s)(1);
    report "c=" & c;
    report "x" & st_t'image(s)(1);
    report st_t'image(s)(1) & "y";
    report "x" & st_t'image(s)(1) & boolean'image(b)(1) & "z";
    wait;
  end process;
end;
