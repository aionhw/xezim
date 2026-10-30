-- VHDL-2019 §16.5 STD.ENV: VHDL_VERSION, TOOL_TYPE, the call's source
-- location (FILE_NAME, FILE_LINE), DIR_SEPARATOR, GETENV of an unset
-- variable, the calendar functions of fixed times (GMTIME, EPOCH of a time
-- record, TO_STRING, "+"), TIME_TO_SECONDS and SECONDS_TO_TIME, the
-- assertion counts and enables, and the directory functions.
use std.textio.all;
use std.env.all;
entity env19 is end entity;
architecture a of env19 is
begin
  process
    variable l : line;
    variable tr, tr2 : time_record;
    variable cs : dir_create_status;
    variable ds : dir_delete_status;
  begin
    write(l, vhdl_version & " " & tool_type & " " & file_name & " " & integer'image(file_line)
      & " " & dir_separator & " " & boolean'image(getenv("XEZIM_SURELY_NOT_SET") = ""));
    writeline(output, l);
    tr := gmtime(86400.0 * 365.0 + 3661.25);
    write(l, to_string(tr) & " " & to_string(tr, 3) & " " & integer'image(tr.year) & " "
      & integer'image(tr.month) & " " & integer'image(tr.dayofyear) & " " & dayofweek'image(tr.weekday));
    writeline(output, l);
    tr2 := tr + 86400.0 * 59.0;
    write(l, to_string(tr2) & " " & dayofweek'image(tr2.weekday) & " "
      & integer'image(integer(epoch(tr2) - epoch(tr))) & " " & integer'image(integer(tr2 - tr)));
    writeline(output, l);
    tr := gmtime(951782400.0);
    write(l, to_string(tr) & " " & integer'image(tr.dayofyear) & " " & to_string(gmtime(-86400.0)));
    writeline(output, l);
    write(l, real'image(time_to_seconds(1500 ms)) & " " & time'image(seconds_to_time(2.5)));
    writeline(output, l);
    assert false report "one" severity error;
    assert false report "two" severity warning;
    write(l, boolean'image(isvhdlassertfailed) & " " & integer'image(getvhdlassertcount) & " "
      & integer'image(getvhdlassertcount(error)) & " " & boolean'image(isvhdlassertfailed(failure)));
    writeline(output, l);
    clearvhdlassert;
    setvhdlassertenable(warning, false);
    assert false report "three" severity warning;
    write(l, integer'image(getvhdlassertcount) & " " & boolean'image(getvhdlassertenable(warning))
      & " " & boolean'image(getvhdlassertenable(error)));
    writeline(output, l);
    write(l, boolean'image(dir_itemexists(".")) & " " & boolean'image(dir_itemisdir("."))
      & " " & boolean'image(dir_itemisfile("env19.vhd")) & " " & boolean'image(dir_itemisdir("env19.vhd"))
      & " " & boolean'image(dir_itemexists("no_such_item")));
    writeline(output, l);
    dir_createdir("made", cs);
    write(l, dir_create_status'image(cs) & " " & boolean'image(dir_itemisdir("made")));
    dir_createdir("made", cs);
    write(l, " " & dir_create_status'image(cs));
    dir_deletedir("made", ds);
    write(l, " " & dir_delete_status'image(ds) & " " & boolean'image(dir_itemexists("made")));
    writeline(output, l);
    wait;
  end process;
end architecture;
