//! Tests for the functions that need host data (design, records, SQL) and
//! the newer pure functions (JSON, Japanese, crypto, embeddings, paths).
//! Expected values follow Claris's documented examples where there are
//! some.

use crate::schema::*;
use crate::{Context, EvalError, MemoryContext, Value, eval_text};
use std::sync::Arc;

fn field(id: u64, name: &str, data_type: DataType) -> Field {
    Field {
        id,
        name: name.into(),
        data_type,
        repetitions: 1,
        ..Default::default()
    }
}

fn object(name: &str, field: &str) -> LayoutObject {
    LayoutObject {
        name: name.into(),
        field: Some(field.into()),
        ..Default::default()
    }
}

fn schema() -> Schema {
    let customers = BaseTable {
        id: 129,
        name: "Customers".into(),
        comment: "People we sell to".into(),
        fields: vec![
            Field {
                indexed: true,
                comment: "Unique identifier".into(),
                ..field(1, "PrimaryKey", DataType::Text)
            },
            Field {
                comment: "[LLM] Full name".into(),
                ..field(2, "Name", DataType::Text)
            },
            Field {
                repetitions: 3,
                ..field(3, "Phone Number", DataType::Text)
            },
            Field {
                storage: Storage::StoredCalc,
                indexed: true,
                ..field(4, "Current Balance", DataType::Number)
            },
            Field {
                storage: Storage::Global,
                ..field(5, "Today's Date", DataType::Date)
            },
            Field {
                storage: Storage::ExternalSecure,
                ..field(6, "Statement", DataType::Container)
            },
            Field {
                display_names: vec![
                    ("fm_table_view".into(), "Given Name".into()),
                    ("fm_common".into(), "First Name".into()),
                ],
                ..field(7, "FirstName", DataType::Text)
            },
            Field {
                next_serial: Some("1005".into()),
                ..field(8, "Serial", DataType::Number)
            },
        ],
    };
    let invoices = BaseTable {
        id: 130,
        name: "Invoices".into(),
        comment: String::new(),
        fields: vec![
            Field {
                annotation: "Unique ID".into(),
                ..field(1, "PrimaryKey", DataType::Text)
            },
            Field {
                annotation: "Customer of the invoice".into(),
                ..field(2, "CustomerID", DataType::Text)
            },
            Field {
                annotation: "Invoice total".into(),
                ..field(3, "Total", DataType::Number)
            },
            field(4, "Notes", DataType::Text),
        ],
    };
    let tab = LayoutObject {
        is_group: true,
        children: vec![LayoutObject {
            name: "btn".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let portal = LayoutObject {
        name: "invoices".into(),
        is_group: true,
        children: vec![object("total", "Invoices::Total")],
        ..Default::default()
    };
    Schema {
        file: "Contacts".into(),
        base_tables: vec![customers, invoices],
        occurrences: vec![
            Occurrence {
                id: 1065089,
                name: "Customers".into(),
                base_table: "Customers".into(),
                source: String::new(),
            },
            Occurrence {
                id: 1065090,
                name: "Invoices".into(),
                base_table: "Invoices".into(),
                source: String::new(),
            },
        ],
        relationships: vec![Relationship {
            left: "Customers".into(),
            right: "Invoices".into(),
            criteria: vec![("PrimaryKey".into(), "=".into(), "CustomerID".into())],
            left_options: RelOptions::default(),
            right_options: RelOptions {
                create: true,
                sorted: true,
                delete: false,
            },
        }],
        layouts: vec![Layout {
            id: 1,
            name: "Data Entry".into(),
            occurrence: "Customers".into(),
            objects: vec![
                LayoutObject {
                    bounds: [36.0, 48.0, 295.0, 65.0],
                    ..object("nameField", "Customers::Name")
                },
                LayoutObject {
                    repetitions_shown: 3,
                    vertical: true,
                    ..object("", "Customers::Phone Number")
                },
                LayoutObject {
                    style: "RadioButton".into(),
                    value_list: "Yes/No List".into(),
                    ..object("", "Customers::Current Balance")
                },
                portal,
                tab,
            ],
        }],
        scripts: vec![
            Named {
                id: 1,
                name: "Start".into(),
            },
            Named {
                id: 2,
                name: "-".into(),
            },
            Named {
                id: 3,
                name: "Report".into(),
            },
        ],
        value_lists: vec![ValueList {
            id: 1,
            name: "Yes/No List".into(),
            items: vec!["Yes".into(), "No".into()],
        }],
    }
}

const PRIVATE_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----
MIICXAIBAAKBgQClXlUitJC19Rjh5QYIMwcpPh4f7/8/p3ohV+1GhhS61VC4DubG
CjdgvWcENL5qK8ohyD1bJvLt1iFpwtODzT7f3OTLlu2VsohNnCBCFMANeR8gwX/W
a9ljB3pquDb9OV2HPyOUMDka1haXWzvVH8NJh5iMhaFtnoyOnDSvDnCBaQIDAQAB
AoGBAImfWQDZ/ri/cmbEn+GA/guvu4hexjEmYviO4VPY0DWww016cUoNRIsI9UvF
zVY0fTLiH1XVs0SF+Sa4UikVTaXPS44lv4IOj3aeWtdNzx8O5bzNbq4nGVvhlPMd
2W7R6gCWOIboIS1TxLNEx+svUaRjVtEJ4BkQ+0lfcoY9+c11AkEA2882ZbeDNa7D
rvJ0nNFNHXhBK9xaW49eSe9vX7tyXcBk+Sh2pUPF74nOcrjGDryscoAPJ+4Qppol
FUVn8ZjzwwJBAMCYeqSlOZwISY/5P6irkO/zZrE+qmCoQR2vYgy/NDI2tb6D2PNQ
NsRl+OdodbBWCZBM0XSU9SELH3ryiWPp/2MCQFsjNlXEWhXXCXGvO6SNr6CpbLQn
4BsYfarAKPiTet7eV79rBI6PZDFmfP/aFtyjZAjxidcvgR3iyb9hELD+jwsCQHD+
hKXoUh5bNl2DzHd3APfmg89lhqxiyqFR7mKwIz16n7KAxF4sJ5WehiOuQ64LLF0+
1HxjkFWsovGNLlIyTSUCQCulbFpHuZyMJZ4qbr8GHPhVUvKo3otbrd2dYS7g7mZ2
lPzDgLYLuw8iWmdBtPSdcodTWVePG3AYjrNV6g6TGqg=
-----END RSA PRIVATE KEY-----";

/// The same key, encrypted with password "secret" (openssl rsa -des3).
const ENCRYPTED_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----
Proc-Type: 4,ENCRYPTED
DEK-Info: DES-EDE3-CBC,DFD99ACF9EF125EF

20eBZ3khTBNebbvBh9yDm+sJHKk/fk3EXgZaRTR7MbLyibZIuB7vdtiIlLR/tmDI
eofvdh2zYxDgzFi/jWxStOUSL6I9CCU2th3NT/j7zW5dBjWUiocs9b8ZDw/+Z+Cb
oMq+w9jkXDrNKpuv92f1iYQfl55BpHco7W3Sa+tGnXXwLq7Jzri+rLDhedGMyr62
LcdCy4SYlY3LeLOKzoRIw2jCiLLNSoZs4LwCIm0zIXWffT2X2sjttOTaVXQ2xJRX
0kfFHVj/Z3WwsP/WRVTyfnLIaabaeN/YVIwwZSUdwzd+vQpJAqQG5CfBHSFT1WU1
Y6zC9Qjd8SEW2w/qbAhuK0JFVvSUBBPtq3FKyU6aOtn23yI0M4HGfYBZtOG/ozFE
5a7E98MUDf99csQ4gu30EXhMSSB+HjbcZ5dNSWjYJ92++C8bUQS3j2Cg9Y2/9XDY
Io6Mmcj9JdZdt1tUM7C18szO1907pI5HIkHzKilUoxTUDNTAcBV1idTIR/zuUoc+
In8mqGx/aB3+LU6JtDkSx+CF9ZNwXW5JzE21RRjsOprhhoiqaNB72So7RaRC8D0q
+eapGY2/0RnnU1u8wWGlpGHbHZ/SvtBEU1jQXhZwODzhnfX8ToB2PCd4OP8xi/WX
6adfwZt8LY6jo2kTbVf9zJzF7aAZh5ufTXC9IWjO6zplpnDlGVUyzKr0FlvoDP4j
wTm0/69z1uVsjgzR18kwgfaV9CHXX+d633pwuX/MXaC4El5loVFVG/BPysJY6T9G
SDPDuHPkWDjhvZm6WGc3BBxv0q4cldEcovXAo9xPom2umdzsq0TIpQ==
-----END RSA PRIVATE KEY-----";

const PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQClXlUitJC19Rjh5QYIMwcpPh4f
7/8/p3ohV+1GhhS61VC4DubGCjdgvWcENL5qK8ohyD1bJvLt1iFpwtODzT7f3OTL
lu2VsohNnCBCFMANeR8gwX/Wa9ljB3pquDb9OV2HPyOUMDka1haXWzvVH8NJh5iM
haFtnoyOnDSvDnCBaQIDAQAB
-----END PUBLIC KEY-----";

/// printf hello | openssl dgst -sha256 -sign key.pem | xxd -p
const HELLO_SHA256_SIG: &str = "0EC25867817324F1C1E194A63039ED1F1B08F8D7B71DAB8B4FBF3D1F51723A919DA2420C70CF1F95A612881F1C673E0A588F431DDD417B300B244AE101FE72CE62578327753E2F8E44AD67FD6322B710F077E96F4D12747F47C70366DA10C13153EF09E854E7DE299A81172DB99EAA787E8E7A56562702782EA5A384136C47A0";

fn ctx() -> MemoryContext {
    let mut c = MemoryContext::default()
        .with_field("Lines::Price", vec![Value::text("1.5"), Value::int(2)])
        .with_field("Empty::Rel", vec![])
        .with_field("Total", vec![Value::int(100)])
        .with_field("City", vec![Value::text("Paris")])
        .with_field(
            "Keys::Private",
            vec![Value::text(PRIVATE_KEY.replace('\n', "\r"))],
        )
        .with_field("Keys::Encrypted", vec![Value::text(ENCRYPTED_KEY)])
        .with_field("Keys::Public", vec![Value::text(PUBLIC_KEY)]);
    c.schema = Some(Arc::new(schema()));
    c.current_occurrence = Some("Customers".into());
    let d = |y, m, dd| Value::Date(crate::value::day_number(y, m, dd));
    c.tables.insert(
        "customers".into(),
        Rows {
            columns: ["Name", "City", "Age", "Joined"].map(String::from).to_vec(),
            rows: vec![
                vec![
                    Value::text("Ann"),
                    Value::text("Paris"),
                    Value::int(34),
                    d(2020, 1, 15),
                ],
                vec![
                    Value::text("bob"),
                    Value::text("Lyon"),
                    Value::int(25),
                    d(2021, 6, 1),
                ],
                vec![
                    Value::text("Cy"),
                    Value::text("Paris"),
                    Value::empty(),
                    d(2019, 3, 3),
                ],
            ],
        },
    );
    c.tables.insert(
        "orders".into(),
        Rows {
            columns: ["Customer", "Amount"].map(String::from).to_vec(),
            rows: [("Ann", 10), ("Ann", 5), ("bob", 7), ("Zed", 3)]
                .map(|(c, n)| vec![Value::text(c), Value::int(n)])
                .to_vec(),
        },
    );
    c.summaries.insert("total|total".into(), Value::int(100));
    c.summaries.insert("total|city".into(), Value::int(60));
    c.self_value = Some(Value::int(5));
    c.flags.insert("recordid".into(), Value::int(7));
    c.flags
        .insert("privilegesetname".into(), Value::text("[Full Access]"));
    c
}

fn check(cases: &[(&str, &str)]) {
    let mut failures = vec![];
    for (src, want) in cases {
        let got = eval_text(src, &mut ctx());
        if got != *want {
            failures.push(format!("{src}\n    want {want:?}\n    got  {got:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn design_lists() {
    check(&[
        ("BaseTableNames ( \"\" )", "Customers\rInvoices"),
        ("BaseTableIDs ( \"Contacts\" )", "129\r130"),
        ("TableNames ( \"\" )", "Customers\rInvoices"),
        ("TableIDs ( \"\" )", "1065089\r1065090"),
        ("LayoutNames ( \"\" )", "Data Entry"),
        ("LayoutIDs ( \"\" )", "1"),
        ("ScriptNames ( \"\" )", "Start\r-\rReport"),
        ("ScriptIDs ( \"\" )", "1\r2\r3"),
        ("ValueListNames ( \"\" )", "Yes/No List"),
        ("ValueListIDs ( \"\" )", "1"),
        ("ValueListItems ( \"\" ; \"yes/no list\" )", "Yes\rNo"),
        ("ValueListItems ( \"\" ; \"nope\" )", ""),
        (
            "BaseTableComment ( \"\" ; \"Customers\" )",
            "People we sell to",
        ),
    ]);
}

#[test]
fn design_fields() {
    check(&[
        // Documented FieldType examples
        (
            "FieldType ( \"\" ; \"Customers::Phone Number\" )",
            "Standard Text Unindexed 3",
        ),
        (
            "FieldType ( \"\" ; \"Current Balance\" )",
            "StoredCalc Number Indexed 1",
        ),
        (
            "FieldType ( \"\" ; \"Today's Date\" )",
            "Global Date Unindexed 1",
        ),
        (
            "FieldType ( \"\" ; \"Statement\" )",
            "External(Secure) Container Unindexed 1",
        ),
        ("FieldType ( \"\" ; \"Nope\" )", ""),
        (
            "FieldComment ( \"\" ; \"Customers::PrimaryKey\" )",
            "Unique identifier",
        ),
        (
            "FieldAnnotation ( \"\" ; \"Invoices::Total\" )",
            "Invoice total",
        ),
        // Documented FieldDisplayNames example
        (
            "FieldDisplayNames ( \"\" ; \"Customers::FirstName\" )",
            "{\"fm_common\":\"First Name\",\"fm_table_view\":\"Given Name\"}",
        ),
        (
            "JSONGetElement ( FieldDisplayNames ( \"\" ; \"Customers::FirstName\" ) ; \"fm_table_view\" )",
            "Given Name",
        ),
        ("FieldDisplayNames ( \"\" ; \"Name\" )", "{}"),
        (
            "GetNextSerialValue ( \"\" ; \"Customers::Serial\" )",
            "1005",
        ),
        ("GetNextSerialValue ( \"\" ; \"Name\" )", ""),
        ("GetBaseTableName ( Invoices::Total )", "Invoices"),
    ]);
}

#[test]
fn design_layouts() {
    check(&[
        (
            "FieldNames ( \"\" ; \"Data Entry\" )",
            "Name\rPhone Number\rCurrent Balance\rInvoices::Total",
        ),
        ("FieldIDs ( \"\" ; \"Data Entry\" )", "2\r3\r4\r1065090::3"),
        (
            "FieldNames ( \"\" ; \"Invoices\" )",
            "PrimaryKey\rCustomerID\rTotal\rNotes",
        ),
        ("FieldNames ( \"\" ; \"Nope\" )", ""),
        // Documented FieldBounds / FieldStyle examples
        (
            "FieldBounds ( \"\" ; \"Data Entry\" ; \"Name\" )",
            "36 48 295 65 0",
        ),
        (
            "FieldStyle ( \"\" ; \"Data Entry\" ; \"Current Balance\" )",
            "RadioButton Yes/No List",
        ),
        (
            "FieldStyle ( \"\" ; \"Data Entry\" ; \"Customers::Name\" )",
            "Standard",
        ),
        (
            "FieldRepetitions ( \"\" ; \"Data Entry\" ; \"Phone Number\" )",
            "3 vertical",
        ),
        (
            "FieldRepetitions ( \"\" ; \"Data Entry\" ; \"Name\" )",
            "1 horizontal",
        ),
        ("FieldBounds ( \"\" ; \"Data Entry\" ; \"Notes\" )", ""),
        (
            "LayoutObjectNames ( \"\" ; \"Data Entry\" )",
            "nameField\rinvoices\r<\rtotal\r>\rbtn",
        ),
        ("LayoutObjectNames ( \"\" ; \"\" )", ""),
        (
            "GetFieldsOnLayout ( \"Data Entry\" )",
            "{\"fields\" : {\"Customers::Current Balance\" : {\"type\" : \"number\"},\
             \"Customers::Name\" : {\"description\" : \"Full name\",\"type\" : \"string\"},\
             \"Customers::Phone Number\" : {\"type\" : \"string\"},\
             \"Invoices::Total\" : {\"description\" : \"Invoice total\",\"type\" : \"number\"}},\
             \"layout_name\" : \"Data Entry\"}",
        ),
    ]);
}

#[test]
fn relationships_and_ddl() {
    check(&[
        (
            "RelationInfo ( \"\" ; \"Customers\" )",
            "Source:Contacts\rTable:Invoices\rOptions:Create Sorted\rInvoices::CustomerID = Customers::PrimaryKey",
        ),
        (
            "RelationInfo ( \"\" ; \"Invoices\" )",
            "Source:Contacts\rTable:Customers\rOptions:\rCustomers::PrimaryKey = Invoices::CustomerID",
        ),
        ("RelationInfo ( \"\" ; \"Nope\" )", ""),
        // Only annotated fields when any field has an annotation, as in the
        // documented Topics example.
        (
            "GetTableDDL ( \"[\\\"Invoices\\\"]\" ; False )",
            "CREATE TABLE \"Invoices\" (\r\"PrimaryKey\" varchar(255), /*Unique ID*/\r\
             \"CustomerID\" varchar(255), /*Customer of the invoice*/\r\
             \"Total\" int, /*Invoice total*/\r\
             PRIMARY KEY (PrimaryKey)\r);\r\r",
        ),
        (
            "GetTableDDL ( \"[\\\"Invoices\\\", \\\"Nope\\\"]\" ; False )",
            "?",
        ),
        (
            "Length ( GetTableDDL ( \"[\\\"Nope\\\", \\\"Customers\\\"]\" ; True ) ) > 0",
            "1",
        ),
        (
            "Left ( GetTableDDL ( \"[\\\"Customers\\\"]\" ; True ) ; 74 )",
            "CREATE TABLE \"Customers\" (\r\"PrimaryKey\" varchar(255), /*Unique identifier*",
        ),
    ]);
}

#[test]
fn records() {
    check(&[
        ("GetNthRecord ( Name ; 2 )", "bob"),
        ("GetNthRecord ( Name ; 9 )", ""),
        ("GetNthRecord ( Lines::Price ; 2 )", "2"),
        ("GetNthRecord ( Name ; 0 )", "?"),
        ("Lookup ( Lines::Price )", "1.5"),
        ("Lookup ( Empty::Rel ; \"none\" )", "none"),
        ("Lookup ( Empty::Rel )", ""),
        ("LookupNext ( Lines::Price ; Higher )", "?"),
        (
            "EvaluationError ( LookupNext ( Lines::Price ; Lower ) )",
            "3",
        ),
        ("GetSummary ( Total ; Total )", "100"),
        ("GetSummary ( Total ; City )", "60"),
        ("Self * 2", "10"),
        ("Status ( CurrentRecordID )", "7"),
        ("Status ( CurrentGroups )", "[Full Access]"),
        ("Status ( CurrentNonsense )", "?"),
        // Documented GetRecordIDsFromFoundSet formats
        ("GetRecordIDsFromFoundSet ( ValueNumber )", "1\r2\r3"),
        (
            "GetRecordIDsFromFoundSet ( JSONString )",
            "[\"1\",\"2\",\"3\"]",
        ),
        ("GetRecordIDsFromFoundSet ( JSONNumber )", "[1,2,3]"),
        ("GetRecordIDsFromFoundSet ( ValueNumberRanges )", "1-3"),
        (
            "GetRecordIDsFromFoundSet ( JSONStringRanges ; \"Customers\" )",
            "[\"1-3\"]",
        ),
    ]);
}

#[test]
fn execute_sql() {
    check(&[
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE City = ? ORDER BY Name\" ; \"\" ; \"\" ; \"Paris\" )",
            "Ann\rCy",
        ),
        (
            "ExecuteSQL ( \"SELECT Name, Age FROM Customers ORDER BY Age DESC\" ; \"|\" ; \";\" )",
            "Ann|34;bob|25;Cy|",
        ),
        (
            "ExecuteSQL ( \"select count(*) from Customers where Age > 30\" ; \"\" ; \"\" )",
            "1",
        ),
        (
            "ExecuteSQL ( \"SELECT Joined FROM Customers WHERE Name = 'bob'\" ; \"\" ; \"\" )",
            "2021-06-01",
        ),
        // Text comparisons are case-sensitive
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE Name LIKE 'b%'\" ; \"\" ; \"\" )",
            "bob",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE Name LIKE 'B%'\" ; \"\" ; \"\" )",
            "",
        ),
        (
            "ExecuteSQL ( \"SELECT \\\"Name\\\" FROM \\\"Customers\\\" WHERE Age IS NULL\" ; \"\" ; \"\" )",
            "Cy",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE Joined > DATE '2020-01-01' ORDER BY Joined\" ; \"\" ; \"\" )",
            "Ann\rbob",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers c WHERE c.Joined < ?\" ; \"\" ; \"\" ; Date ( 1 ; 1 ; 2020 ) )",
            "Cy",
        ),
        (
            "ExecuteSQL ( \"SELECT DISTINCT City FROM Customers ORDER BY City\" ; \"\" ; \"\" )",
            "Lyon\rParis",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE City IN ('Lyon', 'Nice') OR Age BETWEEN 30 AND 40 ORDER BY 1\" ; \"\" ; \"\" )",
            "Ann\rbob",
        ),
        (
            "ExecuteSQL ( \"SELECT * FROM Customers ORDER BY Name OFFSET 1 ROWS FETCH FIRST 1 ROWS ONLY\" ; \"\" ; \"\" )",
            "Cy,Paris,,2019-03-03",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE NOT (City = 'Paris')\" ; \"\" ; \"\" )",
            "bob",
        ),
        (
            "ExecuteSQL ( \"SELECT Nope FROM Customers\" ; \"\" ; \"\" )",
            "?",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Missing\" ; \"\" ; \"\" )",
            "?",
        ),
        (
            "ExecuteSQL ( \"SELECT Name FROM Customers WHERE Name = ?\" ; \"\" ; \"\" )",
            "?",
        ),
        (
            "ExecuteSQL ( \"DELETE FROM Customers\" ; \"\" ; \"\" )",
            "?",
        ),
        (
            "ExecuteSQLe ( \"SELECT Nope FROM Customers\" ; \"\" ; \"\" )",
            "? column Nope does not exist",
        ),
    ]);
}

#[test]
fn sql_joins_and_groups() {
    let q = |sql: &str| format!("ExecuteSQL ( \"{sql}\" ; \"\" ; \"\" )");
    let cases = [
        (
            "SELECT c.Name, o.Amount FROM Customers c JOIN Orders o ON c.Name = o.Customer ORDER BY o.Amount",
            "Ann,5\rbob,7\rAnn,10",
        ),
        (
            "SELECT c.Name, o.Amount FROM Customers c LEFT OUTER JOIN Orders o ON c.Name = o.Customer ORDER BY c.Name, 2",
            "Ann,5\rAnn,10\rCy,\rbob,7",
        ),
        (
            "SELECT Customers.Name FROM Customers, Orders WHERE Name = Customer AND Amount > 6",
            "Ann\rbob",
        ),
        (
            "SELECT o.* FROM Customers c INNER JOIN Orders o ON o.Customer = c.Name WHERE c.City = 'Lyon'",
            "bob,7",
        ),
        (
            "SELECT Customer, COUNT(*), SUM(Amount) FROM Orders GROUP BY Customer ORDER BY SUM(Amount) DESC",
            "Ann,2,15\rbob,1,7\rZed,1,3",
        ),
        (
            "SELECT Customer FROM Orders GROUP BY Customer HAVING COUNT(*) > 1",
            "Ann",
        ),
        (
            "SELECT Customer, SUM(Amount) AS total FROM Orders GROUP BY Customer ORDER BY total",
            "Zed,3\rbob,7\rAnn,15",
        ),
        (
            "SELECT AVG(Amount), MIN(Amount), MAX(Amount), COUNT(DISTINCT Customer) FROM Orders",
            "6.25,3,10,3",
        ),
        (
            "SELECT SUM(Amount), COUNT(Amount) FROM Orders WHERE Amount > 100",
            ",0",
        ),
        (
            "SELECT c.City, SUM(o.Amount) FROM Customers c JOIN Orders o ON o.Customer = c.Name GROUP BY c.City ORDER BY 2",
            "Lyon,7\rParis,15",
        ),
        // FileMaker's "?" for SQL errors.
        ("SELECT Customer, COUNT(*) FROM Orders", "?"),
        ("SELECT Name FROM Customers JOIN Orders", "?"),
        ("SELECT x.* FROM Orders", "?"),
    ];
    let cases: Vec<(String, &str)> = cases.iter().map(|(s, w)| (q(s), *w)).collect();
    let cases: Vec<(&str, &str)> = cases.iter().map(|(s, w)| (s.as_str(), *w)).collect();
    check(&cases);
}

#[test]
fn json_parse_and_values() {
    check(&[
        ("JSONParse ( \"[1,2]\" )", "[1,2]"),
        ("Left ( JSONParse ( \"[3\" ) ; 1 )", "?"),
        // Documented JSONParsedState examples
        (
            "Let ( $a = JSONParse ( \"[3\" ) ; JSONParsedState ( $a ) )",
            "-1",
        ),
        ("Let ( $a = \"[3]\" ; JSONParsedState ( $a ) )", "0"),
        ("JSONParsedState ( JSONParse ( Pi ) )", "-1"),
        (
            "Let ( $a = JSONSetElement ( \"\" ; \"a\" ; \"b\" ; JSONString ) ; JSONParsedState ( $a ) )",
            "3",
        ),
        (
            "Let ( $a = JSONParse ( \"[3]\" ) ; JSONParsedState ( $a ) )",
            "4",
        ),
        ("JSONParsedState ( JSONParse ( \"\" ) )", "-1"),
        ("JSONParse ( \"[3]\" ) = \"[3]\"", "1"),
        (
            "JSONGetElement ( JSONParse ( \"{\\\"a\\\":5}\" ) ; \"a\" ) + 1",
            "6",
        ),
        (
            "JSONValueExists ( \"{\\\"a\\\":[1,2,\\\"x\\\"]}\" ; \"a\" ; 2 )",
            "1",
        ),
        (
            "JSONValueExists ( \"{\\\"a\\\":[1,2,\\\"x\\\"]}\" ; \"a\" ; \"x\" )",
            "1",
        ),
        (
            "JSONValueExists ( \"{\\\"a\\\":[1,2,\\\"x\\\"]}\" ; \"a\" ; 3 )",
            "0",
        ),
        (
            "JSONValueExists ( \"{\\\"a\\\":{\\\"k\\\":2.0}}\" ; \"a\" ; 2 )",
            "0",
        ),
        ("JSONValueCount ( \"[1,1,2]\" ; \"\" ; 1 )", "2"),
        ("JSONAnyValuesExist ( \"[1,2]\" ; \"\" ; \"[5,2]\" )", "1"),
        ("JSONAnyValuesExist ( \"[1,2]\" ; \"\" ; \"[5,6]\" )", "0"),
        (
            "JSONRemoveValues ( \"{\\\"a\\\":[1,2,3,2]}\" ; \"a\" ; \"[2]\" )",
            "{\"a\":[1,3]}",
        ),
        (
            "JSONUniqueValues ( \"[3,1,3,\\\"a\\\",\\\"a\\\"]\" ; \"\" )",
            "[3,1,\"a\"]",
        ),
        (
            "JSONUniqueValues ( \"{\\\"a\\\":[1,1]}\" ; \"b\" )",
            "{\"a\":[1,1]}",
        ),
        ("Left ( JSONUniqueValues ( \"[1\" ; \"\" ) ; 1 )", "?"),
    ]);
}

#[test]
fn japanese() {
    check(&[
        ("KanaHankaku ( \"ガッコウ、パン\" )", "ｶﾞｯｺｳ、ﾊﾟﾝ"),
        ("KanaZenkaku ( \"ｶﾞｯｺｳ ﾊﾟﾝ abc\" )", "ガッコウ パン abc"),
        ("Hiragana ( \"ｶﾀｶﾅ\" )", "かたかな"),
        ("Hiragana ( \"カタカナ\" )", "かたかな"),
        ("Katakana ( \"ひらがな\" )", "ヒラガナ"),
        ("KanjiNumeral ( 123 )", "一二三"),
        ("KanjiNumeral ( \"第5回 ２０\" )", "第五回 二〇"),
        ("NumToJText ( 123456789 ; 0 ; 0 )", "123456789"),
        ("NumToJText ( 123456789 ; 1 ; 0 )", "123,456,789"),
        ("NumToJText ( 123456789 ; 2 ; 0 )", "1億2345万6789"),
        (
            "NumToJText ( 123456789 ; 3 ; 2 )",
            "一億二千三百四十五万六千七百八十九",
        ),
        ("NumToJText ( 123456789 ; 0 ; 1 )", "１２３４５６７８９"),
        ("NumToJText ( 1200 ; 3 ; 3 )", "壱阡弐陌"),
        ("NumToJText ( 100000005 ; 2 ; 0 )", "1億5"),
        ("NumToJText ( 0 ; 3 ; 2 )", "〇"),
        ("NumToJText ( 12 ; 9 ; \"\" )", "12"),
        ("YearName ( Date ( 7 ; 15 ; 2008 ) ; 0 )", "平成20"),
        ("YearName ( Date ( 5 ; 1 ; 2019 ) ; 1 )", "(令)元"),
        ("YearName ( Date ( 1 ; 7 ; 1989 ) ; 2 )", "S64"),
        ("YearName ( Date ( 1 ; 8 ; 1989 ) ; 2 )", "H1"),
        ("YearName ( Date ( 1 ; 1 ; 1850 ) ; 0 )", "西暦1850"),
        ("YearName ( \"\" ; 0 )", ""),
        ("MonthNameJ ( Date ( 6 ; 6 ; 2019 ) )", "6月"),
    ]);
}

#[test]
fn styled_text_as_markup() {
    check(&[
        (
            "GetAsCSS ( \"a<b¶c & \\\"d\\\"\" )",
            "<span style=\"\" >a&lt;b<br/>c &amp; &quot;d&quot;</span>",
        ),
        (
            "GetAsSVG ( \"Frank\" )",
            "<stylelist>\r<style#0>\"\", begin: 1, end: 5</style> \r</stylelist>\r<data>\r<span style=\"0\">Frank</span>\r</data>",
        ),
        ("GetAsSVG ( \"\" )", ""),
    ]);
}

#[test]
fn encryption() {
    check(&[
        // Documented CryptEncrypt example, round-tripped
        (
            "CryptDecrypt ( CryptEncrypt ( \"This needs protection\" ; \"My secret password\" ) ; \"My secret password\" )",
            "This needs protection",
        ),
        ("CryptEncrypt ( \"x\" ; \"k\" )", "encrypted.data"),
        (
            "CryptDecrypt ( CryptEncrypt ( \"x\" ; \"k\" ) ; \"wrong\" )",
            "?",
        ),
        (
            "CryptDecryptBase64 ( CryptEncryptBase64 ( \"abc\" ; \"k\" ) ; \"k\" )",
            "abc",
        ),
        (
            "CryptDecrypt ( CryptEncrypt ( TextEncode ( \"f\" ; \"utf-8\" ; 1 ) ; \"k\" ) ; \"k\" )",
            "utf-8.txt",
        ),
        (
            "CryptEncryptBase64 ( \"a\" ; \"k\" ) = CryptEncryptBase64 ( \"a\" ; \"k\" )",
            "0",
        ),
        ("CryptDecryptBase64 ( \"AAAA\" ; \"k\" )", "?"),
    ]);
}

#[test]
fn signatures() {
    check(&[
        // Deterministic PKCS #1 v1.5: identical to OpenSSL's signature.
        (
            "HexEncode ( CryptGenerateSignature ( \"hello\" ; \"SHA256\" ; Keys::Private ; \"\" ) )",
            HELLO_SHA256_SIG,
        ),
        (
            "HexEncode ( CryptGenerateSignature ( \"hello\" ; \"sha256\" ; Keys::Encrypted ; \"secret\" ) )",
            HELLO_SHA256_SIG,
        ),
        (
            "CryptGenerateSignature ( \"hello\" ; \"SHA256\" ; Keys::Encrypted ; \"bad\" )",
            "?",
        ),
        (
            "CryptGenerateSignature ( \"hello\" ; \"SHA999\" ; Keys::Private ; \"\" )",
            "?",
        ),
        (
            "CryptGenerateSignature ( \"hello\" ; \"SHA256\" ; \"junk\" ; \"\" )",
            "?",
        ),
        (
            "CryptVerifySignature ( \"hello\" ; \"SHA512\" ; Keys::Public ; CryptGenerateSignature ( \"hello\" ; \"SHA512\" ; Keys::Private ; \"\" ) )",
            "1",
        ),
        (
            "CryptVerifySignature ( \"hellO\" ; \"SHA512\" ; Keys::Public ; CryptGenerateSignature ( \"hello\" ; \"SHA512\" ; Keys::Private ; \"\" ) )",
            "0",
        ),
    ]);
    // OpenSSL's signature verifies.
    let src = format!(
        "CryptVerifySignature ( \"hello\" ; \"SHA256\" ; Keys::Public ; HexDecode ( \"{HELLO_SHA256_SIG}\" ; \"s\" ) )"
    );
    assert_eq!(eval_text(&src, &mut ctx()), "1");
}

#[test]
fn embeddings() {
    check(&[
        ("CosineSimilarity ( \"[1,0]\" ; \"[0,1]\" )", "0"),
        ("CosineSimilarity ( \"[1,2]\" ; \"[2,4]\" )", "1"),
        ("CosineSimilarity ( \"[1,2]\" ; \"[1]\" )", "?"),
        ("AddEmbeddings ( \"[1,2]\" ; \"[3,4]\" )", "[4,6]"),
        ("SubtractEmbeddings ( \"[1,2]\" ; \"[3,4]\" )", "[-2,-2]"),
        ("NormalizeEmbedding ( \"[3,4]\" )", "[0.6,0.8]"),
        ("NormalizeEmbedding ( \"[3,4,12]\" ; 2 )", "[0.6,0.8]"),
        (
            "GetEmbeddingAsText ( GetEmbeddingAsFile ( \"[1.5,2]\" ) )",
            "[1.5,2]",
        ),
        ("GetEmbeddingAsFile ( \"[1]\" ; \"v.bin\" )", "v.bin"),
        (
            "GetEmbeddingAsText ( AddEmbeddings ( GetEmbeddingAsFile ( \"[1]\" ) ; \"[2]\" ) )",
            "[3]",
        ),
        ("GetEmbeddingAsText ( \"abc\" )", "?"),
    ]);
}

#[test]
fn containers_and_paths() {
    check(&[
        (
            "GetContainerAttribute ( CryptEncrypt ( \"a\" ; \"k\" ) ; \"filename\" )",
            "encrypted.data",
        ),
        (
            "GetContainerAttribute ( TextEncode ( \"abc\" ; \"utf-8\" ; 1 ) ; \"fileSize\" )",
            "3",
        ),
        (
            "GetContainerAttribute ( TextEncode ( \"abc\" ; \"utf-8\" ; 1 ) ; \"MD5\" )",
            "900150983CD24FB0D6963F7D28E17F72",
        ),
        (
            "GetWidth ( HexDecode ( \"474946383961\" & \"0A000500\" ; \"a.gif\" ) )",
            "10",
        ),
        (
            "GetHeight ( HexDecode ( \"474946383961\" & \"0A000500\" ; \"a.gif\" ) )",
            "5",
        ),
        ("GetWidth ( TextEncode ( \"abc\" ; \"utf-8\" ; 1 ) )", "?"),
        ("GetWidth ( \"\" )", ""),
        (
            "VerifyContainer ( TextEncode ( \"abc\" ; \"utf-8\" ; 1 ) )",
            "1",
        ),
        ("VerifyContainer ( \"\" )", ""),
        (
            "ConvertFromFileMakerPath ( \"filewin:/C:/My Files/a.txt\" ; WinPath )",
            "C:\\My Files\\a.txt",
        ),
        (
            "ConvertFromFileMakerPath ( \"filewin://server/share/a.txt\" ; WinPath )",
            "\\\\server\\share\\a.txt",
        ),
        (
            "ConvertFromFileMakerPath ( \"filewin:/C:/My Files/a.txt\" ; URLPath )",
            "file:///C:/My%20Files/a.txt",
        ),
        (
            "ConvertFromFileMakerPath ( \"filemac:/Macintosh HD/Users/a.txt\" ; PosixPath )",
            "/Volumes/Macintosh HD/Users/a.txt",
        ),
        ("ConvertFromFileMakerPath ( \"C:/x\" ; WinPath )", ""),
        (
            "ConvertToFileMakerPath ( \"C:\\Temp\\x.txt\" ; WinPath )",
            "file:/C:/Temp/x.txt",
        ),
        (
            "ConvertToFileMakerPath ( \"\\\\\\\\srv\\share\\x\" ; WinPath )",
            "file://srv/share/x",
        ),
        (
            "ConvertToFileMakerPath ( \"/Volumes/Data/x.txt\" ; PosixPath )",
            "filemac:/Data/x.txt",
        ),
        (
            "ConvertToFileMakerPath ( \"file:///C:/My%20Files/a.txt\" ; URLPath )",
            "file:/C:/My Files/a.txt",
        ),
    ]);
}

/// A host that knows where it is.
struct Gps(MemoryContext);

impl Context for Gps {
    fn variable(&mut self, name: &str, repetition: usize) -> Value {
        self.0.variable(name, repetition)
    }
    fn set_variable(&mut self, name: &str, repetition: usize, value: Value) {
        self.0.set_variable(name, repetition, value)
    }
    fn host_function(&mut self, name: &str, args: &[Value]) -> Option<Result<Value, EvalError>> {
        (name == "Location").then(|| Ok(Value::text(format!("48.85,2.35 ±{}", args[0].to_text()))))
    }
}

#[test]
fn platform_functions() {
    check(&[
        ("Location ( 10 )", ""),
        ("EvaluationError ( GetSensor ( \"battery\" ) )", "3"),
        (
            "EvaluationError ( GetEmbedding ( \"a\" ; \"m\" ; \"t\" ) )",
            "3",
        ),
        ("EvaluationError ( Furigana ( \"漢字\" ) )", "0"),
    ]);
    let mut host = Gps(MemoryContext::default());
    assert_eq!(eval_text("Location ( 10 )", &mut host), "48.85,2.35 ±10");
    assert_eq!(eval_text("LocationValues ( 10 )", &mut host), "");
    // Without a schema the design functions answer empty text (FileMaker
    // does so for a file that is not open); records say "not available".
    assert_eq!(eval_text("TableNames ( \"\" )", &mut host), "");
    assert_eq!(
        eval_text("EvaluationError ( GetNthRecord ( x ; 1 ) )", &mut host),
        "3"
    );
}
