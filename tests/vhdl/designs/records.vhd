-- Records (§5.3.3): element selection, aggregates, assignment of whole
-- records, a record signal.
entity records is end entity;
architecture a of records is
  type point is record
    x, y : integer;
    tag : character;
  end record;
  signal p : point := (0, 0, 'o');
  function dist2(q : point) return integer is
  begin
    return q.x * q.x + q.y * q.y;
  end function;
begin
  process
    variable q : point := (x => 3, y => 4, tag => 'q');
  begin
    report "q=(" & integer'image(q.x) & "," & integer'image(q.y) & ") tag=" & character'image(q.tag)
      & " dist2=" & integer'image(dist2(q));
    q.x := -1;
    p <= q;
    wait for 1 ns;
    report "p.x=" & integer'image(p.x) & " p.tag=" & character'image(p.tag) & " equal=" & boolean'image(p = q);
    wait;
  end process;
end architecture;
