library ieee; use ieee.std_logic_1164.all;
entity follow is
  port (a : in std_logic_vector(3 downto 0); y : out std_logic_vector(3 downto 0));
end entity;
architecture rtl of follow is
begin
  y <= a;
end architecture;

library ieee; use ieee.std_logic_1164.all;
entity port_exprs is end entity;
architecture tb of port_exprs is
  signal x, m : std_logic_vector(3 downto 0) := "0011";
  signal y_expr, y_name, y_conv : std_logic_vector(3 downto 0);
  function ident(v : std_logic_vector(3 downto 0)) return std_logic_vector is begin return v; end function;
begin
  u_expr: entity work.follow port map (a => x and m, y => y_expr);
  u_name: entity work.follow port map (a => x, y => y_name);
  u_conv: entity work.follow port map (a => ident(x), y => y_conv);
  process
  begin
    report "init expr=" & to_string(y_expr) & " name=" & to_string(y_name) & " conv=" & to_string(y_conv);
    for i in 1 to 3 loop
      wait for 0 ns;
      report "delta " & integer'image(i) & " expr=" & to_string(y_expr) & " name=" & to_string(y_name) & " conv=" & to_string(y_conv);
    end loop;
    wait for 1 ns;
    x <= "1111";
    for i in 1 to 4 loop
      wait for 0 ns;
      report "delta " & integer'image(i) & " expr=" & to_string(y_expr) & " name=" & to_string(y_name) & " conv=" & to_string(y_conv);
    end loop;
    wait;
  end process;
end architecture;
