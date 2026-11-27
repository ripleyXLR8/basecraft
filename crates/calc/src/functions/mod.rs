//! Function registry and the logic/aggregate functions that need
//! unevaluated arguments.

mod crypt;
mod datetime;
mod design;
mod encode;
mod host;
mod jp;
mod json;
mod number;
mod sql;
mod text;

pub use datetime::builtin_get;
pub(crate) use text::base_key;

use crate::ast::Expr;
use crate::error::{EvalError, code};
use crate::eval::{Evaluator, R, to_index};
use crate::num::{self, Num};
use crate::value::Value;
use bigdecimal::Zero;

pub type EagerFn = fn(&mut Evaluator, &[Value]) -> R;
pub type LazyFn = fn(&mut Evaluator, &[Expr]) -> R;

pub enum Imp {
    /// Arguments are evaluated first; any error propagates.
    Eager(EagerFn),
    /// Receives the argument expressions (short-circuit, field references).
    Lazy(LazyFn),
}

pub struct Func {
    pub name: &'static str,
    pub min: usize,
    pub max: usize,
    pub imp: Imp,
}

const N: usize = usize::MAX;

const fn e(name: &'static str, min: usize, max: usize, f: EagerFn) -> Func {
    Func {
        name,
        min,
        max,
        imp: Imp::Eager(f),
    }
}

const fn l(name: &'static str, min: usize, max: usize, f: LazyFn) -> Func {
    Func {
        name,
        min,
        max,
        imp: Imp::Lazy(f),
    }
}

