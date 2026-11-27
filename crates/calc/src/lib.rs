//! FileMaker calculation language: lexer, parser, pretty-printer and
//! evaluator.
//!
//! ```
//! let mut ctx = calc::MemoryContext::default();
//! assert_eq!(calc::eval_text("Left ( \"Hello\" ; 2 )", &mut ctx), "He");
//! ```

pub mod ast;
pub mod error;
pub mod eval;
pub mod functions;
pub mod lexer;
pub mod num;
pub mod parser;
pub mod value;

pub use ast::Expr;
pub use error::{EvalError, ParseError};
pub use eval::{Context, Evaluator, MemoryContext};
pub use parser::parse;
pub use value::Value;

/// Parses and evaluates `src`.
pub fn evaluate(src: &str, ctx: &mut dyn Context) -> Result<Value, EvalError> {
    let e = parse(src)?;
    Evaluator::new(ctx).eval(&e)
}

/// The result as FileMaker displays it: text, or `?` on any error.
pub fn eval_text(src: &str, ctx: &mut dyn Context) -> String {
    evaluate(src, ctx)
        .map(|v| v.to_text())
        .unwrap_or_else(|_| "?".into())
}

#[cfg(test)]
mod tests;
