-- A subprogram nested in another one names the enclosing subprogram's
-- parameters, variables and constants (IEEE 1076-2008 §12.1, §12.2), also
-- from two levels down, through a sibling and under recursion.
library ieee;
use ieee.std_logic_1164.all;

package nested_pkg is
  function name_len(name : string) return integer;
  function count_ones(v : std_logic_vector) return integer;
end package;

package body nested_pkg is
  function name_len(name : string) return integer is
    function inner return integer is
    begin
      return name'length;
    end function;
  begin
    return inner;
  end function;

  -- A parameter with the bounds of each call's actual.
  function count_ones(v : std_logic_vector) return integer is
    variable n : integer := 0;
    procedure tally(i : integer) is
    begin
      if v(i) = '1' then
        n := n + 1;
      end if;
    end procedure;
  begin
    for i in v'range loop
      tally(i);
    end loop;
    return n;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.nested_pkg.all;

entity nested_subprogram_uplevel is end;

architecture a of nested_subprogram_uplevel is
  impure function outer(name : string; k : integer) return integer is
    variable acc : integer := 10;
    constant c : integer := k * 2;
    impure function inner return integer is
    begin
      return name'length + acc + c;
    end function;
    procedure bump(d : integer) is
      impure function deep return integer is
      begin
        return acc + d + k;
      end function;
    begin
      acc := acc + deep;
    end procedure;
    -- Calls a sibling that names the enclosing variables.
    procedure twice(d : integer) is
    begin
      bump(d);
      bump(d);
    end procedure;
  begin
    bump(1);
    twice(name'length);
    return inner + acc;
  end function;

  -- Each activation has its own objects.
  function fact(n : integer) return integer is
    function below return integer is
    begin
      return fact(n - 1);
    end function;
  begin
    if n <= 1 then
      return 1;
    end if;
    return n * below;
  end function;
begin
  process
    variable v : integer := 3;
    procedure pp is
    begin
      v := v + 1;
    end procedure;
  begin
    pp;
    report integer'image(outer("abc.init", 2)) & " " & integer'image(v);
    report integer'image(name_len("abc.init")) & " " & integer'image(name_len(""));
    report integer'image(count_ones("0110100")) & " " & integer'image(count_ones(x"FF0F"));
    report integer'image(fact(6));
    wait;
  end process;
end;