/// Every implemented function. Names listed in the reference but missing
/// here parse fine and evaluate to error 1223 (not implemented).
pub static FUNCTIONS: &[Func] = &[
    // logic
    l("If", 2, 3, if_),
    l("Case", 2, N, case),
    l("Choose", 2, N, choose),
    l("Let", 2, 2, let_),
    l("While", 4, 4, while_),
    l("Evaluate", 1, 2, evaluate),
    l("EvaluationError", 1, 1, evaluation_error),
    l("IsEmpty", 1, 1, is_empty),
    l("IsValid", 1, 1, is_valid),
    e("IsValidExpression", 1, 1, is_valid_expression),
    l("SetPrecision", 2, 2, set_precision),
    l("SetRecursion", 2, 2, set_recursion),
    l("Get", 1, 1, get),
    // fields and repetitions
    e("GetField", 1, 1, get_field),
    l("GetFieldName", 1, 1, get_field_name),
    l("GetRepetition", 2, 2, get_repetition),
    l("Extend", 1, 1, extend),
    l("Last", 1, 1, last),
    // aggregates
    l("Sum", 1, N, sum),
    l("Average", 1, N, average),
    l("Count", 1, N, count),
    l("Min", 1, N, min),
    l("Max", 1, N, max),
    l("StDev", 1, N, stdev),
    l("StDevP", 1, N, stdevp),
    l("Variance", 1, N, variance),
    l("VarianceP", 1, N, variancep),
    l("List", 1, N, list),
    l("NPV", 2, 2, npv),
    // number
    e("Abs", 1, 1, number::abs),
    e("Acos", 1, 1, number::acos),
    e("Asin", 1, 1, number::asin),
    e("Atan", 1, 1, number::atan),
    e("Ceiling", 1, 1, number::ceiling),
    e("Combination", 2, 2, number::combination),
    e("Cos", 1, 1, number::cos),
    e("Degrees", 1, 1, number::degrees),
    e("Div", 2, 2, number::div),
    e("Exp", 1, 1, number::exp),
    e("Factorial", 1, 2, number::factorial),
    e("Floor", 1, 1, number::floor),
    e("FV", 3, 3, number::fv),
    e("Int", 1, 1, number::int_),
    e("Lg", 1, 1, number::lg),
    e("Ln", 1, 1, number::ln),
    e("Log", 1, 1, number::log),
    e("Mod", 2, 2, number::modulo),
    e("Pi", 0, 0, number::pi),
    e("PMT", 3, 3, number::pmt),
    e("PV", 3, 3, number::pv),
    e("Radians", 1, 1, number::radians),
    e("Random", 0, 0, number::random),
    e("RGB", 3, 3, number::rgb),
    e("Round", 2, 2, number::round),
    e("Sign", 1, 1, number::sign),
    e("Sin", 1, 1, number::sin),
    e("Sqrt", 1, 1, number::sqrt),
    e("Tan", 1, 1, number::tan),
    e("Truncate", 2, 2, number::truncate),
    // text
    e("Char", 1, 1, encode::char),
    e("Code", 1, 1, encode::code),
    e("Exact", 2, 2, text::exact),
    e("Filter", 2, 2, text::filter),
    e("FilterValues", 2, 2, text::filter_values),
    e("GetValue", 2, 2, text::get_value),
    e("Hiragana", 1, 1, jp::hiragana),
    e("Katakana", 1, 1, jp::katakana),
    e("Left", 2, 2, text::left),
    e("LeftValues", 2, 2, text::left_values),
    e("LeftWords", 2, 2, text::left_words),
    e("Length", 1, 1, text::length),
    e("Lower", 1, 1, text::lower),
    e("Middle", 3, 3, text::middle),
    e("MiddleValues", 3, 3, text::middle_values),
    e("MiddleWords", 3, 3, text::middle_words),
    e("PatternCount", 2, 2, text::pattern_count),
    e("Position", 4, 4, text::position),
    e("Proper", 1, 1, text::proper),
    e("Quote", 1, 1, text::quote),
    e("Replace", 4, 4, text::replace),
    e("Right", 2, 2, text::right),
    e("RightValues", 2, 2, text::right_values),
    e("RightWords", 2, 2, text::right_words),
    e("RomanHankaku", 1, 1, text::roman_hankaku),
    e("RomanZenkaku", 1, 1, text::roman_zenkaku),
    e("SerialIncrement", 2, 2, text::serial_increment),
    e("SortValues", 1, 3, text::sort_values),
    l("Substitute", 2, N, text::substitute),
    e("Trim", 1, 1, text::trim),
    e("TrimAll", 3, 3, text::trim_all),
    e("UniqueValues", 1, 3, text::unique_values),
    e("Upper", 1, 1, text::upper),
    e("ValueCount", 1, 1, text::value_count),
    e("WordCount", 1, 1, text::word_count),
    // styled text: no style storage yet, the plain text passes through
    e("TextColor", 2, 2, text::plain),
    e("TextColorRemove", 1, 2, text::plain),
    e("TextFont", 2, 3, text::plain),
    e("TextFontRemove", 1, 3, text::plain),
    e("TextFormatRemove", 1, 1, text::plain),
    e("TextSize", 2, 2, text::plain),
    e("TextSizeRemove", 1, 2, text::plain),
    l("TextStyleAdd", 2, 2, text::plain_lazy),
    l("TextStyleRemove", 2, 2, text::plain_lazy),
    // date and time
    e("Date", 3, 3, datetime::date),
    e("Day", 1, 1, datetime::day),
    e("DayName", 1, 1, datetime::day_name),
    e("DayNameJ", 1, 1, datetime::day_name_j),
    e("DayOfWeek", 1, 1, datetime::day_of_week),
    e("DayOfYear", 1, 1, datetime::day_of_year),
    e("Hour", 1, 1, datetime::hour),
    e("Minute", 1, 1, datetime::minute),
    e("Month", 1, 1, datetime::month),
    e("MonthName", 1, 1, datetime::month_name),
    e("Seconds", 1, 1, datetime::seconds),
    e("Time", 3, 3, datetime::time),
    e("Timestamp", 2, 2, datetime::timestamp),
    e("WeekOfYear", 1, 1, datetime::week_of_year),
    e("WeekOfYearFiscal", 2, 2, datetime::week_of_year_fiscal),
    e("Year", 1, 1, datetime::year),
    // conversion
    e("GetAsBoolean", 1, 1, get_as_boolean),
    e("GetAsDate", 1, 1, datetime::get_as_date),
    e("GetAsNumber", 1, 1, get_as_number),
    e("GetAsText", 1, 1, get_as_text),
    e("GetAsTime", 1, 1, datetime::get_as_time),
    e("GetAsTimestamp", 1, 1, datetime::get_as_timestamp),
    e("GetAsURLEncoded", 1, 1, encode::url_encode),
    // encoding and crypto
    e("Base64Decode", 1, 2, encode::base64_decode),
    e("Base64Encode", 1, 1, encode::base64_encode),
    e("Base64EncodeRFC", 2, 2, encode::base64_encode_rfc),
    e("HexDecode", 1, 2, encode::hex_decode),
    e("HexEncode", 1, 1, encode::hex_encode),
    e("CryptDigest", 2, 2, encode::crypt_digest),
    e("CryptAuthCode", 3, 3, encode::crypt_auth_code),
    e("TextEncode", 3, 3, encode::text_encode),
    e("TextDecode", 2, 2, encode::text_decode),
    // JSON
    l("JSONSetElement", 2, N, json::set_element),
    e("JSONGetElement", 2, 2, json::get_element),
    e("JSONGetElementType", 2, 2, json::get_element_type),
    e("JSONDeleteElement", 2, 2, json::delete_element),
    e("JSONListKeys", 2, 2, json::list_keys),
    e("JSONListValues", 2, 2, json::list_values),
    e("JSONFormatElements", 1, 1, json::format_elements),
    e("JSONMakeArray", 3, 3, json::make_array),
    e("JSONParse", 1, 1, json::json_parse),
    e("JSONParsedState", 1, 1, json::json_parsed_state),
    e("JSONValueExists", 3, 3, json::value_exists),
    e("JSONValueCount", 3, 3, json::value_count),
    e("JSONAnyValuesExist", 3, 3, json::any_values_exist),
    e("JSONRemoveValues", 3, 3, json::remove_values),
    e("JSONUniqueValues", 2, 2, json::unique_values),
    // styled text as HTML/SVG
    e("GetAsCSS", 1, 1, text::get_as_css),
    e("GetAsSVG", 1, 1, text::get_as_svg),
    // Japanese
    e("KanaHankaku", 1, 1, jp::kana_hankaku),
    e("KanaZenkaku", 1, 1, jp::kana_zenkaku),
    e("KanjiNumeral", 1, 1, jp::kanji_numeral),
    e("NumToJText", 3, 3, jp::num_to_j_text),
    e("YearName", 2, 2, jp::year_name),
    e("MonthNameJ", 1, 1, jp::month_name_j),
    // encryption and signatures
    e("CryptEncrypt", 2, 2, crypt::crypt_encrypt),
    e("CryptDecrypt", 2, 2, crypt::crypt_decrypt),
    e("CryptEncryptBase64", 2, 2, crypt::crypt_encrypt_base64),
    e("CryptDecryptBase64", 2, 2, crypt::crypt_decrypt_base64),
    e(
        "CryptGenerateSignature",
        4,
        4,
        crypt::crypt_generate_signature,
    ),
    e("CryptVerifySignature", 4, 4, crypt::crypt_verify_signature),
    // SQL
    e("ExecuteSQL", 3, N, sql::execute_sql),
    e("ExecuteSQLe", 3, N, sql::execute_sql_e),
    // design (schema through Context::schema)
    e("BaseTableComment", 2, 2, design::base_table_comment),
    e("BaseTableIDs", 1, 1, design::base_table_ids),
    e("BaseTableNames", 1, 1, design::base_table_names),
    e("FieldAnnotation", 2, 2, design::field_annotation),
    e("FieldBounds", 3, 3, design::field_bounds),
    e("FieldComment", 2, 2, design::field_comment),
    e("FieldDisplayNames", 2, 2, design::field_display_names),
    e("FieldIDs", 2, 2, design::field_ids),
    e("FieldNames", 2, 2, design::field_names),
    e("FieldRepetitions", 3, 3, design::field_repetitions),
    e("FieldStyle", 3, 3, design::field_style),
    e("FieldType", 2, 2, design::field_type),
    l("GetBaseTableName", 1, 1, design::get_base_table_name),
    e("GetFieldsOnLayout", 1, 1, design::get_fields_on_layout),
    e("GetNextSerialValue", 2, 2, design::get_next_serial_value),
    e("GetTableDDL", 2, 2, design::get_table_ddl),
    e("LayoutIDs", 1, 1, design::layout_ids),
    e("LayoutNames", 1, 1, design::layout_names),
    e("LayoutObjectNames", 2, 2, design::layout_object_names),
    e("RelationInfo", 2, 2, design::relation_info),
    e("ScriptIDs", 1, 1, design::script_ids),
    e("ScriptNames", 1, 1, design::script_names),
    e("TableIDs", 1, 1, design::table_ids),
    e("TableNames", 1, 1, design::table_names),
    e("ValueListIDs", 1, 1, design::value_list_ids),
    e("ValueListItems", 2, 2, design::value_list_items),
    e("ValueListNames", 1, 1, design::value_list_names),
    // records (through Context)
    l("GetNthRecord", 2, 2, design::get_nth_record),
    l("GetSummary", 2, 2, design::get_summary),
    l("Lookup", 1, 2, design::lookup),
    l("LookupNext", 2, 2, design::lookup_next),
    e(
        "GetRecordIDsFromFoundSet",
        1,
        2,
        design::get_record_ids_from_found_set,
    ),
    e("Self", 0, 0, design::self_),
    l("Status", 1, 1, design::status),
    // containers, embeddings and paths
    e("GetContainerAttribute", 2, 2, host::get_container_attribute),
    e("GetHeight", 1, 1, host::get_height),
    e("GetWidth", 1, 1, host::get_width),
    e("VerifyContainer", 1, 1, host::verify_container),
    e("CosineSimilarity", 2, 2, host::cosine_similarity),
    e("AddEmbeddings", 2, 2, host::add_embeddings),
    e("SubtractEmbeddings", 2, 2, host::subtract_embeddings),
    e("NormalizeEmbedding", 1, 2, host::normalize_embedding),
    e("GetEmbeddingAsText", 1, 1, host::get_embedding_as_text),
    e("GetEmbeddingAsFile", 1, 2, host::get_embedding_as_file),
    e(
        "ConvertFromFileMakerPath",
        2,
        2,
        host::convert_from_filemaker_path,
    ),
    e(
        "ConvertToFileMakerPath",
        2,
        2,
        host::convert_to_filemaker_path,
    ),
    // platform and services (Context::host_function; "not available" by default)
    e("Location", 1, 2, host::location),
    e("LocationValues", 1, 2, host::location_values),
    e("GetSensor", 1, 3, host::get_sensor),
    e("RangeBeacons", 1, 4, host::range_beacons),
    e("Furigana", 1, 2, host::furigana),
    e("GetLiveText", 2, 2, host::get_live_text),
    e("GetLiveTextAsJSON", 2, 2, host::get_live_text_as_json),
    e("ReadQRCode", 1, 1, host::read_qr_code),
    e("GetTextFromPDF", 1, 1, host::get_text_from_pdf),
    e("GetThumbnail", 3, 3, host::get_thumbnail),
    e("GetAVPlayerAttribute", 1, 1, host::get_av_player_attribute),
    e("GetAddonInfo", 1, 1, host::get_addon_info),
    e("GetPersistentData", 2, 2, host::get_persistent_data),
    e(
        "ListPersistentDataIDs",
        1,
        1,
        host::list_persistent_data_ids,
    ),
    e(
        "GetLayoutObjectAttribute",
        2,
        4,
        host::get_layout_object_attribute,
    ),
    e(
        "GetLayoutObjectOwnerInfo",
        1,
        1,
        host::get_layout_object_owner_info,
    ),
    e("GetEmbedding", 3, 3, host::get_embedding),
    e("GetTokenCount", 1, 1, host::get_token_count),
    e("ComputeModel", 1, N, host::compute_model),
    e("PredictFromModel", 2, 2, host::predict_from_model),
    e("GetModelAttributes", 1, 1, host::get_model_attributes),
    e("GetRAGSpaceInfo", 1, 2, host::get_rag_space_info),
];

