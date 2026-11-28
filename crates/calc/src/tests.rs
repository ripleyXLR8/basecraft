//! Table-driven tests from known FileMaker behaviour.

use crate::error::code;
use crate::functions::{FUNCTIONS, GET_REFERENCE, REFERENCE, reference_name};
use crate::{MemoryContext, Value, eval_text, evaluate};

fn eval(src: &str) -> String {
    eval_text(src, &mut ctx())
}

fn ctx() -> MemoryContext {
    MemoryContext::default()
        .with_field("Name", vec![Value::text("William Smith")])
        .with_field(
            "Amount",
            vec![
                Value::int(10),
                Value::empty(),
                Value::int(20),
                Value::int(30),
            ],
        )
        .with_field(
            "Lines::Price",
            vec![Value::text("1.5"), Value::int(2), Value::text("abc")],
        )
        .with_field("T::Empty", vec![Value::empty()])
}

fn error_code(src: &str) -> u32 {
    evaluate(src, &mut ctx()).unwrap_err().code
}

fn check(cases: &[(&str, &str)]) {
    let mut failures = vec![];
    for (src, want) in cases {
        let got = eval(src);
        if got != *want {
            failures.push(format!("{src}\n    want {want:?}\n    got  {got:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn operators() {
    check(&[
        ("1 + 2 * 3", "7"),
        ("(1 + 2) * 3", "9"),
        ("10 - 2 - 3", "5"),
        ("2 ^ 3 ^ 2", "64"),
        ("-2 ^ 2", "4"),
        ("2 ^ -1", ".5"),
        ("1 / 3", ".3333333333333333"),
        ("2 / 3", ".6666666666666667"),
        ("10 / 4", "2.5"),
        ("0.1 + 0.2", ".3"),
        ("1 / 0", "?"),
        ("\"a\" & \"b\" & 1", "ab1"),
        ("\"a\" & ¶ & \"b\"", "a\rb"),
        ("\"x¶y\"", "x\ry"),
        ("\"x\\¶y\"", "x¶y"),
        ("\"say \\\"hi\\\" \\\\\"", "say \"hi\" \\"),
        ("1 + 2 & 3", "33"),
        ("\"5\" + 3", "8"),
        ("\"\" + 3", "3"),
        ("1 = 1", "1"),
        ("1 ≠ 2", "1"),
        ("1 <> 1", "0"),
        ("2 ≥ 2 and 2 <= 3", "1"),
        ("\"abc\" = \"ABC\"", "1"),
        ("\"abc\" < \"abd\"", "1"),
        ("not 0", "1"),
        ("not 1 = 0", "1"),
        ("1 and 0 or 1", "1"),
        ("1 xor 1", "0"),
        ("1 xor 0", "1"),
        ("\"abc\" and 1", "0"),
        ("\"2\" and 1", "1"),
        ("0 and (1/0)", "0"),
        ("1 or (1/0)", "1"),
        ("True + True", "2"),
        ("1000000000000000000000 * 1000000000000000000000", "1.0E+42"),
        ("123456789.123456789 * 1000", "123456789123.456789"),
        ("1 /* inline */ + // to end of line\n 1", "2"),
        ("If ( 1 , \"y\" , \"n\" )", "y"),
        ("lEfT ( \"Hello\" ; 2 )", "He"),
    ]);
}

#[test]
fn logic_functions() {
    check(&[
        ("If ( 1 ; \"yes\" ; \"no\" )", "yes"),
        ("If ( 0 ; \"yes\" )", ""),
        ("If ( \"\" ; 1 ; 2 )", "2"),
        ("If ( 1 ; 1 ; 1/0 )", "1"),
        ("Case ( 0 ; \"a\" ; 1 ; \"b\" ; \"c\" )", "b"),
        ("Case ( 0 ; \"a\" ; 0 ; \"b\" ; \"c\" )", "c"),
        ("Case ( 0 ; \"a\" )", ""),
        ("Case ( 1 ; \"a\" ; 1/0 ; \"b\" )", "a"),
        ("Choose ( 0 ; \"zero\" ; \"one\" )", "zero"),
        ("Choose ( 1 ; \"zero\" ; \"one\" )", "one"),
        ("Choose ( 1.9 ; \"zero\" ; \"one\" )", "one"),
        ("Choose ( 5 ; \"zero\" ; \"one\" )", ""),
        ("Let ( x = 5 ; x * 2 )", "10"),
        ("Let ( [ x = 5 ; y = x + 1 ] ; x & y )", "56"),
        ("Let ( [ x = 1 ; x = x + 1 ] ; x )", "2"),
        ("Let ( X = 1 ; x )", "1"),
        ("Let ( x = 1 ; Let ( x = 2 ; x ) & x )", "21"),
        ("Let ( $v = 3 ; $v + $V )", "6"),
        (
            "While ( [ i = 0 ; out = \"\" ] ; i < 3 ; [ i = i + 1 ; out = out & i ] ; out )",
            "123",
        ),
        ("While ( i = 0 ; i < 0 ; i = i + 1 ; i )", "0"),
        (
            "SetRecursion ( While ( i = 0 ; 1 ; i = i + 1 ; i ) ; 10 )",
            "?",
        ),
        ("Evaluate ( \"1 + 2\" )", "3"),
        ("Evaluate ( \"\" )", ""),
        ("Evaluate ( \"1 +\" )", "?"),
        ("Let ( x = 1 ; Evaluate ( \"x\" ) )", "?"),
        ("Let ( $x = 7 ; Evaluate ( \"$x\" ) )", "7"),
        ("EvaluationError ( 1 + 1 )", "0"),
        ("EvaluationError ( Evaluate ( \"1 +\" ) )", "1204"),
        ("EvaluationError ( NoSuchField )", "102"),
        ("EvaluationError ( Left ( \"a\" ) )", "1201"),
        ("EvaluationError ( Left ( \"a\" ; 1 ; 2 ) )", "1202"),
        ("EvaluationError ( Location ( 1 ) )", "0"),
        ("EvaluationError ( NoSuchFunction ( 1 ) )", "?"),
        ("EvaluationError ( Get ( NotAFlag ) )", "1215"),
        ("IsEmpty ( \"\" )", "1"),
        ("IsEmpty ( 0 )", "0"),
        ("IsEmpty ( T::Empty )", "1"),
        ("IsEmpty ( NoSuchField )", "1"),
        ("IsValid ( Name )", "1"),
        ("IsValid ( NoSuchField )", "0"),
        ("IsValidExpression ( \"1 + 1\" )", "1"),
        ("IsValidExpression ( \"1 +\" )", "0"),
        (
            "SetPrecision ( 5 / 9 ; 30 )",
            ".555555555555555555555555555556",
        ),
        (
            "SetPrecision ( 1.321321321321321321321321321 ; 0 )",
            "1.3213213213213213",
        ),
        ("SetPrecision ( 22 / 7 ; 20 )", "3.14285714285714285714"),
        (
            "SetPrecision ( Pi ; 30 )",
            "3.14159265358979323846264338328",
        ),
        (
            "SetPrecision ( Exp ( 50 ) ; 25 )",
            "5184705528587072464087.4533229334853848274691006",
        ),
        (
            "SetPrecision ( Average ( 5/9 ; 1/7 ; 5/7 ) ; 25 )",
            ".4708994708994708994708995",
        ),
    ]);
}

#[test]
fn fields_and_variables() {
    let mut c = ctx();
    assert_eq!(eval_text("Let ( $$g = \"kept\" ; 1 )", &mut c), "1");
    assert_eq!(eval_text("$$G", &mut c), "kept");
    assert_eq!(
        eval_text("Let ( $r[2] = \"two\" ; $r[2] & \"|\" & $r )", &mut c),
        "two|"
    );
    check(&[
        ("Name", "William Smith"),
        ("name", "William Smith"),
        ("Amount[3]", "20"),
        ("Amount[9]", ""),
        ("GetRepetition ( Amount ; 4 )", "30"),
        ("Lines::Price", "1.5"),
        ("Sum ( Amount )", "60"),
        ("Sum ( Lines::Price )", "3.5"),
        ("Sum ( 1 ; 2 ; 3 )", "6"),
        ("Sum ( Amount ; 5 )", "65"),
        ("Count ( Amount )", "3"),
        ("Count ( Lines::Price )", "3"),
        ("Average ( Amount )", "20"),
        ("Average ( 1 ; 2 )", "1.5"),
        ("Min ( Amount )", "10"),
        ("Max ( Amount ; 100 )", "100"),
        (
            "Max ( Date ( 1 ; 1 ; 2020 ) ; Date ( 1 ; 2 ; 2020 ) )",
            "1/2/2020",
        ),
        ("List ( Amount )", "10\r20\r30"),
        ("List ( \"a\" ; \"\" ; \"b\" )", "a\rb"),
        ("Last ( Amount )", "30"),
        (
            "StDev ( 2 ; 4 ; 4 ; 4 ; 5 ; 5 ; 7 ; 9 )",
            "2.1380899352993951",
        ),
        ("StDevP ( 2 ; 4 ; 4 ; 4 ; 5 ; 5 ; 7 ; 9 )", "2"),
        ("Variance ( 1 ; 2 ; 3 ; 4 )", "1.6666666666666667"),
        ("VarianceP ( 1 ; 2 ; 3 ; 4 )", "1.25"),
        ("Let ( Amount = 5 ; Sum ( Amount ) )", "5"),
        ("GetField ( \"Name\" )", "William Smith"),
        ("GetField ( \"Lines::Price\" )", "1.5"),
        ("GetFieldName ( Lines::Price )", "Lines::Price"),
        ("Extend ( Name )", "William Smith"),
        ("NoSuchField", "?"),
        ("Get ( CalculationRepetitionNumber )", "1"),
        ("Length ( Get ( UUID ) )", "36"),
        ("Get ( CurrentDate ) - Get ( CurrentDate )", "0"),
        ("Get ( AccountName )", "?"),
    ]);
    let mut c = ctx();
    c.flags.insert("accountname".into(), Value::text("Admin"));
    assert_eq!(eval_text("Get ( AccountName )", &mut c), "Admin");
}

#[test]
fn text_functions() {
    check(&[
        ("Left ( \"Hello\" ; 2 )", "He"),
        ("Left ( \"Hello\" ; 0 )", ""),
        ("Left ( \"Hello\" ; -1 )", ""),
        ("Left ( \"Hello\" ; 99 )", "Hello"),
        ("Left ( \"Hello\" ; 2.9 )", "He"),
        ("Right ( \"Hello\" ; 3 )", "llo"),
        ("Right ( \"Hello\" ; 99 )", "Hello"),
        ("Middle ( \"(408)555-9054\" ; 2 ; 3 )", "408"),
        ("Middle ( \"abcdefghij\" ; 5 ; 2 )", "ef"),
        ("Middle ( \"abc\" ; 0 ; 2 )", "ab"),
        ("Middle ( \"abc\" ; -5 ; 2 )", "ab"),
        ("Middle ( \"abc\" ; 4 ; 2 )", ""),
        ("Middle ( \"abc\" ; 2 ; 99 )", "bc"),
        ("Middle ( \"abc\" ; 2 ; -1 )", ""),
        ("Position ( \"Mississippi\" ; \"iss\" ; 1 ; 1 )", "2"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 1 ; 2 )", "5"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 3 ; 1 )", "5"),
        ("Position ( \"Mississippi\" ; \"ISS\" ; 1 ; 1 )", "2"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 0 ; 1 )", "2"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 11 ; -1 )", "5"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 100 ; -2 )", "2"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 4 ; -1 )", "2"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 1 ; 3 )", "0"),
        ("Position ( \"Mississippi\" ; \"iss\" ; 1 ; 0 )", "0"),
        ("Position ( \"aaaa\" ; \"aa\" ; 1 ; 2 )", "3"),
        ("Position ( \"abc\" ; \"\" ; 1 ; 1 )", "0"),
        (
            "Right ( Name ; Length ( Name ) - Position ( Name ; \" \" ; Length ( Name ) ; -1 ) )",
            "Smith",
        ),
        (
            "Left ( Name ; Position ( Name ; \" \" ; 1 ; 1 ) - 1 )",
            "William",
        ),
        ("PatternCount ( \"Mississippi\" ; \"ss\" )", "2"),
        ("PatternCount ( \"AAAA\" ; \"aa\" )", "2"),
        ("PatternCount ( \"abc\" ; \"\" )", "0"),
        ("Substitute ( \"Hello\" ; \"l\" ; \"L\" )", "HeLLo"),
        ("Substitute ( \"Hello\" ; \"L\" ; \"x\" )", "Hello"),
        (
            "Substitute ( \"abc\" ; [ \"a\" ; \"1\" ] ; [ \"b\" ; \"2\" ] )",
            "12c",
        ),
        (
            "Substitute ( \"abc\" ; [ \"a\" ; \"b\" ] ; [ \"b\" ; \"c\" ] )",
            "ccc",
        ),
        ("Substitute ( \"abc\" ; \"\" ; \"x\" )", "abc"),
        ("Substitute ( \"a¶b\" ; ¶ ; \", \" )", "a, b"),
        ("Trim ( \"  a b  \" )", "a b"),
        ("TrimAll ( \"  a   b  \" ; 0 ; 0 )", "a b"),
        ("Upper ( \"abc é\" )", "ABC É"),
        ("Lower ( \"ABC\" )", "abc"),
        ("Proper ( \"hello WORLD\" )", "Hello World"),
        ("Proper ( \"mary-jane o'neil\" )", "Mary-Jane O'neil"),
        ("Length ( \"Hello\" )", "5"),
        ("Length ( \"\" )", "0"),
        ("Length ( 1.50 )", "3"),
        ("Length ( \"héllo\" )", "5"),
        ("Replace ( \"Hello\" ; 2 ; 3 ; \"ipp\" )", "Hippo"),
        ("Replace ( \"abc\" ; 2 ; 0 ; \"X\" )", "aXbc"),
        (
            "Filter ( \"(408) 555-1212\" ; \"0123456789\" )",
            "4085551212",
        ),
        ("Filter ( \"AbC\" ; \"abc\" )", "b"),
        ("Exact ( \"abc\" ; \"ABC\" )", "0"),
        ("Exact ( \"abc\" ; \"abc\" )", "1"),
        ("Quote ( \"say \\\"hi\\\"\" )", "\"say \\\"hi\\\"\""),
        ("SerialIncrement ( \"abc12\" ; 1 )", "abc13"),
        ("SerialIncrement ( \"abc09\" ; 1 )", "abc10"),
        ("SerialIncrement ( \"abc99\" ; 1 )", "abc100"),
        ("SerialIncrement ( \"a1b001\" ; 2 )", "a1b003"),
        ("Char ( 65 )", "A"),
        ("Char ( 9800097 )", "ab"),
        ("Code ( \"A\" )", "65"),
        ("Code ( \"ab\" )", "9800097"),
        ("TextColor ( \"red\" ; RGB ( 255 ; 0 ; 0 ) )", "red"),
        ("TextStyleAdd ( \"x\" ; Bold + Italic )", "x"),
        ("RGB ( 255 ; 0 ; 0 )", "16711680"),
        ("RGB ( 1 ; 2 ; 3 )", "66051"),
        ("Katakana ( \"ひらがな\" )", "ヒラガナ"),
        ("Hiragana ( \"カタカナ\" )", "かたかな"),
        ("RomanZenkaku ( \"A1 \" )", "Ａ１\u{3000}"),
        ("RomanHankaku ( \"Ａ１\" )", "A1"),
    ]);
}

#[test]
fn value_lists_and_words() {
    check(&[
        ("ValueCount ( \"a¶b¶c\" )", "3"),
        ("ValueCount ( \"a¶b¶\" )", "2"),
        ("ValueCount ( \"a¶¶b\" )", "3"),
        ("ValueCount ( \"\" )", "0"),
        ("GetValue ( \"a¶b¶c\" ; 2 )", "b"),
        ("GetValue ( \"a¶b¶c\" ; 4 )", ""),
        ("GetValue ( \"a¶b¶c\" ; 0 )", ""),
        ("LeftValues ( \"a¶b¶c\" ; 2 )", "a\rb\r"),
        ("RightValues ( \"a¶b¶c\" ; 2 )", "b\rc\r"),
        ("MiddleValues ( \"a¶b¶c¶d\" ; 2 ; 2 )", "b\rc\r"),
        ("MiddleValues ( \"a¶b\" ; 5 ; 2 )", ""),
        (
            "FilterValues ( \"Plaid¶Canvas¶Suitcase\" ; \"Plaid¶Canvas\" )",
            "Plaid\rCanvas\r",
        ),
        (
            "FilterValues ( \"Small¶Medium¶Large\" ; \"medium¶small\" )",
            "Small\rMedium\r",
        ),
        ("FilterValues ( \"a¶b\" ; \"\" )", ""),
        ("SortValues ( \"b¶C¶a\" )", "a\rb\rC\r"),
        ("SortValues ( \"10¶9¶100\" ; 2 )", "9\r10\r100\r"),
        ("SortValues ( \"10¶9¶100\" )", "10\r100\r9\r"),
        ("UniqueValues ( \"Apple¶apple¶Banana\" )", "Apple\rBanana\r"),
        ("UniqueValues ( \"1¶1.0¶2\" ; 2 )", "1\r2\r"),
        ("WordCount ( \"Plaid Canvas Shirt\" )", "3"),
        ("WordCount ( \"  one,two;three  \" )", "3"),
        ("WordCount ( \"It's 3.14 or 1,000\" )", "4"),
        ("WordCount ( \"\" )", "0"),
        ("LeftWords ( \"Plaid Canvas Shirt\" ; 2 )", "Plaid Canvas"),
        ("LeftWords ( \"  Plaid, Canvas!  \" ; 9 )", "Plaid, Canvas"),
        ("RightWords ( \"Plaid Canvas Shirt\" ; 1 )", "Shirt"),
        ("MiddleWords ( \"Plaid Canvas Shirt\" ; 2 ; 1 )", "Canvas"),
        (
            "MiddleWords ( \"Plaid Canvas Shirt\" ; 2 ; 9 )",
            "Canvas Shirt",
        ),
        ("LeftWords ( \"abc\" ; 0 )", ""),
    ]);
}

#[test]
fn number_functions() {
    check(&[
        ("Abs ( -2.5 )", "2.5"),
        ("Abs ( \"\" )", ""),
        ("Round ( 2.5 ; 0 )", "3"),
        ("Round ( -2.5 ; 0 )", "-3"),
        ("Round ( 3.14159 ; 2 )", "3.14"),
        ("Round ( 1234.5 ; -2 )", "1200"),
        ("Round ( 1.005 ; 2 )", "1.01"),
        ("Truncate ( 3.789 ; 2 )", "3.78"),
        ("Truncate ( -3.789 ; 0 )", "-3"),
        ("Truncate ( 1789 ; -2 )", "1700"),
        ("Int ( 2.9 )", "2"),
        ("Int ( -2.9 )", "-2"),
        ("Floor ( -2.1 )", "-3"),
        ("Floor ( 2.9 )", "2"),
        ("Ceiling ( 2.1 )", "3"),
        ("Ceiling ( -2.9 )", "-2"),
        ("Mod ( 7 ; 3 )", "1"),
        ("Mod ( -7 ; 3 )", "2"),
        ("Mod ( 7 ; -3 )", "-2"),
        ("Mod ( -7 ; -3 )", "-1"),
        ("Mod ( 5.5 ; 2 )", "1.5"),
        ("Mod ( 5 ; 0 )", "5"),
        ("Div ( 7 ; 3 )", "2"),
        ("Div ( -7 ; 3 )", "-3"),
        ("Div ( 2.5 ; 1 )", "2"),
        ("Sign ( -4 )", "-1"),
        ("Sign ( 0 )", "0"),
        ("Sign ( 0.1 )", "1"),
        ("Sqrt ( 4 )", "2"),
        ("Sqrt ( 2 )", "1.414213562373095"),
        ("Sqrt ( -1 )", "?"),
        ("Exp ( 0 )", "1"),
        ("Exp ( 1 )", "2.7182818284590452"),
        ("Ln ( 1 )", "0"),
        ("Ln ( 0 )", "?"),
        ("Log ( 1000 )", "3"),
        ("Log ( 0.01 )", "-2"),
        ("Lg ( 1024 )", "10"),
        ("Pi", "3.1415926535897932"),
        ("Round ( Pi ; 4 )", "3.1416"),
        ("Sin ( 0 )", "0"),
        ("Cos ( 0 )", "1"),
        ("Round ( Sin ( Pi / 2 ) ; 10 )", "1"),
        ("Round ( Atan ( 1 ) * 4 ; 10 )", "3.1415926536"),
        ("Round ( Degrees ( Pi ) ; 10 )", "180"),
        ("Radians ( 180 )", "3.1415926535897932"),
        ("Factorial ( 5 )", "120"),
        ("Factorial ( 5 ; 2 )", "20"),
        ("Factorial ( 0 )", "1"),
        ("Factorial ( 25 )", "15511210043330985984000000"),
        ("Factorial ( -1 )", "?"),
        ("Combination ( 5 ; 2 )", "10"),
        ("Combination ( 52 ; 5 )", "2598960"),
        ("Random < 1 and Random ≥ 0", "1"),
        ("Round ( FV ( 50 ; .1 / 12 ; 120 ) ; 2 )", "10242.25"),
        ("Round ( PV ( 1000 ; .1 ; 10 ) ; 4 )", "6144.5671"),
        ("Round ( PMT ( 10000 ; .08 / 12 ; 36 ) ; 2 )", "313.36"),
        ("PMT ( 1200 ; 0 ; 12 )", "100"),
        ("Round ( NPV ( Amount ; 0 ) ; 2 )", "60"),
        ("GetAsNumber ( \"abc12d3.4\" )", "123.4"),
        ("GetAsNumber ( \"abc\" )", ""),
        ("GetAsNumber ( \"-5 apples\" )", "-5"),
        ("GetAsNumber ( \"1/1/2020\" )", "112020"),
        ("GetAsNumber ( \"$1,234.50\" )", "1234.5"),
        ("GetAsBoolean ( \"abc\" )", "0"),
        ("GetAsBoolean ( \"0\" )", "0"),
        ("GetAsBoolean ( \"x1\" )", "1"),
        ("GetAsBoolean ( -0.5 )", "1"),
        ("GetAsText ( 1 / 4 )", ".25"),
        ("GetAsText ( 1.10 )", "1.1"),
    ]);
}

#[test]
fn npv_matches_documented_example() {
    let mut c = MemoryContext::default().with_field(
        "Loan",
        [-2000, 600, 300, 500, 700, 400]
            .into_iter()
            .map(Value::int)
            .collect(),
    );
    assert!(eval_text("NPV ( Loan ; .05 )", &mut c).starts_with("156.91277445"));
}

#[test]
fn dates_and_times() {
    check(&[
        ("Date ( 1 ; 15 ; 2024 )", "1/15/2024"),
        ("Date ( 13 ; 1 ; 2023 )", "1/1/2024"),
        ("Date ( 3 ; 0 ; 2024 )", "2/29/2024"),
        ("Date ( 1 ; 1 ; 1 )", "1/1/0001"),
        ("GetAsNumber ( Date ( 1 ; 1 ; 1 ) )", "1"),
        ("GetAsNumber ( Date ( 1 ; 1 ; 2000 ) )", "730120"),
        ("Date ( 1 ; 1 ; 0 )", "?"),
        ("Date ( 1 ; 1 ; 4001 )", "?"),
        ("Date ( 1 ; 15 ; 2024 ) + 20", "2/4/2024"),
        ("Date ( 1 ; 15 ; 2024 ) - 15", "12/31/2023"),
        ("Date ( 3 ; 1 ; 2024 ) - Date ( 2 ; 1 ; 2024 )", "29"),
        ("Year ( Date ( 7 ; 4 ; 1776 ) )", "1776"),
        ("Month ( \"7/4/1776\" )", "7"),
        ("Day ( Date ( 7 ; 4 ; 1776 ) )", "4"),
        ("DayOfWeek ( Date ( 1 ; 1 ; 2024 ) )", "2"),
        ("DayName ( Date ( 1 ; 1 ; 2024 ) )", "Monday"),
        ("DayNameJ ( Date ( 1 ; 7 ; 2024 ) )", "日曜日"),
        ("MonthName ( Date ( 12 ; 1 ; 2024 ) )", "December"),
        ("DayOfYear ( Date ( 12 ; 31 ; 2024 ) )", "366"),
        ("WeekOfYear ( Date ( 1 ; 1 ; 2024 ) )", "1"),
        ("WeekOfYear ( Date ( 1 ; 7 ; 2024 ) )", "2"),
        ("WeekOfYear ( Date ( 12 ; 31 ; 2022 ) )", "53"),
        ("WeekOfYearFiscal ( Date ( 1 ; 1 ; 2021 ) ; 2 )", "53"),
        ("WeekOfYearFiscal ( Date ( 1 ; 4 ; 2021 ) ; 2 )", "1"),
        ("WeekOfYearFiscal ( Date ( 12 ; 31 ; 2024 ) ; 2 )", "1"),
        ("Year ( \"\" )", ""),
        ("Year ( \"nonsense\" )", "?"),
        ("Time ( 9 ; 5 ; 0 )", "9:05:00"),
        ("Time ( 25 ; 0 ; 0 )", "25:00:00"),
        ("Time ( 0 ; 0 ; 1.5 )", "0:00:01.5"),
        ("Time ( 0 ; 90 ; 0 )", "1:30:00"),
        ("Time ( -1 ; 0 ; 0 )", "-1:00:00"),
        ("GetAsNumber ( Time ( 1 ; 0 ; 0 ) )", "3600"),
        ("Time ( 9 ; 0 ; 0 ) + 60", "9:01:00"),
        ("Time ( 10 ; 0 ; 0 ) - Time ( 9 ; 30 ; 0 )", "0:30:00"),
        ("Hour ( Time ( 13 ; 5 ; 9 ) )", "13"),
        ("Minute ( Time ( 13 ; 5 ; 9 ) )", "5"),
        ("Seconds ( Time ( 13 ; 5 ; 9.25 ) )", "9.25"),
        ("Hour ( \"1:30 PM\" )", "13"),
        (
            "Timestamp ( Date ( 1 ; 15 ; 2024 ) ; Time ( 13 ; 5 ; 9 ) )",
            "1/15/2024 1:05:09 PM",
        ),
        ("GetAsNumber ( Timestamp ( Date ( 1 ; 1 ; 1 ) ; 0 ) )", "0"),
        (
            "GetAsNumber ( Timestamp ( Date ( 1 ; 2 ; 1 ) ; 1 ) )",
            "86401",
        ),
        (
            "Timestamp ( Date ( 1 ; 15 ; 2024 ) ; Time ( 23 ; 0 ; 0 ) ) + 7200",
            "1/16/2024 1:00 AM",
        ),
        (
            "Hour ( Timestamp ( Date ( 1 ; 15 ; 2024 ) ; Time ( 23 ; 0 ; 0 ) ) )",
            "23",
        ),
        ("Year ( Timestamp ( Date ( 1 ; 15 ; 2024 ) ; 0 ) )", "2024"),
        ("GetAsDate ( \"1/15/2024\" )", "1/15/2024"),
        ("GetAsDate ( \"2024-01-15\" )", "?"),
        ("GetAsDate ( 738900 )", "1/15/2024"),
        ("GetAsDate ( \"x\" )", "?"),
        ("GetAsTime ( \"13:05\" )", "13:05:00"),
        // Read with AM/PM: shown in the 12-hour form (oracle).
        ("GetAsTime ( \"1:05:09 pm\" )", "1:05:09 PM"),
        ("GetAsTime ( \"1:05:09 pm\" ) + 60", "13:06:09"),
        ("Hour ( GetAsTime ( \"1:05:09 pm\" ) )", "13"),
        ("GetAsTime ( 3661 )", "1:01:01"),
        (
            "GetAsTimestamp ( \"1/15/2024 13:05:09\" )",
            "1/15/2024 1:05:09 PM",
        ),
        ("GetAsTimestamp ( \"1/15/2024\" )", "1/15/2024 12:00 AM"),
        ("GetAsTimestamp ( 86400 )", "1/2/0001 12:00 AM"),
        ("GetAsText ( Date ( 2 ; 3 ; 2024 ) )", "2/3/2024"),
        ("Date ( 2 ; 3 ; 2024 ) & \"\"", "2/3/2024"),
        ("Date ( 2 ; 3 ; 2024 ) = \"2/3/2024\"", "1"),
        ("Date ( 2 ; 3 ; 2024 ) > Date ( 2 ; 2 ; 2024 )", "1"),
    ]);
}

#[test]
fn json() {
    check(&[
        (
            "JSONSetElement ( \"{}\" ; \"a\" ; 1 ; JSONNumber )",
            "{\"a\":1}",
        ),
        (
            "JSONSetElement ( \"\" ; \"a\" ; \"x\" ; JSONString )",
            "{\"a\":\"x\"}",
        ),
        ("JSONSetElement ( \"{}\" ; \"b\" ; 1 ; 2 )", "{\"b\":1}"),
        (
            "JSONSetElement ( \"{\\\"b\\\":1}\" ; \"a\" ; 2 ; JSONNumber )",
            "{\"a\":2,\"b\":1}",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"a.b[1]\" ; true ; JSONBoolean )",
            "{\"a\":{\"b\":[null,true]}}",
        ),
        (
            "JSONSetElement ( \"[1]\" ; \"[+]\" ; 2 ; JSONNumber )",
            "[1,2]",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"a\" ; \"[1,2]\" ; JSONArray )",
            "{\"a\":[1,2]}",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"a\" ; \"\" ; JSONObject )",
            "{\"a\":{}}",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"a\" ; \"\" ; JSONNull )",
            "{\"a\":null}",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"a\" ; 1/4 ; JSONNumber )",
            "{\"a\":0.25}",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"a\" ; \"x¶y\\\"\" ; JSONString )",
            "{\"a\":\"x\\ry\\\"\"}",
        ),
        (
            "JSONSetElement ( \"{}\" ; [ \"a\" ; 1 ; JSONNumber ] ; [ \"b\" ; \"t\" ; JSONString ] )",
            "{\"a\":1,\"b\":\"t\"}",
        ),
        (
            "JSONSetElement ( \"{}\" ; \"['a.b']\" ; 1 ; JSONNumber )",
            "{\"a.b\":1}",
        ),
        (
            "JSONGetElement ( \"{ \\\"a\\\" : 11, \\\"b\\\" : 22 }\" ; \"b\" )",
            "22",
        ),
        ("JSONGetElement ( \"[ true, false, true ]\" ; 1 )", "0"),
        ("JSONGetElement ( \"[ 1, 2, 3, 5 ]\" ; \"[:]\" )", "5"),
        (
            "JSONGetElement ( \"{\\\"a\\\":{\\\"b\\\":[1,{\\\"c\\\":\\\"deep\\\"}]}}\" ; \"a.b[1].c\" )",
            "deep",
        ),
        (
            "JSONGetElement ( \"{\\\"a\\\":{\\\"b\\\":2}}\" ; \"a\" )",
            "{\"b\":2}",
        ),
        ("JSONGetElement ( \"{\\\"a\\\":null}\" ; \"a\" )", ""),
        ("JSONGetElement ( \"{\\\"a\\\":1}\" ; \"zz\" )", ""),
        ("JSONGetElement ( \"{\\\"a\\\":1.50}\" ; \"a\" ) + 1", "2.5"),
        ("Left ( JSONGetElement ( \"{bad\" ; \"a\" ) ; 2 )", "? "),
        (
            "EvaluationError ( JSONGetElement ( \"{bad\" ; \"a\" ) )",
            "0",
        ),
        (
            "JSONGetElementType ( \"{\\\"a\\\":\\\"x\\\"}\" ; \"a\" )",
            "1",
        ),
        ("JSONGetElementType ( \"{\\\"a\\\":[]}\" ; \"a\" )", "4"),
        (
            "JSONListKeys ( \"{\\\"b\\\":1,\\\"a\\\":2}\" ; \"\" )",
            "a\rb",
        ),
        ("JSONListKeys ( \"[5,6,7]\" ; \"\" )", "0\r1\r2"),
        (
            "JSONListValues ( \"{\\\"b\\\":1,\\\"a\\\":\\\"x\\\"}\" ; \"\" )",
            "x\r1",
        ),
        (
            "JSONDeleteElement ( \"{\\\"a\\\":1,\\\"b\\\":2}\" ; \"a\" )",
            "{\"b\":2}",
        ),
        ("JSONDeleteElement ( \"[1,2,3]\" ; \"[1]\" )", "[1,3]"),
        (
            "JSONDeleteElement ( \"{\\\"a\\\":{\\\"b\\\":1,\\\"c\\\":2}}\" ; \"a.b\" )",
            "{\"a\":{\"c\":2}}",
        ),
        (
            "JSONFormatElements ( \"{\\\"a\\\":{\\\"lnk\\\":false,\\\"id\\\":12}}\" )",
            "{\r\t\"a\" : \r\t{\r\t\t\"id\" : 12,\r\t\t\"lnk\" : false\r\t}\r}",
        ),
        ("JSONFormatElements ( \"[]\" )", "[]"),
        (
            "JSONMakeArray ( \"1¶2¶3\" ; \"¶\" ; JSONNumber )",
            "[1,2,3]",
        ),
        (
            "JSONMakeArray ( \"a,b\" ; \",\" ; JSONString )",
            "[\"a\",\"b\"]",
        ),
    ]);
}

#[test]
fn encodings() {
    check(&[
        ("Base64Encode ( \"FileMaker\" )", "RmlsZU1ha2Vy\r\n"),
        ("Base64Encode ( \"ab\" )", "YWI=\r\n"),
        ("Base64Encode ( \"a\" )", "YQ==\r\n"),
        ("Base64Decode ( \"RmlsZU1ha2Vy\" )", "FileMaker"),
        (
            "Base64Decode ( Base64Encode ( \"héllo wörld\" ) )",
            "héllo wörld",
        ),
        (
            "Length ( Base64Encode ( Left ( \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\" ; 60 ) ) )",
            "84",
        ),
        (
            "Length ( Base64EncodeRFC ( 4648 ; Left ( \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\" ; 60 ) ) )",
            "80",
        ),
        ("HexEncode ( \"FileMaker\" )", "46696C654D616B6572"),
        ("HexDecode ( \"46696C654D616B6572\" )", "FileMaker"),
        (
            "HexEncode ( CryptDigest ( \"abc\" ; \"MD5\" ) )",
            "900150983CD24FB0D6963F7D28E17F72",
        ),
        (
            "HexEncode ( CryptDigest ( \"abc\" ; \"SHA1\" ) )",
            "A9993E364706816ABA3E25717850C26C9CD0D89D",
        ),
        (
            "HexEncode ( CryptDigest ( \"abc\" ; \"SHA256\" ) )",
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        ),
        (
            "Base64Encode ( CryptDigest ( \"\" ; \"SHA256\" ) )",
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=\r\n",
        ),
        ("CryptDigest ( \"abc\" ; \"nope\" )", "?"),
        (
            "HexEncode ( CryptAuthCode ( \"The quick brown fox jumps over the lazy dog\" ; \"SHA256\" ; \"key\" ) )",
            "F7BC83F430538424B13298E6AA6FB143EF4D59A14946175997479DBC2D1A3CD8",
        ),
        (
            "HexEncode ( CryptAuthCode ( \"The quick brown fox jumps over the lazy dog\" ; \"MD5\" ; \"key\" ) )",
            "80070713463E7749B90C2DC24911E275",
        ),
        ("GetAsURLEncoded ( \"a b&c=é\" )", "a%20b%26c%3D%C3%A9"),
        (
            "TextDecode ( TextEncode ( \"a¶b\" ; \"utf-8\" ; 4 ) ; \"utf-8\" )",
            "a\rb",
        ),
        ("TextEncode ( \"a\" ; \"utf-8\" ; 1 )", "utf-8.txt"),
    ]);
}

#[test]
fn errors_never_panic() {
    // Every reference function must parse and evaluate (to a value or an
    // error) with any number of arguments.
    for line in REFERENCE.lines() {
        let name = reference_name(line);
        let big = "99999999999999999999";
        let extremes = format!("{big} ; -{big} ; {big} ; -{big}");
        let min = "-9223372036854775808 ; -9223372036854775808 ; -9223372036854775808 ; -9223372036854775808";
        let tiny = "0.0000000000000000000001 ; -.5 ; 0 ; \"\"";
        for args in [
            "",
            "1",
            "1 ; 2",
            "\"a\" ; \"b\" ; 3",
            "1;2;3;4;5",
            &extremes,
            min,
            tiny,
        ] {
            let src = format!("{name} ( {args} )");
            let _ = evaluate(&src, &mut ctx());
        }
        let _ = evaluate(name, &mut ctx());
    }
    assert_eq!(eval("ExecuteSQL ( \"x\" ; \"\" ; \"\" )"), "?");
    assert_eq!(error_code("Self"), code::UNAVAILABLE);
    assert_eq!(eval("FieldNames ( \"\" ; \"\" )"), "");
    // Deep nesting and self-evaluating text fail cleanly.
    let deep = format!("{}1{}", "(".repeat(150), ")".repeat(150));
    assert_eq!(eval(&deep), "1");
    let too_deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert_eq!(eval(&too_deep), "?");
    assert_eq!(eval(&"-".repeat(5000)), "?");
    // Literals beyond FileMaker's range and infinite JSON numbers.
    assert_eq!(eval(&format!(".{}1", "0".repeat(900))), "?");
    let _ = evaluate("JSONFormatElements ( \"[1e999]\" )", &mut ctx());
    assert_eq!(
        eval("Let ( $x = \"Evaluate ( $x )\" ; Evaluate ( $x ) )"),
        "?"
    );
}

/// Prints coverage against docs/reference/functions.txt and checks the
/// registry only names real functions (plus `Get`).
#[test]
fn registry_coverage() {
    let reference: Vec<&str> = REFERENCE
        .lines()
        .map(reference_name)
        .filter(|n| !n.is_empty())
        .collect();
    let implemented = reference
        .iter()
        .filter(|n| crate::functions::lookup(n).is_some())
        .count();
    for f in FUNCTIONS {
        assert!(
            f.name == "Get" || reference.iter().any(|r| r.eq_ignore_ascii_case(f.name)),
            "{} is not a FileMaker function",
            f.name
        );
    }
    let missing: Vec<&&str> = reference
        .iter()
        .filter(|n| crate::functions::lookup(n).is_none())
        .collect();
    let flags: Vec<&str> = GET_REFERENCE
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Get ( ")?.strip_suffix(" )"))
        .filter(|f| *f != "flag")
        .collect();
    let builtin = flags
        .iter()
        .filter(|f| crate::functions::builtin_get(f).is_some())
        .count();
    println!("functions: {implemented} / {} implemented", reference.len());
    println!("missing: {missing:?}");
    println!("Get() flags built in: {builtin} / {}", flags.len());
    assert_eq!(reference.len(), 234);
    assert_eq!(flags.len(), 139); // plus the "Get ( flag )" header line
}

/// The custom functions pasted into FileMaker Pro 26 to check these
/// results (FORMAT.md §16).
fn custom_ctx() -> MemoryContext {
    let defs: [(&str, &str, &str); 7] = [
        ("Fact", "n", "If ( n <= 1 ; 1 ; n * Fact ( n - 1 ) )"),
        ("AddTwo", "a;b", "a + b"),
        ("Hello", "", "\"Hello\""),
        (
            "WhoAmI",
            "",
            "Get ( AccountName ) & \"/\" & Get ( RecordNumber )",
        ),
        ("Deep", "n", "If ( n <= 0 ; 0 ; 1 + Deep ( n - 1 ) )"),
        (
            "Tail",
            "n;acc",
            "If ( n <= 0 ; acc ; Tail ( n - 1 ; acc + 1 ) )",
        ),
        ("Sq", "x", "x * x // square"),
    ];
    let names: Vec<String> = defs.iter().map(|d| d.0.to_lowercase()).collect();
    let mut c = MemoryContext::default();
    for (name, params, formula) in defs {
        let params = params
            .split(';')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        let f = crate::CustomFunction::new(name, params, formula, &|n| {
            names.contains(&n.to_lowercase())
        });
        c.custom.insert(name.to_lowercase(), std::sync::Arc::new(f));
    }
    c.flags.insert("accountname".into(), Value::text("Admin"));
    c.flags.insert("recordnumber".into(), Value::int(0));
    c
}

#[test]
fn custom_functions_match_filemaker() {
    let cases = [
        ("Fact ( 5 )", "120"),
        ("Fact ( 20 )", "2432902008176640000"),
        ("AddTwo ( 2 ; 3 )", "5"),
        ("AddTwo ( \"a\" ; 1 )", "1"),
        ("Hello", "Hello"),
        ("Hello & \"!\"", "Hello!"),
        ("WhoAmI", "Admin/0"),
        ("Sq ( 7 )", "49"),
        ("Deep ( 100 )", "100"),
        ("Tail ( 10001 ; 0 )", "10001"),
        ("Let ( n = 3 ; Fact ( n ) + n )", "9"),
        ("Fact ( Fact ( 3 ) )", "720"),
        ("Fact ( -1 )", "1"),
        ("Sq ( \"abc\" )", "0"),
        ("If ( 1 ; 2 ; Tail ( 49999 ; 0 ) )", "2"),
        ("Evaluate ( \"Fact ( 4 )\" )", "24"),
    ];
    let mut failures = vec![];
    for (src, want) in cases {
        let got = eval_text(src, &mut custom_ctx());
        if got != want {
            failures.push(format!("{src}: want {want:?}, got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let err = |src: &str| evaluate(src, &mut custom_ctx()).unwrap_err().code;
    assert_eq!(err("AddTwo ( 1 )"), code::TOO_FEW_PARAMS);
    assert_eq!(err("Hello ( 1 )"), code::TOO_MANY_PARAMS);
    assert_eq!(err("AddTwo ( 1 ; 2 ; 3 )"), code::TOO_MANY_PARAMS);
}

/// FileMaker allows 49,999 custom function calls in one calculation, deep
/// or not; the 50,000th makes it "?", which EvaluationError does not catch.
/// An Evaluate counts its own calls.
#[test]
fn custom_function_call_limit() {
    let r = |src: &str| eval_text(src, &mut custom_ctx());
    assert_eq!(r("Tail ( 49998 ; 0 )"), "49998");
    assert_eq!(r("Deep ( 49998 )"), "49998");
    assert_eq!(r("Tail ( 49999 ; 0 )"), "?");
    assert_eq!(r("Deep ( 49999 )"), "?");
    assert_eq!(r("Tail ( 20000 ; 0 ) & \"|\" & Tail ( 29998 ; 0 )"), "?");
    assert_eq!(r("EvaluationError ( Tail ( 49999 ; 0 ) )"), "?");
    assert_eq!(r("IsEmpty ( Tail ( 49999 ; 0 ) )"), "?");
    assert_eq!(
        r("Evaluate ( \"Tail ( 30000 ; 0 )\" ) + Evaluate ( \"Tail ( 30000 ; 0 )\" )"),
        "60000"
    );
    assert_eq!(r("EvaluationError ( Deep ( 20000 ) )"), "0");
}
