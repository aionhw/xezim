-- VHDL-2019 language additions: conditional expressions (§9.1), sequential
-- block statements, empty record types, and subtypes of variables and
-- signals inferred from their initial values.
use std.textio.all;
entity lang19 is end entity;
architecture a of lang19 is
  type pair is record
    x, y : integer;
  end record;
  type nothing is record
  end record;
  type iv is array (natural range <>) of integer;
  constant pa : pair := (1, 2);
  constant pb : pair := (3, 4);
  function pick(c : boolean; a, b : pair) return pair is
  begin
    return a when c else b;
  end function;
  function len(v : bit_vector) return natural is
  begin
    return v'length;
  end function;
  -- A conditional expression as a signal's initial value and an actual.
  signal s : integer := 5 when pa.x = 1 else 6;
  signal bv : bit_vector := "0110";
  signal e : nothing;
begin
  process
    variable l : line;
    variable i : integer := 3;
    variable v : bit_vector := "10101";
    variable arr : iv(0 to 3) := (10, 20, 30, 40);
    variable n : integer;
    variable p : pair;
  begin
    -- Scalars, vectors, records and strings; only the chosen value is
    -- evaluated (arr(i) out of range is not read).
    n := arr(i) when i <= arr'high else -1;
    write(l, "n=" & integer'image(n));
    i := 7;
    n := arr(i) when i <= arr'high else -1;
    write(l, " n=" & integer'image(n));
    v := "11111" when n < 0 else "00000";
    write(l, " v=" & to_string(v));
    p := pick(false, pa, pb);
    write(l, " p=" & integer'image(p.x) & integer'image(p.y));
    write(l, " s=" & integer'image(s));
    writeline(output, l);
    -- Inferred subtypes.
    write(l, "v'range=" & integer'image(v'left) & " to " & integer'image(v'right)
      & " bv'length=" & integer'image(bv'length));
    writeline(output, l);
    -- A sequential block: its variable is initialised each time it runs.
    for k in 1 to 3 loop
      blk : block is
        variable acc : integer := k * 10;
        constant twice : integer := 2 * k;
      begin
        acc := acc + twice;
        write(l, "blk" & integer'image(k) & "=" & integer'image(acc));
        inner : block
          variable z : integer;
        begin
          z := acc + 1;
          write(l, " inner=" & integer'image(z));
        end block inner;
        writeline(output, l);
      end block blk;
    end loop;
    wait;
  end process;
end architecture;