/// The function signatures extracted from FileMaker Pro 26.
pub const REFERENCE: &str = include_str!("../../../../docs/reference/functions.txt");
/// The `Get()` flags extracted from FileMaker Pro 26.
pub const GET_REFERENCE: &str = include_str!("../../../../docs/reference/get_functions.txt");

/// Name part of a reference line: `Left ( text ; n )` -> `Left`.
pub fn reference_name(line: &str) -> &str {
    line.split(|c: char| c == '(' || c.is_whitespace())
        .next()
        .unwrap_or("")
}

pub fn lookup(name: &str) -> Option<&'static Func> {
    // ponytail: linear scan over ~150 entries; a HashMap if profiling says so.
    FUNCTIONS.iter().find(|f| f.name.eq_ignore_ascii_case(name))
}

/// Whether `name` is a FileMaker function (implemented or not).
pub fn exists(name: &str) -> bool {
    lookup(name).is_some()
        || REFERENCE
            .lines()
            .any(|l| reference_name(l).eq_ignore_ascii_case(name))
}

pub fn call(ev: &mut Evaluator, name: &str, args: &[Expr]) -> R {
    let Some(f) = lookup(name) else {
        let known = REFERENCE
            .lines()
            .any(|l| reference_name(l).eq_ignore_ascii_case(name));
        return Err(if known {
            EvalError::new(
                code::NOT_IMPLEMENTED,
                format!("function {name} is not implemented yet"),
            )
        } else {
            EvalError::new(
                code::NO_SUCH_FUNCTION,
                format!("function {name} does not exist"),
            )
        });
    };
    if args.is_empty() && f.min > 0 {
        return Err(EvalError::new(
            code::OPERAND_EXPECTED,
            format!("{} ( ) needs a parameter", f.name),
        ));
    }
    if args.len() < f.min {
        return Err(EvalError::new(
            code::TOO_FEW_PARAMS,
            format!("too few parameters in {}", f.name),
        ));
    }
    if args.len() > f.max {
        return Err(EvalError::new(
            code::TOO_MANY_PARAMS,
            format!("too many parameters in {}", f.name),
        ));
    }
    let r = match f.imp {
        Imp::Lazy(g) => g(ev, args),
        Imp::Eager(g) => {
            let vals = args
                .iter()
                .map(|a| ev.eval(a))
                .collect::<Result<Vec<_>, _>>()?;
            g(ev, &vals)
        }
    };
    // Design functions answer a missing file, table, layout, field or
    // value list with empty text, not an error.
    match r {
        Err(e)
            if EMPTY_WHEN_MISSING.contains(&f.name)
                && matches!(
                    e.code,
                    code::FILE_MISSING
                        | code::FIELD_MISSING
                        | code::LAYOUT_MISSING
                        | code::TABLE_MISSING
                ) =>
        {
            Ok(Value::empty())
        }
        r => r,
    }
}

