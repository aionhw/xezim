-- TEXTIO reads (§16.4) as GHDL does them: READ of INTEGER, REAL,
-- BOOLEAN, TIME, BIT, BIT_VECTOR, CHARACTER and STRING with GOOD, SREAD,
-- HREAD and OREAD: what each accepts, what a successful read leaves of the
-- line (its indices), that a failed one leaves the line (and a scalar
-- value gets its subtype's leftmost value); STD_LOGIC_1164's READ and
-- HREAD; a READ without GOOD that fails stops with GHDL's assertion.
use std.textio.all;
library ieee;
use ieee.std_logic_1164.all;

entity textio_read is
end entity;

architecture a of textio_read is
  procedure show(variable l : inout line; what : string) is
  begin
    if l = null then
      report what & " null";
    else
      report what & " [" & l.all & "] " & integer'image(l'left) & " " & integer'image(l'right);
    end if;
  end procedure;
begin
  process
    variable l : line;
    variable i : integer;
    variable r : real;
    variable b : boolean;
    variable c : character;
    variable bv : bit_vector(3 downto 0);
    variable bv6 : bit_vector(5 downto 0);
    variable s : string(1 to 5);
    variable t : time;
    variable g : boolean;
    variable n : natural;
    variable slv : std_logic_vector(7 downto 0);
    procedure ri(str : string) is begin
      deallocate(l); l := new string'(str); i := -999; read(l, i, g);
      report "int '" & str & "' -> " & integer'image(i) & " " & boolean'image(g); show(l, "  ");
    end procedure;
    procedure rr(str : string) is begin
      deallocate(l); l := new string'(str); r := -999.0; read(l, r, g);
      report "real '" & str & "' -> " & real'image(r) & " " & boolean'image(g); show(l, "  ");
    end procedure;
    procedure rb(str : string) is begin
      deallocate(l); l := new string'(str); b := false; read(l, b, g);
      report "bool '" & str & "' -> " & boolean'image(b) & " " & boolean'image(g); show(l, "  ");
    end procedure;
    procedure rt(str : string) is begin
      deallocate(l); l := new string'(str); t := 0 fs; read(l, t, g);
      report "time '" & str & "' -> " & time'image(t) & " " & boolean'image(g); show(l, "  ");
    end procedure;
    procedure rbv(str : string) is begin
      deallocate(l); l := new string'(str); bv := "0000"; read(l, bv, g);
      report "bv '" & str & "' -> " & to_string(bv) & " " & boolean'image(g); show(l, "  ");
    end procedure;
    procedure rh(str : string) is begin
      deallocate(l); l := new string'(str); bv6 := "000000"; hread(l, bv6, g);
      report "hread6 '" & str & "' -> " & to_string(bv6) & " " & boolean'image(g); show(l, "  ");
      deallocate(l); l := new string'(str); bv6 := "000000"; oread(l, bv6, g);
      report "oread6 '" & str & "' -> " & to_string(bv6) & " " & boolean'image(g); show(l, "  ");
    end procedure;
    procedure rs(str : string) is begin
      deallocate(l); l := new string'(str); s := "....."; read(l, s, g);
      report "str '" & str & "' -> " & s & " " & boolean'image(g); show(l, "  ");
      deallocate(l); l := new string'(str); s := "....."; sread(l, s, n);
      report "sread '" & str & "' -> " & s & " " & integer'image(n); show(l, "  ");
    end procedure;
    procedure rc(str : string) is begin
      deallocate(l); l := new string'(str); c := '.'; read(l, c, g);
      report "char '" & str & "' -> " & c & " " & boolean'image(g); show(l, "  ");
    end procedure;
  begin
    ri("  42 abc"); ri("+5"); ri("1_000"); ri("12abc"); ri("  abc"); ri("   "); ri("  -");
    ri("2147483647"); ri("-2147483648"); ri("1.5"); ri(HT & "7");
    rr("1.5e2"); rr("-.5"); rr("3"); rr("1.0E-2"); rr("  2.5x"); rr("1."); rr("-1.25E+1 z");
    rb("TRUE"); rb("truex"); rb(" false;"); rb("f");
    rt("15 ns"); rt("1.5 us"); rt("5ns"); rt("10 NS"); rt("7 sec"); rt("3 xs"); rt("-4 fs"); rt("1 hr"); rt("0.05 ns"); rt("15 nsx");
    rbv("1_0_1_0"); rbv(" 1010z"); rbv("101"); rbv("10101");
    rh("3F"); rh("7F"); rh("2_a"); rh("  77 x"); rh("03F");
    rs("abc"); rs("abcdefg"); rs("  ab cd"); rs("abcde"); rs(" abcde");
    rc(" x"); rc("y");
    deallocate(l);
    l := new string'("  1x");
    read(l, bv(0), g);
    report "bit " & to_string(bv(0)) & " " & boolean'image(g);
    deallocate(l);
    l := new string'("01ZX10LH A5 0110");
    read(l, slv);
    report "slv " & to_string(slv);
    hread(l, slv);
    report "hread slv " & to_hstring(slv) & " " & to_string(slv);
    read(l, bv);
    report "bv " & to_string(bv) & " " & to_hstring(bv) & " " & to_ostring(bv);
    deallocate(l);
    l := new string'("notanumber");
    read(l, i);
    report "not reached";
    wait;
  end process;
end architecture;
