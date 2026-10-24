-- Functions and procedures (§4): overloading by parameter type, default
-- parameters, out/inout parameters, recursion, local variables, a package
-- of subprograms with a deferred constant.
package util is
  constant base : integer;
  function twice(x : integer) return integer;
  function twice(x : real) return real;
  function fact(n : natural) return natural;
  procedure swap(a, b : inout integer);
  procedure bump(x : inout integer; by : integer := 1);
end package;
package body util is
  constant base : integer := 40;
  function twice(x : integer) return integer is begin return 2 * x; end function;
  function twice(x : real) return real is begin return 2.0 * x; end function;
  function fact(n : natural) return natural is
  begin
    if n <= 1 then return 1; end if;
    return n * fact(n - 1);
  end function;
  procedure swap(a, b : inout integer) is
    variable t : integer;
  begin
    t := a; a := b; b := t;
  end procedure;
  procedure bump(x : inout integer; by : integer := 1) is
  begin
    x := x + by;
  end procedure;
end package body;

use work.util.all;
entity subprograms is end entity;
architecture a of subprograms is
  function local_add(a, b : integer) return integer is
    variable s : integer := 0;
  begin
    s := a + b;
    return s;
  end function;
begin
  process
    variable x, y : integer := 0;
  begin
    x := 3; y := 9;
    swap(x, y);
    report "swap: x=" & integer'image(x) & " y=" & integer'image(y);
    bump(x);
    bump(x, by => 10);
    report "bump: x=" & integer'image(x);
    report "twice: " & integer'image(twice(21)) & " " & real'image(twice(1.25));
    report "fact(10)=" & integer'image(fact(10)) & " base=" & integer'image(base);
    report "local_add=" & integer'image(local_add(base, 2));
    wait;
  end process;
end architecture;