const EMPTY_WHEN_MISSING: [&str; 25] = [
    "BaseTableComment",
    "BaseTableIDs",
    "BaseTableNames",
    "FieldAnnotation",
    "FieldBounds",
    "FieldComment",
    "FieldDisplayNames",
    "FieldIDs",
    "FieldNames",
    "FieldRepetitions",
    "FieldStyle",
    "FieldType",
    "GetNextSerialValue",
    "LayoutIDs",
    "LayoutNames",
    "LayoutObjectNames",
    "RelationInfo",
    "ScriptIDs",
    "ScriptNames",
    "TableIDs",
    "TableNames",
    "ValueListIDs",
    "ValueListItems",
    "ValueListNames",
    "GetFieldsOnLayout",
];

// ---- small argument helpers shared by the function modules -------------

pub(crate) fn txt(a: &[Value], i: usize) -> String {
    a.get(i).map(Value::to_text).unwrap_or_default()
}

pub(crate) fn number(a: &[Value], i: usize) -> Num {
    a.get(i).map(Value::num_or_zero).unwrap_or_default()
}

pub(crate) fn int(a: &[Value], i: usize) -> Result<i64, EvalError> {
    a.get(i).map(to_index).unwrap_or(Ok(0))
}

// ---- logic ----------------------------------------------------------------

