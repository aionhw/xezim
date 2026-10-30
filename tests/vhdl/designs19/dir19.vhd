-- VHDL-2019 §16.5 STD.ENV directories: DIR_CREATEDIR, DIR_OPEN (the
-- canonical name and the items, "." and ".." among them), DIR_CLOSE,
-- DIR_DELETEFILE and DIR_DELETEDIR of a directory with a file the design
-- writes; DIR_WORKINGDIR.
use std.textio.all;
use std.env.all;
entity dir19 is end entity;
architecture a of dir19 is
  type text_file is file of character;
begin
  process
    variable l : line;
    variable d : directory;
    variable os : dir_open_status;
    variable cs : dir_create_status;
    variable ds : dir_delete_status;
    variable fs : file_delete_status;
    variable has_a : boolean := false;
    file f : text_file;
  begin
    dir_createdir("sub", cs);
    file_open(f, "sub/a.txt", write_mode);
    write(f, 'x');
    file_close(f);
    dir_open(d, "sub", os);
    write(l, dir_open_status'image(os) & " " & integer'image(d.items'length));
    for i in d.items'range loop
      if d.items(i).all = "a.txt" then
        has_a := true;
      end if;
    end loop;
    write(l, " " & boolean'image(has_a) & " "
      & boolean'image(d.name.all(d.name'length - 3 to d.name'length) = "/sub")
      & " " & boolean'image(dir_workingdir & "/sub" = d.name.all));
    dir_close(d);
    dir_open(d, "no_such_dir", os);
    write(l, " " & dir_open_status'image(os));
    dir_deletedir("sub", ds);
    write(l, " " & dir_delete_status'image(ds));
    dir_deletefile("sub/a.txt", fs);
    dir_deletedir("sub", ds);
    write(l, " " & file_delete_status'image(fs) & " " & dir_delete_status'image(ds)
      & " " & boolean'image(dir_itemexists("sub")));
    writeline(output, l);
    wait;
  end process;
end architecture;
