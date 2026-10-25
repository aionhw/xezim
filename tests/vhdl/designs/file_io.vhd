-- Files (§5.5, §6.4.2.5): FILE_OPEN with and without a status (OPEN_OK,
-- NAME_ERROR, STATUS_ERROR), the open kinds, FILE_CLOSE, ENDFILE, READ and
-- WRITE of integers, records with nested records, constrained and
-- unconstrained arrays (READ with LENGTH), and a file declared with file
-- open information. The values are stored as GHDL stores them: the header
-- of a file of a composite type is read back here as characters.
entity file_io is
end entity;

architecture a of file_io is
  type int_file is file of integer;
  type inner is record
    c : character;
    t : time;
  end record;
  type rec is record
    a : integer;
    b : real;
    v : bit_vector(3 downto 0);
    n : inner;
    ok : boolean;
  end record;
  type rec_file is file of rec;
  type str_file is file of string;
  type pair is array (0 to 1) of integer;
  type pair_file is file of pair;
  type char_file is file of character;
  file early : int_file open write_mode is "early.bin";
begin
  process
    file f : int_file;
    file rf : rec_file;
    file sf : str_file;
    file pf : pair_file;
    file cf : char_file;
    variable v, n, sum : integer := 0;
    variable r : rec;
    variable st : file_open_status;
    variable s : string(1 to 4);
    variable len : natural;
    variable p : pair;
    variable c : character;
    variable hdr : string(1 to 40);
  begin
    write(early, 7);
    file_close(early);
    file_open(f, "early.bin");
    read(f, v);
    file_close(f);
    report "early=" & integer'image(v);

    file_open(f, "ints.bin", write_mode);
    for i in 1 to 5 loop
      write(f, i * i - 10);
    end loop;
    file_close(f);
    file_open(st, f, "ints.bin", append_mode);
    report "append " & file_open_status'image(st);
    write(f, 1000);
    file_open(st, f, "ints.bin", read_mode);
    report "again " & file_open_status'image(st);
    file_close(f);
    file_open(f, "ints.bin", read_mode);
    while not endfile(f) loop
      read(f, v);
      sum := sum + v;
      n := n + 1;
    end loop;
    file_close(f);
    report "count=" & integer'image(n) & " sum=" & integer'image(sum) & " last=" & integer'image(v);
    file_open(st, f, "no/such/dir/x.bin", read_mode);
    report "missing " & file_open_status'image(st);

    file_open(rf, "recs.bin", write_mode);
    write(rf, (7, 2.5, "1010", ('q', 3 ns), true));
    write(rf, (-3, -0.125, "0110", ('z', 1 fs), false));
    file_close(rf);
    file_open(rf, "recs.bin", read_mode);
    read(rf, r);
    report "rec " & integer'image(r.a) & " " & real'image(r.b) & " " & to_string(r.v)
      & " " & r.n.c & " " & time'image(r.n.t) & " " & boolean'image(r.ok);
    read(rf, r);
    report "rec " & integer'image(r.a) & " " & real'image(r.b) & " " & to_string(r.v)
      & " " & r.n.c & " " & time'image(r.n.t) & " " & boolean'image(r.ok)
      & " eof " & boolean'image(endfile(rf));
    file_close(rf);

    file_open(cf, "recs.bin", read_mode);
    n := 0;
    loop
      read(cf, c);
      exit when c = '.';
      n := n + 1;
      if c = LF then hdr(n) := '/'; else hdr(n) := c; end if;
    end loop;
    file_close(cf);
    report "header " & hdr(1 to n);

    file_open(sf, "strs.bin", write_mode);
    write(sf, "hello");
    write(sf, "ab");
    file_close(sf);
    file_open(sf, "strs.bin", read_mode);
    s := "....";
    read(sf, s, len);
    report "s=" & s & " len=" & integer'image(len);
    s := "....";
    read(sf, s, len);
    report "s=" & s & " len=" & integer'image(len) & " eof " & boolean'image(endfile(sf));
    file_close(sf);

    file_open(pf, "pairs.bin", write_mode);
    write(pf, (11, -22));
    file_close(pf);
    file_open(pf, "pairs.bin", read_mode);
    read(pf, p);
    file_close(pf);
    report "pair " & integer'image(p(0)) & " " & integer'image(p(1));
    wait;
  end process;
end architecture;
