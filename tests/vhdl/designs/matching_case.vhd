-- Matching case and selected assignment (§10.9, §10.5.4, §11.6): choices
-- with '-' compared by "?="; the matching operators (§9.2.3) and the
-- implicit condition operator in an if.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity matching_case is end entity;
architecture a of matching_case is
  signal s : std_logic_vector(3 downto 0) := "0000";
  signal y : integer;
  signal b : bit := '0';
  signal z : integer;
begin
  with s select? y <= 1 when "1---", 2 when "01--", 3 when "001-", 4 when others;
  with b select? z <= 10 when '1', 20 when others;
  process
    variable v : std_logic_vector(2 downto 0);
  begin
    for i in 0 to 7 loop
      v := std_logic_vector(to_unsigned(i, 3));
      case? v is
        when "1--" => report "1-- " & to_string(v);
        when "01-" | "001" => report "01- or 001 " & to_string(v);
        when others => report "other " & to_string(v);
      end case?;
    end loop;
    v := "H0L";
    case? v is
      when "1-0" => report "H0L matches 1-0";
      when others => report "H0L no match";
    end case?;
    s <= "0110"; b <= '1'; wait for 1 ns; report "y=" & integer'image(y) & " z=" & integer'image(z);
    s <= "1110"; b <= '0'; wait for 1 ns; report "y=" & integer'image(y) & " z=" & integer'image(z);
    s <= "0001"; wait for 1 ns; report "y=" & integer'image(y);
    report "m " & std_logic'image(std_logic_vector'("1-0") ?= "110") & std_logic'image('1' ?/= 'H')
      & std_logic'image(unsigned'("10") ?< "11") & std_logic'image('X' ?= '1');
    if s ?= "0001" then report "cond ok"; end if;
    wait;
  end process;
end architecture;
