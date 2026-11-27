//! Evaluator: operators, name resolution, Let/While scopes, and the
//! [`Context`] the host implements to supply fields, variables and `Get()`.

use crate::ast::{BinOp, Expr, UnOp};
use crate::error::{EvalError, code};
use crate::functions;
use crate::num::{self, Num};
use crate::value::{self, Value};
use bigdecimal::ToPrimitive;
use std::cmp::Ordering;
use std::collections::HashMap;

pub type R = Result<Value, EvalError>;

/// What the engine needs from its host. Field and table names arrive as
/// written in the formula; FileMaker treats them case-insensitively.
pub trait Context {
    /// One repetition (1-based) of a field.
    fn field(&mut self, table: Option<&str>, name: &str, repetition: usize) -> R {
        let _ = repetition;
        Err(EvalError::new(
            code::FIELD_MISSING,
            format!(
                "field {}{name} is missing",
                table.map(|t| format!("{t}::")).unwrap_or_default()
            ),
        ))
    }

    /// Every value a field reference stands for in an aggregate: the related
    /// records' values for a related field, or the repetitions of a repeating
    /// field. Defaults to the single value.
    fn field_values(&mut self, table: Option<&str>, name: &str) -> Result<Vec<Value>, EvalError> {
        Ok(vec![self.field(table, name, 1)?])
    }

    /// `Get(flag)`; `None` falls back to the engine's built-in flags.
    fn get(&mut self, flag: &str) -> Option<Value> {
        let _ = flag;
        None
    }

    /// `$local` and `$$global` variables; `name` includes the dollar signs.
    fn variable(&mut self, name: &str, repetition: usize) -> Value;
    fn set_variable(&mut self, name: &str, repetition: usize, value: Value);
}

/// In-memory context for tests and tools. Keys are lower-case
/// `table::field` (or just `field`).
#[derive(Default, Debug)]
pub struct MemoryContext {
    pub fields: HashMap<String, Vec<Value>>,
    pub flags: HashMap<String, Value>,
    pub vars: HashMap<(String, usize), Value>,
}

fn field_key(table: Option<&str>, name: &str) -> String {
    match table {
        Some(t) => format!("{t}::{name}").to_lowercase(),
        None => name.to_lowercase(),
    }
}

impl MemoryContext {
    pub fn with_field(mut self, name: &str, values: Vec<Value>) -> Self {
        self.fields.insert(name.to_lowercase(), values);
        self
    }
}

impl Context for MemoryContext {
    fn field(&mut self, table: Option<&str>, name: &str, repetition: usize) -> R {
        match self.fields.get(&field_key(table, name)) {
            Some(v) => Ok(v
                .get(repetition.wrapping_sub(1))
                .cloned()
                .unwrap_or_else(Value::empty)),
            None => Err(EvalError::new(
                code::FIELD_MISSING,
                format!("field {name} is missing"),
            )),
        }
    }

    fn field_values(&mut self, table: Option<&str>, name: &str) -> Result<Vec<Value>, EvalError> {
        self.fields
            .get(&field_key(table, name))
            .cloned()
            .ok_or_else(|| EvalError::new(code::FIELD_MISSING, format!("field {name} is missing")))
    }

    fn get(&mut self, flag: &str) -> Option<Value> {
        self.flags.get(&flag.to_lowercase()).cloned()
    }

    fn variable(&mut self, name: &str, repetition: usize) -> Value {
        self.vars
            .get(&(name.to_lowercase(), repetition))
            .cloned()
            .unwrap_or_else(Value::empty)
    }

    fn set_variable(&mut self, name: &str, repetition: usize, value: Value) {
        self.vars.insert((name.to_lowercase(), repetition), value);
    }
}

pub struct Evaluator<'a> {
    pub ctx: &'a mut dyn Context,
    scopes: Vec<HashMap<String, Value>>,
    /// Decimal places for division and math functions (SetPrecision).
    pub scale: i64,
    /// Iteration limit for While (SetRecursion).
    pub max_iterations: u64,
    /// Nesting of Evaluate() calls, bounded so self-evaluating text fails
    /// instead of overflowing the stack.
    pub(crate) evaluate_depth: u32,
}

