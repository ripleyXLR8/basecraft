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
pub mod schema;
pub mod value;

pub use ast::Expr;
pub use error::{EvalError, ParseError};
pub use eval::{Context, CustomFunction, Evaluator, MemoryContext};
pub use parser::{parse, parse_with};
pub use value::Value;

/// Parses and evaluates `src`.
pub fn evaluate(src: &str, ctx: &mut dyn Context) -> Result<Value, EvalError> {
    let e = parse_with(src, &|n| ctx.custom_function(n).is_some())?;
    Evaluator::new(ctx).eval(&e)
}

/// Like [`evaluate`], with the code EvaluationError would report for a
/// result (0, or an error a function absorbed: `Count ( 1 ; 1/0 )` is 1
/// with error 15).
pub fn evaluate_with_code(src: &str, ctx: &mut dyn Context) -> Result<(Value, u32), EvalError> {
    let e = parse_with(src, &|n| ctx.custom_function(n).is_some())?;
    let mut ev = Evaluator::new(ctx);
    let v = ev.eval(&e)?;
    Ok((v, ev.sticky.map_or(0, |e| e.code)))
}

/// The result as FileMaker displays it: text, or `?` on any error.
pub fn eval_text(src: &str, ctx: &mut dyn Context) -> String {
    evaluate(src, ctx)
        .map(|v| v.to_text())
        .unwrap_or_else(|_| "?".into())
}

#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod tests;
