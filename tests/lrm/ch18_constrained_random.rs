//! IEEE 1800-2023 clause 18: constrained random value generation.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check};

// §18.12: Scope randomize without the std:: prefix
#[test]
fn c18_12_scope_randomize_no_std() {
    // Known gap (§18.12): `a` reference `r=1 x=17`, xezim `r=0 x=0`
    check(
        "18.12_scope_randomize_no_std",
        include_str!("sv/18.12_scope_randomize_no_std.sv"),
        "t18_12",
        Order::Exact,
        &["T|b|r=1 x=18"],
        &["T|a|"],
    );
}

// §18.13, §18.14: srandom, get/set_randstate, urandom_range
#[test]
fn c18_13_random_stability() {
    // Not compared: raw random values (implementation-defined).
    check(
        "c18_seed",
        include_str!("sv/c18_seed.sv"),
        "c18_seed",
        Order::Exact,
        &[
            "T|18.13.3|srandom same seed equal=1",
            "T|18.13.5|get/set randstate equal=1",
            "T|18.13.1|urandom seed same=1",
            "T|18.13.2|urandom_range rev ok=1",
            "T|18.13.2|urandom_range one ok=1",
            "T|18.13|process randstate equal=1",
            "T|18.13|process srandom equal=1",
        ],
        &["T|18.14|value u1=", "T|18.14|t2 first="],
    );
}

// §18.17.7: Value-returning productions (int prod : {return v;})
#[test]
fn c18_17_7_value_production() {
    // Known gap (§18.17.7): `b`: xezim misses `after n=11`
    check(
        "18.17.7_value_production",
        include_str!("sv/18.17.7_value_production.sv"),
        "t18_17",
        Order::Exact,
        &["T|a|before"],
        &["T|b|"],
    );
}

// §18.17, §18.17.1, §18.17.5, §18.17.6, §18.17.7: randsequence productions, weights, rand join, break/return
#[test]
fn c18_17_randsequence() {
    // Known gap (§18.17.7): `18.17.7`: xezim misses `value prods n=11`
    check(
        "c18_rs",
        include_str!("sv/c18_rs.sv"),
        "c18_rs",
        Order::Exact,
        &[
            "T|18.17|struct s=FYrrr2",
            "T|18.17.6|break/return s=abcd",
            "T|18.17.7|args vals='{30, 40}",
            "T|18.17.1|weights a<b=1 total=400",
            "T|18.17.5|rand join len=4 order12=1 order34=1",
        ],
        &["T|18.17.7|value prods n=11"],
    );
}

// §18.5.1, §18.7: external constraints, dist defaults, local::
#[test]
fn c18_5_1_external_constraints() {
    check(
        "c18_ext2",
        include_str!("sv/c18_ext2.sv"),
        "c18_ext2",
        Order::Exact,
        &[
            "T|18.5.1|external r=1 x in=1 y==2x=1",
            "T|18.5.4|dist default: zero~1 onetwo~1 other_nonzero=0",
            "T|18.7|with name resolution object first a=4 (expect 4)",
            "T|18.7.1|local:: a=9 (expect 9)",
            "T|18.8|with on non-rand r=0 a=5",
        ],
        &[],
    );
}

// §18.5.3, §18.5.4, §18.5.10, §18.5.11, §18.5.13, §18.9, ...: randomize, dist, soft, rand_mode, constraint_mode, inline constraints
#[test]
fn c18_constrained_random() {
    // Not compared: dist and solve-before counts depend on the random generator (implementation-defined).
    check(
        "c18_basic",
        include_str!("sv/c18_basic.sv"),
        "c18_basic",
        Order::Exact,
        &[
            "T|18.5|violations=0 pre=200 post=200 randc first 8 all seen=11111111",
            "T|18.5.13|soft later wins v=7 r=1",
            "T|18.5.13|soft overridden v=9 r=1",
            "T|18.6|contradiction r=0 v kept=77",
            "T|18.9|cmode off r=1 v>10=1 mode=0",
            "T|18.8|rand_mode off r=0 a=0 b=9 mode=0",
            "T|18.8|all modes on r=1 a=1",
            "T|18.5.11|static constraint off via other inst r=1 v=4",
            "T|18.11|restricted list r=0 a=0 b=0",
            "T|18.11|nonrand made rand r=0 nonrand=0",
            "T|18.11.1|randomize(null) r=0",
            "T|18.11.1|randomize(null) after fix r=1",
        ],
        &["T|18.5.4|dist d1", "T|18.5.9|solve before"],
    );
}

