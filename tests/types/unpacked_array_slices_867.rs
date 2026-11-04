//! §7.4.6 (§7.4.5 in 1800-2023), §7.6, §10.3, §10.4.2, §13.5, §23.3.3 (#867):
//! a slice of an unpacked array, `a[l:r]`, `a[b +: w]` or `a[b -: w]`, is an
//! unpacked array of constant size whose base may vary at run time, and
//! pairs its elements left to right with the other side of an assignment.
//! It was read and written wrongly almost everywhere it can appear: as the
//! source or the target of a blocking or nonblocking assignment, slice to
//! slice in opposite directions, as a row or part of a row of a
//! two-dimensional array, as an argument, in `$size`, in a continuous
//! assignment and on a port. A whole-array or slice read in `always_comb`
//! also had no sensitivity at all. The expected lines are the reference
//! simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| {
            m.starts_with("T|")
                || m.starts_with("PASS")
                || m.starts_with("FAIL")
                || m.starts_with("ALL")
        })
        .collect()
}

/// The reporter's seven places, one check each.
#[test]
fn reporter_minimal_cases_pass() {
    let out = lines(include_str!("unpacked_array_slices_min_867.sv"));
    assert!(!out.iter().any(|l| l.starts_with("FAIL")), "{out:#?}");
    assert_eq!(out.last().map(String::as_str), Some("ALL PASS"), "{out:#?}");
}

/// Thirty-one shapes in an `initial` block: bases, directions, overlap,
/// rows, nonblocking, arguments, `$size`, continuous assignments, ports.
#[test]
fn slice_shapes_match_the_reference() {
    let out = lines(include_str!("unpacked_array_slices_867.sv"));
    let want = [
        "T|01 b=a[7-:W]      17 16 15 14 13 12",
        "T|02 b=a[7:2]       17 16 15 14 13 12",
        "T|03 b=a[i+:W]      16 15 14 13 12 11",
        "T|04 b=a[j-:W]      17 16 15 14 13 12",
        "T|05 b=w[63-:W]     3f 3e 3d 3c 3b 3a",
        "T|06 a[0+:W]=b      17 16 a5 a4 a3 a2 a1 a0",
        "T|07 a[7:2]=b       a5 a4 a3 a2 a1 a0 11 10",
        "T|08 a[j-:W]=b      a5 a4 a3 a2 a1 a0 11 10",
        "T|09 e6=d[2+:6]     32 33 34 35 36 37",
        "T|10 e6=d[2:7]      32 33 34 35 36 37",
        "T|11 d[1+:6]=e6     30 50 51 52 53 54 55 37",
        "T|12 b=d[1+:6]      31 32 33 34 35 36",
        "T|13 a[0+:6]=d[2+:6] 17 16 32 33 34 35 36 37",
        "T|14 a[2+:6]=a[0+:6] 15 14 13 12 11 10 11 10",
        "T|15 m[1]=w[58+:W]  3f 3e 3d 3c 3b 3a",
        "T|16 c[0+:2]=c2     9b 9a 99 98 97 96 / 95 94 93 92 91 90",
        "T|17 m[1][0+:3]=x3  cb ca c9 00 00 00",
        "T|18 x3=m[1][1+:3]  c9 c8 c7",
        "T|19 a[0+:W]<=b     17 16 a5 a4 a3 a2 a1 a0",
        "T|20 b<=a[7-:W]     17 16 15 14 13 12",
        "T|21 m[0]<=w[i+:W]  06 05 04 03 02 01",
        "T|22 leftmost(w[58+:W]) 3f rightmost(a[j-:W]) 12",
        "T|23 fill(a[0+:W])  17 16 e5 e4 e3 e2 e1 e0",
        "T|24 fill(d[2:7])   30 31 e5 e4 e3 e2 e1 e0",
        "T|25 size 6 6 6",
        "T|26 cw=w[58+:W]     3f 3e 3d 3c 3b 3a",
        "T|27 cv=w[k+:W]      0f 0e 0d 0c 0b 0a",
        "T|28 cv k=20         19 18 17 16 15 14",
        "T|29 ct[6:1]=b       xx a5 a4 a3 a2 a1 a0 xx",
        "T|30 ports po=80 7f 7e 7d 7c 7b pres=76 71 pv=xx xx xx xx xx 80",
        "T|31 port k=12 pv=7d 7c 7b 7a 79 78",
    ];
    assert_eq!(out, want);
}

/// Slices in clocked and combinational processes, and of `int` arrays.
#[test]
fn slices_in_processes_match_the_reference() {
    let out = lines(include_str!("unpacked_array_slices_procs_867.sv"));
    let want = [
        "T|ff a=17 a5 a4 a3 a2 a1 a0 10",
        "T|ff r=16 15 14 13 12 11",
        "T|comb s=a5 a4 a3 a2 a1 a0 a0 10",
        "T|int ic=100 101 102 103 ia=101 0 -1 -2 -3 106",
        "T|ff2 r=17 a5 a4 a3 a2 a1 s7=ff",
        "T|int2 ic=-1 -2 -3 106",
    ];
    assert_eq!(out, want);
}