fn if_(ev: &mut Evaluator, a: &[Expr]) -> R {
    if ev.eval(&a[0])?.truth() {
        ev.eval(&a[1])
    } else {
        a.get(2).map_or(Ok(Value::empty()), |e| ev.eval(e))
    }
}

fn case(ev: &mut Evaluator, a: &[Expr]) -> R {
    for pair in a.chunks(2) {
        match pair {
            [test, result] => {
                if ev.eval(test)?.truth() {
                    return ev.eval(result);
                }
            }
            [default] => return ev.eval(default),
            _ => unreachable!(),
        }
    }
    Ok(Value::empty())
}

fn choose(ev: &mut Evaluator, a: &[Expr]) -> R {
    let i = to_index(&ev.eval(&a[0])?)?;
    match usize::try_from(i).ok().and_then(|i| a.get(i + 1)) {
        Some(e) => ev.eval(e),
        None => Ok(Value::empty()),
    }
}

fn let_(ev: &mut Evaluator, a: &[Expr]) -> R {
    ev.scoped(|ev| {
        ev.bind(&a[0])?;
        ev.eval(&a[1])
    })
}

fn while_(ev: &mut Evaluator, a: &[Expr]) -> R {
    ev.scoped(|ev| {
        ev.bind(&a[0])?;
        let mut n = 0u64;
        while ev.eval(&a[1])?.truth() {
            n += 1;
            if n > ev.max_iterations {
                return Err(EvalError::generic(WHILE_LIMIT));
            }
            ev.bind(&a[2])?;
        }
        ev.eval(&a[3])
    })
}

fn evaluate(ev: &mut Evaluator, a: &[Expr]) -> R {
    let src = ev.eval(&a[0])?.to_text();
    if let Some(triggers) = a.get(1) {
        // Trigger fields only matter for stored calculations; evaluate for
        // errors (a [list] is one: 1204).
        ev.eval(triggers)?;
    }
    if src.trim().is_empty() {
        return Ok(Value::empty());
    }
    let expr = crate::parser::parse(&src)?;
    if ev.evaluate_depth >= 32 {
        return Err(EvalError::generic("Evaluate is nested too deeply"));
    }
    ev.evaluate_depth += 1;
    let r = ev.isolated(|ev| ev.eval(&expr));
    ev.evaluate_depth -= 1;
    r
}

