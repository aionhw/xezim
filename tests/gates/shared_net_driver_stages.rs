//! IEEE 1800-2023 §28.16, §28.12, §29.3: every driver of a net delays its own
//! transitions, and the net resolves the delayed values. The fall delay of a
//! primitive was kept under the net's name, so beside another driver it
//! delayed the whole net; a net whose drivers were delayed tristate
//! primitives read as the last of them alone, undelayed; a primitive with a
//! zero rise delay fell at once; and a UDP output beside another driver was
//! not resolved at all. Each transcript below is the reference simulator's.

fn monitor(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.contains(" | "))
        .collect()
}

/// §28.6, §28.7, §28.16: two delayed tristate primitives on one net, a
/// delayed tristate or MOS primitive beside a continuous assignment.
#[test]
fn delayed_tristate_drivers_resolve_with_their_own_delays() {
    let expected = [
        "0 a=x b=x en1=x en2=x | bus=x bus2=x mix=x mixd=x mos=x inv=x",
        "1 a=1 b=0 en1=1 en2=0 | bus=x bus2=x mix=x mixd=x mos=x inv=x",
        "3 a=1 b=0 en1=1 en2=0 | bus=1 bus2=1 mix=1 mixd=x mos=x inv=x",
        "4 a=1 b=0 en1=1 en2=0 | bus=1 bus2=1 mix=1 mixd=x mos=x inv=0",
        "21 a=0 b=0 en1=1 en2=0 | bus=1 bus2=1 mix=1 mixd=x mos=x inv=0",
        "23 a=0 b=0 en1=1 en2=0 | bus=0 bus2=1 mix=0 mixd=x mos=x inv=x",
        "26 a=0 b=0 en1=1 en2=0 | bus=0 bus2=0 mix=0 mixd=0 mos=0 inv=x",
        "41 a=0 b=0 en1=1 en2=1 | bus=0 bus2=0 mix=0 mixd=0 mos=0 inv=x",
        "43 a=0 b=0 en1=1 en2=1 | bus=0 bus2=0 mix=0 mixd=0 mos=0 inv=0",
        "61 a=1 b=0 en1=1 en2=1 | bus=0 bus2=0 mix=0 mixd=0 mos=0 inv=0",
        "62 a=1 b=0 en1=1 en2=1 | bus=0 bus2=x mix=0 mixd=x mos=x inv=0",
        "63 a=1 b=0 en1=1 en2=1 | bus=x bus2=x mix=x mixd=x mos=x inv=0",
        "81 a=1 b=0 en1=0 en2=1 | bus=x bus2=x mix=x mixd=x mos=x inv=0",
        "82 a=1 b=0 en1=0 en2=1 | bus=x bus2=0 mix=x mixd=0 mos=0 inv=0",
        "83 a=1 b=0 en1=0 en2=1 | bus=0 bus2=0 mix=0 mixd=0 mos=0 inv=0",
        "101 a=1 b=1 en1=0 en2=1 | bus=0 bus2=0 mix=1 mixd=0 mos=0 inv=0",
        "102 a=1 b=1 en1=0 en2=1 | bus=0 bus2=0 mix=1 mixd=1 mos=1 inv=1",
        "103 a=1 b=1 en1=0 en2=1 | bus=1 bus2=0 mix=1 mixd=1 mos=1 inv=1",
        "104 a=1 b=1 en1=0 en2=1 | bus=1 bus2=1 mix=1 mixd=1 mos=1 inv=1",
        "121 a=1 b=1 en1=0 en2=0 | bus=1 bus2=1 mix=z mixd=1 mos=1 inv=1",
        "123 a=1 b=1 en1=0 en2=0 | bus=z bus2=z mix=z mixd=1 mos=1 inv=1",
        "124 a=1 b=1 en1=0 en2=0 | bus=z bus2=z mix=z mixd=1 mos=1 inv=x",
        "141 a=1 b=1 en1=1 en2=1 | bus=z bus2=z mix=1 mixd=1 mos=1 inv=x",
        "142 a=1 b=1 en1=1 en2=1 | bus=z bus2=1 mix=1 mixd=1 mos=1 inv=x",
        "143 a=1 b=1 en1=1 en2=1 | bus=1 bus2=1 mix=1 mixd=1 mos=1 inv=1",
        "161 a=1 b=0 en1=1 en2=1 | bus=1 bus2=1 mix=x mixd=1 mos=1 inv=1",
        "162 a=1 b=0 en1=1 en2=1 | bus=1 bus2=1 mix=x mixd=x mos=x inv=0",
        "163 a=1 b=0 en1=1 en2=1 | bus=x bus2=x mix=x mixd=x mos=x inv=0",
    ];
    assert_eq!(
        monitor(include_str!("shared_net_tristate_delays.sv")),
        expected
    );
}

