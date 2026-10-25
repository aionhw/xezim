-- Generic types of entities (§6.5.3): one entity instantiated with
-- INTEGER, REAL and a record type, positionally and by name; the type's
-- equality in the architecture.
entity reg is
  generic (type t; init : t);
  port (clk : in bit; d : in t; q : out t := init; changed : out boolean);
end entity;
architecture a of reg is
begin
  process (clk) begin
    if clk = '1' then
      changed <= d /= q;
      q <= d;
    end if;
  end process;
end architecture;

entity generic_types is end entity;
architecture a of generic_types is
  type pair is record x, y : integer; end record;
  signal clk : bit := '0';
  signal di, qi : integer := 0;
  signal dr, qr : real := 0.0;
  signal dp, qp : pair := (0, 0);
  signal ci, cr, cp : boolean;
begin
  ri: entity work.reg generic map (integer, 3) port map (clk, di, qi, ci);
  rr: entity work.reg generic map (t => real, init => 0.5) port map (clk, dr, qr, cr);
  rp: entity work.reg generic map (t => pair, init => (1, 2)) port map (clk => clk, d => dp, q => qp, changed => cp);
  process begin
    report "qi=" & integer'image(qi) & " qr=" & real'image(qr) & " qp=" & integer'image(qp.x) & "," & integer'image(qp.y);
    di <= 5; dr <= 2.5; dp <= (1, 2); wait for 1 ns; clk <= '1'; wait for 1 ns;
    report "qi=" & integer'image(qi) & " qr=" & real'image(qr) & " qp=" & integer'image(qp.x) & "," & integer'image(qp.y)
      & " changed " & boolean'image(ci) & boolean'image(cr) & boolean'image(cp);
    wait;
  end process;
end architecture;