/// The error code of its argument; a While or recursion limit is not
/// caught (FileMaker returns "?").
fn evaluation_error(ev: &mut Evaluator, a: &[Expr]) -> R {
    match ev.eval(&a[0]) {
        Ok(_) => Ok(Value::int(0)),
        Err(e) if e.message == WHILE_LIMIT => Err(e),
        Err(e) => Ok(Value::int(e.code as i64)),
    }
}

const WHILE_LIMIT: &str = "While exceeded its iteration limit";

fn is_empty(ev: &mut Evaluator, a: &[Expr]) -> R {
    Ok(Value::bool(ev.eval(&a[0]).map_or(true, |v| v.is_empty())))
}

fn is_valid(ev: &mut Evaluator, a: &[Expr]) -> R {
    Ok(Value::bool(ev.eval(&a[0]).is_ok()))
}

/// Valid when it parses and every field it names exists (Let and While
/// variables aside).
fn is_valid_expression(ev: &mut Evaluator, a: &[Value]) -> R {
    let Ok(e) = crate::parser::parse(&txt(a, 0)) else {
        return Ok(Value::bool(false));
    };
    Ok(Value::bool(fields_exist(ev, &e, &mut vec![])))
}

fn fields_exist(ev: &mut Evaluator, e: &Expr, locals: &mut Vec<String>) -> bool {
    let missing = |r: R| matches!(r, Err(e) if e.code == code::FIELD_MISSING);
    match e {
        Expr::Field {
            table: None, name, ..
        } if locals.iter().any(|l| l.eq_ignore_ascii_case(name)) || ev.local(name).is_some() => {
            true
        }
        Expr::Field { table, name, .. } => !missing(ev.ctx.field(table.as_deref(), name, 1)),
        Expr::Call { name, args } => {
            let binds = ["Let", "While"]
                .iter()
                .any(|f| f.eq_ignore_ascii_case(name));
            if binds && let Some(b) = args.first() {
                let list = match b {
                    Expr::List(items) => items.as_slice(),
                    one => std::slice::from_ref(one),
                };
                for item in list {
                    if let Expr::Binary(_, target, value) = item {
                        if !fields_exist(ev, value, locals) {
                            return false;
                        }
                        if let Expr::Field { name, .. } = &**target {
                            locals.push(name.clone());
                        }
                    }
                }
                return args[1..].iter().all(|a| fields_exist(ev, a, locals));
            }
            args.iter().all(|a| fields_exist(ev, a, locals))
        }
        Expr::Unary(_, x) => fields_exist(ev, x, locals),
        Expr::Binary(_, l, r) => fields_exist(ev, l, locals) && fields_exist(ev, r, locals),
        Expr::List(items) => items.iter().all(|a| fields_exist(ev, a, locals)),
        _ => true,
    }
}

fn set_precision(ev: &mut Evaluator, a: &[Expr]) -> R {
    let p = to_index(&ev.eval(&a[1])?)?.clamp(num::DEFAULT_SCALE, 400);
    let saved = std::mem::replace(&mut ev.scale, p);
    let r = ev.eval(&a[0]);
    ev.scale = saved;
    // The documented SetPrecision ( 1.3213213213213213213... ; 0 ) returns
    // 16 decimals: the result itself is rounded to the precision.
    match r? {
        Value::Number(n) => Ok(Value::Number(num::round(&n, p))),
        v => Ok(v),
    }
}

fn set_recursion(ev: &mut Evaluator, a: &[Expr]) -> R {
    let n = to_index(&ev.eval(&a[1])?)?.max(0) as u64;
    let saved = std::mem::replace(&mut ev.max_iterations, n);
    let r = ev.eval(&a[0]);
    ev.max_iterations = saved;
    r
}

