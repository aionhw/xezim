-- VHDL-2019 §5.5.2 FILE_STATE, FILE_MODE, FILE_SIZE, FILE_POSITION,
-- FILE_CANSEEK, FILE_SEEK (from the beginning, the current position and the
-- end) and FILE_REWIND of a file of INTEGER. Positions and sizes are in
-- bytes of the file's elements, as written (xezim writes GHDL's binary
-- format: a 4-byte INTEGER after a header, not counted); the golden is
-- written for that format (fileapi19.lrm).
use std.textio.all;
entity fileapi19 is end entity;
architecture a of fileapi19 is
  type int_file is file of integer;
begin
  process
    variable l : line;
    file f : int_file;
    variable v : integer;
  begin
    write(l, file_open_state'image(file_state(f)));
    file_open(f, "data.bin", write_mode);
    for i in 1 to 5 loop
      write(f, i * 10);
    end loop;
    write(l, " " & file_open_state'image(file_state(f)) & " " & file_open_kind'image(file_mode(f))
      & " size=" & integer'image(file_size(f)) & " pos=" & integer'image(file_position(f))
      & " " & boolean'image(file_canseek(f)));
    file_close(f);
    write(l, " " & file_open_state'image(file_state(f)));
    writeline(output, l);
    file_open(f, "data.bin", read_mode);
    file_seek(f, 8);
    read(f, v);
    write(l, "seek8=" & integer'image(v) & " pos=" & integer'image(file_position(f)));
    file_seek(f, 4, file_origin_current);
    read(f, v);
    write(l, " cur+4=" & integer'image(v));
    file_rewind(f);
    read(f, v);
    write(l, " rewind=" & integer'image(v));
    file_seek(f, -4, file_origin_end);
    read(f, v);
    write(l, " end-4=" & integer'image(v) & " " & boolean'image(endfile(f)));
    file_close(f);
    writeline(output, l);
    wait;
  end process;
end architecture;