/// Named constants usable as bare words in formulas.
fn constant(name: &str) -> Option<Value> {
    let n = name.to_ascii_lowercase();
    let v = match n.as_str() {
        "true" => 1,
        "false" => 0,
        "jsonraw" => 0,
        "jsonstring" => 1,
        "jsonnumber" => 2,
        "jsonobject" => 3,
        "jsonarray" => 4,
        "jsonboolean" => 5,
        "jsonnull" => 6,
        _ => return None,
    };
    Some(Value::int(v))
}

/// Integer argument (counts, positions, precisions). Limited to ±2^53 so
/// callers can add and subtract small offsets without overflow.
pub fn to_index(v: &Value) -> Result<i64, EvalError> {
    const LIMIT: u64 = 1 << 53;
    num::trunc(&v.num_or_zero(), 0)
        .to_i64()
        .filter(|i| i.unsigned_abs() <= LIMIT)
        .ok_or_else(|| EvalError::new(code::CANNOT_EVALUATE_NUMBER, "number out of range"))
}

impl<'a> Evaluator<'a> {
    pub fn new(ctx: &'a mut dyn Context) -> Self {
        Evaluator {
            ctx,
            scopes: Vec::new(),
            scale: num::DEFAULT_SCALE,
            max_iterations: 50_000,
            evaluate_depth: 0,
        }
    }

    pub fn eval(&mut self, e: &Expr) -> R {
        match e {
            Expr::Number(n) => Ok(Value::Number(n.clone())),
            Expr::Text(s) => Ok(Value::Text(s.clone())),
            Expr::List(_) => Err(EvalError::new(
                code::LIST_NOT_ALLOWED,
                "list usage is not allowed here",
            )),
            Expr::Field {
                table: None,
                name,
                rep: None,
            } => match self.local(name) {
                Some(v) => Ok(v),
                None => self.ctx.field(None, name, 1),
            },
            Expr::Field { table, name, rep } => {
                let rep = self.repetition(rep)?;
                self.ctx.field(table.as_deref(), name, rep)
            }
            Expr::Var { name, rep } => {
                let rep = self.repetition(rep)?;
                Ok(self.ctx.variable(name, rep))
            }
            Expr::Call { name, args } => functions::call(self, name, args),
            Expr::Unary(op, e) => {
                let v = self.eval(e)?;
                Ok(match (op, v) {
                    (UnOp::Not, v) => Value::bool(!v.truth()),
                    (UnOp::Neg, Value::Time(t)) => Value::Time(-t),
                    (UnOp::Neg, v) => Value::num(-v.num_or_zero()),
                    (UnOp::Plus, v @ (Value::Date(_) | Value::Time(_) | Value::Timestamp(_))) => v,
                    (UnOp::Plus, v) => Value::num(v.num_or_zero()),
                })
            }
            Expr::Binary(op, l, r) => match op {
                BinOp::And => Ok(Value::bool(self.eval(l)?.truth() && self.eval(r)?.truth())),
                BinOp::Or => Ok(Value::bool(self.eval(l)?.truth() || self.eval(r)?.truth())),
                BinOp::Xor => Ok(Value::bool(self.eval(l)?.truth() != self.eval(r)?.truth())),
                _ => {
                    let (l, r) = (self.eval(l)?, self.eval(r)?);
                    binary(*op, l, r, self.scale)
                }
            },
        }
    }

