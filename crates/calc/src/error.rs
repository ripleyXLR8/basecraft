//! Parse and evaluation errors, carrying FileMaker error codes.

use std::fmt;

/// FileMaker error codes used by the calculation engine.
pub mod code {
    /// Command is unavailable (wrong platform or mode).
    pub const UNAVAILABLE: u32 = 3;
    /// Unknown option, e.g. a digest algorithm FileMaker does not support.
    pub const BAD_OPTION: u32 = 4;
    /// Square root of a negative, logarithm of 0, Factorial(-1)...
    pub const NUMBER_DOMAIN: u32 = 14;
    pub const DIVIDE_BY_ZERO: u32 = 15;
    /// Base64Decode / HexDecode of malformed input.
    pub const BAD_ENCODING: u32 = 17;
    pub const DATE_INVALID: u32 = 500;
    pub const TIME_INVALID: u32 = 501;
    pub const FILE_MISSING: u32 = 100;
    pub const FIELD_MISSING: u32 = 102;
    pub const LAYOUT_MISSING: u32 = 105;
    pub const TABLE_MISSING: u32 = 106;
    pub const SQL_ERROR: u32 = 8310;
    pub const GENERIC: u32 = 1200;
    pub const TOO_FEW_PARAMS: u32 = 1201;
    pub const TOO_MANY_PARAMS: u32 = 1202;
    pub const UNEXPECTED_END: u32 = 1203;
    pub const OPERAND_EXPECTED: u32 = 1204;
    pub const UNTERMINATED_COMMENT: u32 = 1205;
    pub const UNTERMINATED_TEXT: u32 = 1206;
    pub const UNBALANCED_PAREN: u32 = 1207;
    pub const OPERATOR_MISSING: u32 = 1208;
    pub const NAME_MISSING: u32 = 1209;
    pub const LIST_NOT_ALLOWED: u32 = 1211;
    /// An operator is expected here (tokens left after a complete formula).
    pub const OPERATOR_EXPECTED: u32 = 1212;
    /// An aggregate (Sum, Count, NPV...) given one value that is not a field.
    pub const FIELD_ALONE_NEEDED: u32 = 1214;
    pub const BAD_GET_PARAM: u32 = 1215;
    pub const CANNOT_EVALUATE_NUMBER: u32 = 1218;
    pub const NOT_IMPLEMENTED: u32 = 1223;
    pub const NO_SUCH_FUNCTION: u32 = 1224;
    pub const NOT_SUPPORTED: u32 = 1225;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Character offset in the source.
    pub pos: usize,
    pub code: u32,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "error {} at {}: {}", self.code, self.pos, self.message)
    }
}

impl std::error::Error for ParseError {}

/// A calculation error. FileMaker displays these as `?`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalError {
    pub code: u32,
    pub message: String,
}

impl EvalError {
    pub fn new(code: u32, message: impl Into<String>) -> EvalError {
        EvalError {
            code,
            message: message.into(),
        }
    }

    pub fn generic(message: impl Into<String>) -> EvalError {
        EvalError::new(code::GENERIC, message)
    }
}

impl From<ParseError> for EvalError {
    fn from(e: ParseError) -> EvalError {
        EvalError {
            code: e.code,
            message: e.to_string(),
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for EvalError {}