fn get(ev: &mut Evaluator, a: &[Expr]) -> R {
    let Expr::Field {
        table: None,
        name: flag,
        rep: None,
    } = &a[0]
    else {
        return Err(EvalError::new(code::BAD_GET_PARAM, "invalid Get parameter"));
    };
    let known = GET_REFERENCE.lines().map(|l| {
        l.trim()
            .trim_start_matches("Get")
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim()
    });
    let Some(canonical) = known
        .into_iter()
        .find(|k| k.eq_ignore_ascii_case(flag) && *k != "flag")
    else {
        return Err(EvalError::new(
            code::BAD_GET_PARAM,
            format!("{flag} is not a Get parameter"),
        ));
    };
    if let Some(v) = ev.ctx.get(canonical) {
        return Ok(v);
    }
    datetime::builtin_get(canonical).ok_or_else(|| {
        EvalError::new(
            code::NOT_SUPPORTED,
            format!("Get ( {canonical} ) needs a host"),
        )
    })
}

// ---- fields -----------------------------------------------------------------

fn get_field(ev: &mut Evaluator, a: &[Value]) -> R {
    let name = txt(a, 0);
    match name.split_once("::") {
        Some((t, f)) => ev.ctx.field(Some(t), f, 1),
        None => ev.ctx.field(None, &name, 1),
    }
}

fn get_field_name(_: &mut Evaluator, a: &[Expr]) -> R {
    match &a[0] {
        Expr::Field { table, name, .. } => Ok(Value::text(match table {
            Some(t) => format!("{t}::{name}"),
            None => name.clone(),
        })),
        _ => Err(EvalError::new(
            code::NAME_MISSING,
            "GetFieldName needs a field",
        )),
    }
}

fn get_repetition(ev: &mut Evaluator, a: &[Expr]) -> R {
    let n = to_index(&ev.eval(&a[1])?)?.max(0) as usize;
    match &a[0] {
        Expr::Field { table, name, .. } => ev.ctx.field(table.as_deref(), name, n),
        Expr::Var { name, .. } => Ok(ev.ctx.variable(name, n)),
        e if n == 1 => ev.eval(e),
        _ => Ok(Value::empty()),
    }
}

fn extend(ev: &mut Evaluator, a: &[Expr]) -> R {
    // ponytail: no repeating calculation context yet, so Extend is identity.
    ev.eval(&a[0])
}

fn last(ev: &mut Evaluator, a: &[Expr]) -> R {
    let vals = expand(ev, &a[0])?;
    Ok(vals
        .into_iter()
        .rev()
        .find(|v| !v.is_empty())
        .unwrap_or_else(Value::empty))
}

// ---- aggregates ---------------------------------------------------------------

/// A bare field reference stands for all its values (related records or
/// repetitions); any other expression for its single value.
fn expand(ev: &mut Evaluator, e: &Expr) -> Result<Vec<Value>, EvalError> {
    match e {
        Expr::Field {
            table,
            name,
            rep: None,
        } => {
            // A Let variable or constant shadows the field.
            if let Some(v) = table.is_none().then(|| ev.local(name)).flatten() {
                return Ok(vec![v]);
            }
            ev.ctx.field_values(table.as_deref(), name)
        }
        e => Ok(vec![ev.eval(e)?]),
    }
}

/// An aggregate given a single value that is not a field: error 1214.
fn field_alone(a: &[Expr]) -> Result<(), EvalError> {
    match a {
        [Expr::Field { .. }] => Ok(()),
        [_] => Err(EvalError::new(
            code::FIELD_ALONE_NEEDED,
            "a field alone is needed here",
        )),
        _ => Ok(()),
    }
}

fn values(ev: &mut Evaluator, a: &[Expr]) -> Result<Vec<Value>, EvalError> {
    field_alone(a)?;
    let mut out = Vec::new();
    for e in a {
        out.extend(expand(ev, e)?);
    }
    Ok(out)
}

fn numbers(ev: &mut Evaluator, a: &[Expr]) -> Result<Vec<Num>, EvalError> {
    Ok(values(ev, a)?.iter().filter_map(Value::to_num).collect())
}

fn sum(ev: &mut Evaluator, a: &[Expr]) -> R {
    Ok(Value::num(numbers(ev, a)?.iter().sum()))
}

fn average(ev: &mut Evaluator, a: &[Expr]) -> R {
    let n = numbers(ev, a)?;
    if n.is_empty() {
        return Ok(Value::empty());
    }
    let total: Num = n.iter().sum();
    Ok(Value::Number(
        num::div(&total, &num::int(n.len() as i64), ev.scale)
            .ok_or_else(|| EvalError::new(code::DIVIDE_BY_ZERO, "division by zero"))?,
    ))
}

fn count(ev: &mut Evaluator, a: &[Expr]) -> R {
    Ok(Value::int(
        values(ev, a)?.iter().filter(|v| !v.is_empty()).count() as i64,
    ))
}