    /// A bare name that is a Let variable or a named constant.
    pub fn local(&self, name: &str) -> Option<Value> {
        let key = name.to_lowercase();
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(&key))
            .cloned()
            .or_else(|| constant(name))
    }

    fn repetition(&mut self, rep: &Option<Box<Expr>>) -> Result<usize, EvalError> {
        match rep {
            None => Ok(1),
            Some(e) => {
                let v = self.eval(e)?;
                Ok(to_index(&v)?.max(0) as usize)
            }
        }
    }

    /// Runs `f` inside a fresh Let/While scope.
    pub fn scoped<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, EvalError>,
    ) -> Result<T, EvalError> {
        self.scopes.push(HashMap::new());
        let r = f(self);
        self.scopes.pop();
        r
    }

    /// Runs `f` with no Let variables visible (Evaluate).
    pub fn isolated<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::take(&mut self.scopes);
        let r = f(self);
        self.scopes = saved;
        r
    }

    /// Evaluates `name = expr` bindings (a single one or a `[...]` list)
    /// into the innermost scope, in order.
    pub fn bind(&mut self, bindings: &Expr) -> Result<(), EvalError> {
        let list = match bindings {
            Expr::List(items) => items.as_slice(),
            one => std::slice::from_ref(one),
        };
        for b in list {
            let Expr::Binary(BinOp::Eq, target, value) = b else {
                return Err(EvalError::new(
                    code::OPERATOR_MISSING,
                    "\"=\" expected in variable definition",
                ));
            };
            let v = self.eval(value)?;
            match &**target {
                Expr::Field {
                    table: None,
                    name,
                    rep: None,
                } => {
                    self.scopes
                        .last_mut()
                        .expect("bind outside a scope")
                        .insert(name.to_lowercase(), v);
                }
                Expr::Var { name, rep } => {
                    let rep = self.repetition(rep)?;
                    self.ctx.set_variable(name, rep, v);
                }
                _ => return Err(EvalError::new(code::NAME_MISSING, "variable name expected")),
            }
        }
        Ok(())
    }
}

fn temporal(v: &Value) -> bool {
    matches!(v, Value::Date(_) | Value::Time(_) | Value::Timestamp(_))
}

fn date(d: Num) -> R {
    let d = num::trunc(&d, 0).to_i64().filter(|d| value::valid_day(*d));
    d.map(Value::Date)
        .ok_or_else(|| EvalError::generic("date out of range"))
}

/// Text compares case-insensitively; anything else compares as numbers.
pub fn compare(l: &Value, r: &Value) -> Ordering {
    match (l, r) {
        (Value::Text(a), Value::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        _ => l.num_or_zero().cmp(&r.num_or_zero()),
    }
}

pub fn binary(op: BinOp, l: Value, r: Value, scale: i64) -> R {
    use BinOp::*;
    use Value::*;
    let (a, b) = (l.num_or_zero(), r.num_or_zero());
    Ok(match op {
        Concat => Text(l.to_text() + &r.to_text()),
        Eq => Value::bool(compare(&l, &r).is_eq()),
        Ne => Value::bool(compare(&l, &r).is_ne()),
        Lt => Value::bool(compare(&l, &r).is_lt()),
        Le => Value::bool(compare(&l, &r).is_le()),
        Gt => Value::bool(compare(&l, &r).is_gt()),
        Ge => Value::bool(compare(&l, &r).is_ge()),
        Add => match (&l, &r) {
            (Date(_), x) | (x, Date(_)) if !temporal(x) => date(a + b)?,
            (Time(_), x) | (x, Time(_)) if !temporal(x) => Time(num::fix(a + b)),
            (Timestamp(_), x) | (x, Timestamp(_)) if !matches!(x, Date(_) | Timestamp(_)) => {
                Timestamp(num::fix(a + b))
            }
            _ => Value::num(a + b),
        },
        Sub => match (&l, &r) {
            (Date(_), x) if !temporal(x) => date(a - b)?,
            (Time(_), x) if !matches!(x, Date(_) | Timestamp(_)) => Time(num::fix(a - b)),
            (Timestamp(_), x) if !matches!(x, Date(_) | Timestamp(_)) => Timestamp(num::fix(a - b)),
            _ => Value::num(a - b),
        },
        Mul => Value::num(a * b),
        Div => {
            Number(num::div(&a, &b, scale).ok_or_else(|| EvalError::generic("division by zero"))?)
        }
        Pow => Number(num::pow(&a, &b, scale).ok_or_else(|| EvalError::generic("invalid power"))?),
        And | Or | Xor => unreachable!("logical operators short-circuit in eval"),
    })
}