// §18.5.8, §18.4: object arrays, with clauses, std::randomize
#[test]
fn c18_constrained_random_deep() {
    check(
        "c18_deep",
        include_str!("sv/c18_deep.sv"),
        "c18_deep",
        Order::Exact,
        &[
            "T|18.5.8|obj array r=1 v=0,2,4,6 n=8",
            "T|18.7|with nested contradiction r=0",
            "T|18.5|bidirectional x=6",
            "T|18.7|with data='{'hfa, 'hfb, 'hfc, 'hfd} kind<3=1",
            "T|18.4|logic rand no x=1 signed s<-100=1",
            "T|18.5.5|queue unique sum r=1 ok=1",
            "T|18.12|std array '{0, 1, 4, 9}",
            "T|18.12|loop var in with b4=1",
            "T|18.12|loop var in with b4=2",
            "T|18.12|loop var in with b4=3",
        ],
        &[],
    );
}

// §18.5.5, §18.5.9, §18.5.12, §18.12, §18.16: unique, ordered constraints, functions in constraints, randcase
#[test]
fn c18_constrained_random_more() {
    // Not compared: values of solutions that the constraints leave open (random).
    // Known gap (§18.12): module-scope `randomize(x)` without `std::` returns 0 (reference 1)
    check(
        "c18_more",
        include_str!("sv/c18_more.sv"),
        "c18_more",
        Order::Exact,
        &[
            "T|18.5.8|nested obj r=1 it ok=1 n ok=1 arr=3,4,5",
            "T|18.5.5|unique dup=0",
            "T|18.5.8.1|ordered r=1 bad=0",
            "T|18.5.12|fn in constraint x=5",
            "T|18.7.1|local:: x=13",
            "T|18.5|enum bad=0",
            "T|18.12|std::randomize r=1 ok=1",
            "T|18.12|std b8 ok=1",
            "T|18.12|std contradiction r=0",
            "T|18.16|randcase zero weight=0 sum=300 ratio ok=1",
            "T|18.16|randcase all zero z=0",
        ],
        &[
            "T|18.5.8.1|queue size ok=1 q=",
            "T|18.12|scope randomize r=1 x=",
            "T|18.12|scope randomize r=0",
        ],
    );
}

// §18.5.2, §18.5.13.2, §18.6.2: constraint inheritance, pre/post_randomize, randc, foreach constraints
#[test]
fn c18_constraint_inheritance() {
    // Not compared: values of solutions that the constraints leave open (random).
    // Known gap (§18.5.8.1): `18.5.8.1` reference `2D foreach r=1 m='{'{0, 1, 2}, '{3, 4, 5}}`, xezim `2D foreach r=1 m=14`
    check(
        "c18_inh",
        include_str!("sv/c18_inh.sv"),
        "c18_inh",
        Order::Exact,
        &[
            "T|18.6.2|B pre",
            "T|18.6.2|D pre",
            "T|18.6.2|B post",
            "T|18.5.2|override r=1 x<3&&x>=1=1",
            "T|18.4.2|randc cycle full=1111",
            "T|18.4.2|randc cycle full=1111",
            "T|18.4.2|randc cycle full=1111",
            "T|18.4|struct members r=1 ok=1",
            "T|18.5.13.2|disable soft r=1 in=1",
            "T|18.5.8|null rand handle r=1",
            "T|18.5.6|cond bad=0",
            "T|18.5.8.2|sum r=1 sum=40",
            "T|18.6|null obj randomize r=0",
        ],
        &[
            "T|18.5|tight r=1 a=",
            "T|18.5.8.1|size r=1 size==n 1 d=",
            "T|18.5.8.1|",
        ],
    );
}

// IEEE 1800-2017 §18.5.4, §11.4.13, Table 11-21: an `inside` / `dist` item
// `~k` is sized with the other operands; `!k` and reductions are 1-bit. None
// of them drops out of the set.
#[test]
fn c18_inside_unary_items() {
    check(
        "c18_inside_unary",
        include_str!("sv/c18_inside_unary.sv"),
        "c18iu",
        Order::Exact,
        &[
            "T|inside_unary|bitnot ok=1 n=a",
            "T|inside_unary|bitnot widened ok=1 m=fa",
            "T|inside_unary|bitnot int ok=1 x=-6",
            "T|inside_unary|lognot ok=1 en=1",
            "T|inside_unary|xor-reduce ok=1 en=0",
            "T|inside_unary|or-reduce dist ok=1 en=1",
            "T|inside_unary|pair only=1 both=1",
            "T|inside_unary|dist toggles some=1",
        ],
        &[],
    );
}
