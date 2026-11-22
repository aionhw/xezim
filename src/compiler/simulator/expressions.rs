//! Simulator expressions.

use super::collection_methods::PatternCmpMode;
use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

mod collection_reads;
mod evaluation;
mod member_access;
mod plain_reads;
mod signedness;
