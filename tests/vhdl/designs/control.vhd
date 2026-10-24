-- Sequential control (§10.8–§10.12): if/elsif, case with ranges and
-- alternatives, for loops in both directions, while, labelled next/exit
-- from nested loops, null ranges.
entity control is end entity;
architecture a of control is
begin
  process
    variable acc : integer := 0;
    variable msg : integer := 0;
  begin
    for i in 1 to 10 loop
      case i is
        when 1 | 3 => acc := acc + 100;
        when 4 to 6 => acc := acc + 10;
        when others => acc := acc + 1;
      end case;
    end loop;
    report "case acc=" & integer'image(acc);
    acc := 0;
    for i in 5 downto 1 loop
      acc := acc * 10 + i;
    end loop;
    report "downto acc=" & integer'image(acc);
    acc := 0;
    outer: for i in 0 to 9 loop
      inner: for j in 0 to 9 loop
        next outer when j > i;
        exit outer when i * j > 20;
        acc := acc + 1;
      end loop inner;
    end loop outer;
    report "nested acc=" & integer'image(acc);
    acc := 0;
    while acc < 1000 loop
      acc := acc * 2 + 1;
    end loop;
    report "while acc=" & integer'image(acc);
    for i in 3 to 1 loop
      report "never";
    end loop;
    if acc > 2000 then
      msg := 1;
    elsif acc > 1000 then
      msg := 2;
    else
      msg := 3;
    end if;
    report "if msg=" & integer'image(msg);
    wait;
  end process;
end architecture;
