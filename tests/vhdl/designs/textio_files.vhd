-- TEXTIO files (§16.4): a TEXT file declared with open information,
-- WRITE with justification and field width, HWRITE, OWRITE, BWRITE, REAL
-- with DIGITS, TIME with a unit, TEE, WRITELINE to a file; READLINE of
-- the file in another process (a CR before the line end is dropped),
-- ENDFILE, and READLINE past the end, which stops with GHDL's assertion.
use std.textio.all;

entity textio_files is
end entity;

architecture a of textio_files is
  file log_f : text open write_mode is "log.txt";
  signal phase : integer := 0;
begin
  writer : process
    variable l : line;
  begin
    write(l, string'("count"), left, 8);
    write(l, 42, right, 5);
    writeline(log_f, l);
    hwrite(l, bit_vector'("10111010"));
    write(l, ' ');
    owrite(l, bit_vector'("101110"));
    write(l, ' ');
    bwrite(l, bit_vector'("1100"), right, 6);
    writeline(log_f, l);
    write(l, 3.14159, right, 10, 3);
    write(l, string'("|"));
    write(l, 1.0e10);
    write(l, string'("|"));
    write(l, 2.5 ns, left, 10, ps);
    write(l, string'("|"));
    writeline(log_f, l);
    write(l, string'("dos"));
    write(l, CR);
    writeline(log_f, l);
    write(l, string'("tee line"));
    tee(log_f, l);
    file_close(log_f);
    phase <= 1;
    wait;
  end process;

  reader : process
    file f : text;
    variable l : line;
    variable n : natural := 0;
  begin
    wait until phase = 1;
    file_open(f, "log.txt", read_mode);
    while not endfile(f) loop
      readline(f, l);
      n := n + 1;
      report integer'image(n) & ": [" & l.all & "] " & integer'image(l'length);
    end loop;
    readline(f, l);
    report "not reached";
    wait;
  end process;
end architecture;
