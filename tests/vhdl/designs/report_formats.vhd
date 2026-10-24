entity report_formats is
end entity;
architecture a of report_formats is
  signal s : integer := 0;
begin
  process
    variable r : real := 3.25;
    variable t : time := 1500 ps;
  begin
    report "plain report";
    report "note sev" severity note;
    report "warning sev" severity warning;
    wait for 10 ns;
    report "error sev" severity error;
    assert false report "assert default";
    assert 1 = 2 report "assert warning" severity warning;
    report "image int=" & integer'image(-42) & " real=" & real'image(r) & " time=" & time'image(t);
    report "image real2=" & real'image(1.0e10) & " " & real'image(0.1) & " " & real'image(-2.5e-7);
    report "image time2=" & time'image(1 us) & " " & time'image(0 ns) & " " & time'image(2.5 ns);
    report "image bool=" & boolean'image(true) & " char=" & character'image('a') & " bit=" & bit'image('1');
    wait for 1 ns;
    report "failure next";
    report "fail sev" severity failure;
    report "after failure";
    wait;
  end process;
end architecture;
