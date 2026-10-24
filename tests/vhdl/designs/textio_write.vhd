use std.textio.all;

entity textio_write is
end entity;

architecture sim of textio_write is
begin
  process
    variable l : line;
    variable l2 : line;
  begin
    write(l, string'("hello"));
    write(l, ' ');
    write(l, 42);
    write(l, ' ');
    write(l, -7, right, 5);
    write(l, '|');
    write(l, 13, left, 6);
    write(l, '|');
    writeline(output, l);
    write(l, true); write(l, ' '); write(l, false); write(l, ' '); write(l, bit'('1'));
    write(l, ' '); write(l, bit_vector'("1010")); write(l, ' ');
    write(l, 1.5); write(l, ' '); write(l, 3.14159, right, 0, 2); write(l, ' '); write(l, 1.0e10);
    writeline(output, l);
    write(l, 10 ns); write(l, ' '); write(l, 1500 ps, right, 0, ns); write(l, ' '); write(l, now);
    write(l, ' '); write(l, 2 us, left, 10); write(l, '|');
    writeline(output, l);
    report "len after writeline: " & integer'image(l'length);
    if l = null then report "null"; else report "not null"; end if;
    writeline(output, l);
    l2 := new string'("abc");
    report "l2: " & l2.all & " " & integer'image(l2'length) & " " & l2(2);
    l2.all(1) := 'X';
    report "l2: " & l2.all;
    deallocate(l2);
    if l2 = null then report "l2 null"; end if;
    write(l, string'("x"), right, 4); write(l, string'("y"), left, 3); write(l, '|');
    writeline(output, l);
    swrite(l, "sw"); write(l, justify("j", right, 3));
    writeline(output, l);
    write(l, 1.0 / 3.0); write(l, ' '); write(l, -0.000123); write(l, ' '); write(l, 0.0); write(l, ' '); write(l, 100.0);
    writeline(output, l);
    write(l, 2.5, right, 8, 3); write(l, '|');
    writeline(output, l);
    wait for 7 ns;
    write(l, now); write(l, ' '); write(l, now, right, 0, ps); write(l, ' '); write(l, 7 ns, right, 0, us);
    writeline(output, l);
    wait;
  end process;
end architecture;
