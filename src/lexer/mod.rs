//! Lexer module.

pub mod token;
pub mod scanner;

pub use token::{Token, TokenKind};
pub use scanner::Lexer;
