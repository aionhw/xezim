entity time_units is end entity;
architecture a of time_units is begin
  process
    variable i : integer := integer'high;
  begin
    wait for 2 ms; report "2ms";
    wait for 998 ms; report "1sec";
    wait for 59 sec; report "1min";
    wait for 59 min; report "1hr";
    report "high=" & integer'image(i) & " low=" & integer'image(integer'low);
    report "timehigh=" & time'image(time'high);
    wait;
  end process;
end architecture;