/// Min / Max: text values compare as text (collation order) when every
/// value is text, otherwise as numbers (text without a number counts as
/// 0); on a tie the later value wins.
fn extreme(ev: &mut Evaluator, a: &[Expr], want: std::cmp::Ordering) -> R {
    let vals: Vec<Value> = values(ev, a)?
        .into_iter()
        .filter(|v| !v.is_empty())
        .collect();
    let all_text = vals
        .iter()
        .all(|v| matches!(v, Value::Text(_) | Value::Json { .. }));
    let mut best: Option<Value> = None;
    for v in vals {
        let better = best.as_ref().is_none_or(|b| {
            let o = if all_text {
                text::collate(&v.to_text(), &b.to_text())
            } else {
                num::shown(&v.num_or_zero()).cmp(&num::shown(&b.num_or_zero()))
            };
            o == want || o.is_eq()
        });
        if better {
            best = Some(v);
        }
    }
    Ok(best.unwrap_or_else(Value::empty))
}
fn min(ev: &mut Evaluator, a: &[Expr]) -> R {
    extreme(ev, a, std::cmp::Ordering::Less)
}

fn max(ev: &mut Evaluator, a: &[Expr]) -> R {
    extreme(ev, a, std::cmp::Ordering::Greater)
}

/// Sum of squared deviations / (n - sample).
fn var(ev: &mut Evaluator, a: &[Expr], sample: bool) -> Result<Option<Num>, EvalError> {
    let n = numbers(ev, a)?;
    let count = n.len() as i64 - sample as i64;
    if n.is_empty() {
        return Ok(None);
    }
    if count <= 0 {
        return Ok(Some(Num::zero()));
    }
    let w = ev.scale + 10;
    let mean = num::div(&n.iter().sum(), &num::int(n.len() as i64), w)
        .ok_or_else(|| EvalError::new(code::DIVIDE_BY_ZERO, "division by zero"))?;
    let ss: Num = n.iter().map(|x| (x - &mean) * (x - &mean)).sum();
    Ok(num::div(&ss, &num::int(count), w))
}

fn variance_like(ev: &mut Evaluator, a: &[Expr], sample: bool, root: bool) -> R {
    let scale = ev.scale;
    match var(ev, a, sample)? {
        None => Ok(Value::empty()),
        Some(v) if root => num::sqrt(&v, scale)
            .map(Value::Number)
            .ok_or_else(|| EvalError::new(code::NUMBER_DOMAIN, "negative variance")),
        Some(v) => Ok(Value::Number(num::round(&v, scale))),
    }
}

fn stdev(ev: &mut Evaluator, a: &[Expr]) -> R {
    variance_like(ev, a, true, true)
}

fn stdevp(ev: &mut Evaluator, a: &[Expr]) -> R {
    variance_like(ev, a, false, true)
}

fn variance(ev: &mut Evaluator, a: &[Expr]) -> R {
    variance_like(ev, a, true, false)
}

fn variancep(ev: &mut Evaluator, a: &[Expr]) -> R {
    variance_like(ev, a, false, false)
}

fn list(ev: &mut Evaluator, a: &[Expr]) -> R {
    let vals = values(ev, a)?;
    let parts: Vec<String> = vals
        .iter()
        .filter(|v| !v.is_empty())
        .map(Value::to_text)
        .collect();
    Ok(Value::Text(parts.join("\r")))
}

/// Sum of payment_i / (1 + rate)^i, i starting at 1.
fn npv(ev: &mut Evaluator, a: &[Expr]) -> R {
    field_alone(&a[..1])?;
    let payments = expand(ev, &a[0])?;
    let rate = ev.eval(&a[1])?.num_or_zero();
    let w = ev.scale + 10;
    let base = num::int(1) + rate;
    let mut total = Num::zero();
    let mut factor = num::int(1);
    for p in payments.iter().filter_map(Value::to_num) {
        factor = num::fix(&factor * &base);
        total += num::div(&p, &factor, w).ok_or_else(|| EvalError::generic("division by zero"))?;
    }
    Ok(Value::Number(num::round(&total, ev.scale)))
}

// ---- conversion ----------------------------------------------------------------

fn get_as_text(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::Text(a[0].to_text()))
}

fn get_as_number(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(a[0]
        .to_num()
        .map(Value::Number)
        .unwrap_or_else(Value::empty))
}

fn get_as_boolean(_: &mut Evaluator, a: &[Value]) -> R {
    Ok(Value::bool(a[0].truth()))
}