/// §28.16: `#(0, 5)` rises at once and falls after 5, alone on its net and
/// beside another driver.
#[test]
fn zero_rise_primitive_falls_after_its_fall_delay() {
    let expected = [
        "0 a=x en=x | alone=x trs=x both=x mixed=x",
        "1 a=1 en=1 | alone=1 trs=1 both=1 mixed=x",
        "2 a=1 en=1 | alone=1 trs=1 both=1 mixed=1",
        "21 a=0 en=1 | alone=1 trs=1 both=x mixed=1",
        "22 a=0 en=1 | alone=1 trs=1 both=x mixed=x",
        "26 a=0 en=1 | alone=0 trs=0 both=0 mixed=0",
        "41 a=1 en=1 | alone=1 trs=1 both=1 mixed=x",
        "42 a=1 en=1 | alone=1 trs=1 both=1 mixed=1",
        "61 a=1 en=0 | alone=1 trs=z both=1 mixed=1",
        "81 a=0 en=0 | alone=1 trs=z both=x mixed=1",
        "82 a=0 en=0 | alone=1 trs=z both=x mixed=x",
        "86 a=0 en=0 | alone=0 trs=z both=0 mixed=0",
        "101 a=0 en=1 | alone=0 trs=z both=0 mixed=0",
        "106 a=0 en=1 | alone=0 trs=0 both=0 mixed=0",
    ];
    assert_eq!(
        monitor(include_str!("zero_rise_primitive_delays.sv")),
        expected
    );
}

/// §29.3, §28.12: a UDP output is one driver of its net; its instance delay
/// is its own.
#[test]
fn udp_output_resolves_with_the_other_drivers_of_its_net() {
    let expected = [
        "0 a=x b=x g=x | plain=x slow=x pair=x two=x seq=x alone=x",
        "1 a=1 b=1 g=1 | plain=1 slow=x pair=x two=x seq=1 alone=x",
        "2 a=1 b=1 g=1 | plain=1 slow=x pair=1 two=x seq=1 alone=1",
        "3 a=1 b=1 g=1 | plain=1 slow=1 pair=1 two=x seq=1 alone=1",
        "4 a=1 b=1 g=1 | plain=1 slow=1 pair=1 two=1 seq=1 alone=1",
        "21 a=0 b=1 g=1 | plain=x slow=1 pair=1 two=1 seq=x alone=1",
        "23 a=0 b=1 g=1 | plain=x slow=x pair=1 two=x seq=x alone=1",
        "26 a=0 b=1 g=1 | plain=x slow=x pair=x two=x seq=x alone=0",
        "41 a=1 b=1 g=1 | plain=1 slow=x pair=x two=x seq=1 alone=0",
        "42 a=1 b=1 g=1 | plain=1 slow=x pair=1 two=x seq=1 alone=1",
        "43 a=1 b=1 g=1 | plain=1 slow=1 pair=1 two=1 seq=1 alone=1",
        "61 a=1 b=0 g=1 | plain=x slow=1 pair=1 two=1 seq=x alone=1",
        "62 a=1 b=0 g=1 | plain=x slow=x pair=x two=1 seq=x alone=1",
        "64 a=1 b=0 g=1 | plain=x slow=x pair=x two=x seq=x alone=1",
        "81 a=1 b=0 g=0 | plain=x slow=x pair=x two=x seq=x alone=1",
        "101 a=0 b=0 g=0 | plain=0 slow=x pair=x two=x seq=x alone=1",
        "103 a=0 b=0 g=0 | plain=0 slow=0 pair=x two=0 seq=x alone=1",
        "106 a=0 b=0 g=0 | plain=0 slow=0 pair=0 two=0 seq=x alone=0",
        "121 a=0 b=z g=0 | plain=0 slow=0 pair=0 two=0 seq=1 alone=0",
        "124 a=0 b=z g=0 | plain=0 slow=0 pair=0 two=x seq=1 alone=0",
        "141 a=0 b=z g=1 | plain=0 slow=0 pair=0 two=x seq=0 alone=0",
    ];
    assert_eq!(monitor(include_str!("shared_net_udp_outputs.sv")), expected);
}
