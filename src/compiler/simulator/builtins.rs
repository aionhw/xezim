//! Built-in method classification, string-to-number conversion, real math arguments,
//! stochastic queues and aggregate comparison outcomes.

use super::*;

/// §20.15 stochastic-analysis queue state for `$q_initialize`/`$q_add`/
/// `$q_remove`/`$q_full`/`$q_exam`.
///
/// `q_type`: 1 = FIFO (§20.15.1 Table 20-9), 2 = LIFO. Each entry records the
/// user `job_id`, `inform_id`, and the sim-time tick at which it was added so
/// `$q_exam` can compute the wait-time statistics of Table 20-10. Statistics
/// accumulate across the queue's lifetime: `max_len_seen` is the high-water
/// mark (code 3); inter-arrival samples feed code 2; per-job waits feed codes
/// 4/5/6. `i64::MIN`/sentinels encode "no sample yet" (the LRM leaves these
/// stats implementation-defined; many tools report -1 in that case).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct StochasticQueue {
    pub(super) q_type: i64,
    pub(super) max_length: i64,
    pub(super) entries: VecDeque<(i64, i64, i64)>, // (job_id, inform_id, arrival_tick)
    // lifetime statistics
    pub(super) arrivals: u64,
    pub(super) last_arrival_tick: Option<i64>,
    pub(super) sum_interarrival: i128,
    pub(super) max_len_seen: usize,
    pub(super) shortest_wait_ever: Option<i64>,
    pub(super) longest_wait_queued: Option<i64>,
    pub(super) sum_wait_removed: i128,
    pub(super) count_wait_removed: u64,
}

/// Evaluate the single real argument of a §20.8.2 one-operand real math
/// function (`$sin`, `$sqrt`, …). Defaults to 0.0 when absent.
#[inline]
pub(super) fn real1<F>(args: &[Expression], eval: &mut F) -> f64
where
    F: FnMut(&Expression) -> Value,
{
    args.first().map(|a| eval(a).to_f64()).unwrap_or(0.0)
}

/// Evaluate the two real arguments of a §20.8.2 two-operand real math function
/// (`$atan2(y,x)`, `$hypot(x,y)`), defaulting missing operands to 0.0.
#[inline]
pub(super) fn real2<F>(args: &[Expression], eval: &mut F) -> (f64, f64)
where
    F: FnMut(&Expression) -> Value,
{
    (
        args.first().map(|a| eval(a).to_f64()).unwrap_or(0.0),
        args.get(1).map(|a| eval(a).to_f64()).unwrap_or(0.0),
    )
}

/// §6.16.9 string→number conversion: scan LEADING digit/underscore
/// characters and stop at the first other character. No whitespace skip —
/// `"  512".atoi()` is 0 (the C-atoi trim diverged from the reference) —
/// and `'+'` is not consumed. `"12_34"` scans to 1234. A single leading
/// `'-'` negates. For the non-decimal conversions x/z/? are legal 4-STATE
/// digits: `"3fZ".atohex()` is 12'h3fZ (a partially-z integer), matching
/// the reference, not a scan-stop. Result is the signed 32-bit `integer`
/// the LRM specifies.
pub(super) fn sv_ato_value(text: &str, radix: u32) -> Value {
    let mut it = text.chars().peekable();
    let neg = it.peek() == Some(&'-');
    if neg {
        it.next();
    }
    let mut digits = String::new();
    for c in it {
        if c == '_' {
            continue;
        }
        let is_digit = c.to_digit(radix).is_some()
            || (radix != 10 && matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'));
        if !is_digit {
            break;
        }
        digits.push(c);
    }
    let mut v = if digits.is_empty() {
        Value::zero(32)
    } else {
        Value::from_str_radix(&digits, radix, 32)
    };
    if neg && !v.has_xz() {
        let n = v.to_u64().unwrap_or(0) as i64;
        v = Value::from_u64((-n) as u64, 32);
    }
    v.is_signed = true;
    v
}

/// Builtin collection / string / enum methods `eval_builtin_method`
/// implements, classified once per call from the method name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BuiltinM {
    First,
    Last,
    Num,
    Next,
    Prev,
    Size,
    Len,
    Getc,
    Substr,
    PushBack,
    PushFront,
    PopFront,
    PopBack,
    Sort,
    Rsort,
    Reverse,
    Shuffle,
    Insert,
    Sum,
    Product,
    And,
    Or,
    Xor,
    Min,
    Max,
    Unique,
    UniqueIndex,
    Find,
    FindFirst,
    FindLast,
    FindIndex,
    FindFirstIndex,
    FindLastIndex,
    Exists,
    Delete,
    Other,
}

impl BuiltinM {
    #[inline]
    pub(super) fn classify(mname: &str) -> BuiltinM {
        match mname {
            "first" => BuiltinM::First,
            "last" => BuiltinM::Last,
            "num" => BuiltinM::Num,
            "next" => BuiltinM::Next,
            "prev" => BuiltinM::Prev,
            "size" => BuiltinM::Size,
            "len" => BuiltinM::Len,
            "getc" => BuiltinM::Getc,
            "substr" => BuiltinM::Substr,
            "push_back" => BuiltinM::PushBack,
            "push_front" => BuiltinM::PushFront,
            "pop_front" => BuiltinM::PopFront,
            "pop_back" => BuiltinM::PopBack,
            "sort" => BuiltinM::Sort,
            "rsort" => BuiltinM::Rsort,
            "reverse" => BuiltinM::Reverse,
            "shuffle" => BuiltinM::Shuffle,
            "insert" => BuiltinM::Insert,
            "sum" => BuiltinM::Sum,
            "product" => BuiltinM::Product,
            "and" => BuiltinM::And,
            "or" => BuiltinM::Or,
            "xor" => BuiltinM::Xor,
            "min" => BuiltinM::Min,
            "max" => BuiltinM::Max,
            "unique" => BuiltinM::Unique,
            "unique_index" => BuiltinM::UniqueIndex,
            "find" => BuiltinM::Find,
            "find_first" => BuiltinM::FindFirst,
            "find_last" => BuiltinM::FindLast,
            "find_index" => BuiltinM::FindIndex,
            "find_first_index" => BuiltinM::FindFirstIndex,
            "find_last_index" => BuiltinM::FindLastIndex,
            "exists" => BuiltinM::Exists,
            "delete" => BuiltinM::Delete,
            _ => BuiltinM::Other,
        }
    }
}

/// The outcome of comparing an aggregate element by element, with another
/// aggregate or with an assignment pattern (IEEE 1800-2023 §11.4.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PatCmp {
    Equal,
    Unequal,
    /// An x/z element or member leaves the relation open: `==` and `!=`
    /// are x (§11.4.5).
    Unknown,
}

impl PatCmp {
    /// Fold the next element's outcome in: any unequal element decides;
    /// otherwise an unknown one makes the result unknown.
    pub(super) fn and(self, next: PatCmp) -> PatCmp {
        match (self, next) {
            (PatCmp::Unequal, _) | (_, PatCmp::Unequal) => PatCmp::Unequal,
            (PatCmp::Unknown, _) | (_, PatCmp::Unknown) => PatCmp::Unknown,
            _ => PatCmp::Equal,
        }
    }

    /// The 1-bit result of an equality (`want_eq`) or inequality operator.
    pub(super) fn value(self, want_eq: bool) -> Value {
        match self {
            PatCmp::Equal => Value::from_u64(want_eq as u64, 1),
            PatCmp::Unequal => Value::from_u64(!want_eq as u64, 1),
            PatCmp::Unknown => Value::new(1),
        }
    }
}
