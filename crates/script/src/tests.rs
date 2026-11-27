//! Headless tests: a small database built in code, scripts built from
//! steps, and a test host that records dialogs and answers them.

use super::*;
use fmp12::scripts::{DialogButton, FindRequest};
use std::collections::VecDeque;

const T: u32 = 130; // table and occurrence "People"
const NAME: u32 = 1;
const AGE: u32 = 2;
const CITY: u32 = 3;

/// Assembles a calculation token stream from space-separated tokens:
/// `$var`, integers, `"text"`, `Get(Flag)`, `People::Name`, operators.
fn c(src: &str) -> Calc {
    let xor = |s: &str| s.bytes().map(|b| b ^ 0x5A).collect::<Vec<u8>>();
    let mut out = Vec::new();
    // Split on spaces, keeping quoted text whole.
    let mut toks: Vec<String> = Vec::new();
    let mut quoted = false;
    for ch in src.chars() {
        match ch {
            ' ' if !quoted => toks.push(String::new()),
            _ => {
                if toks.is_empty() {
                    toks.push(String::new());
                }
                quoted ^= ch == '"';
                toks.last_mut().unwrap().push(ch);
            }
        }
    }
    for tok in toks.iter().map(String::as_str).filter(|t| !t.is_empty()) {
        if let Some(name) = tok.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
            out.extend([0x13, name.len() as u8]);
            out.extend(xor(name));
            out.push(0);
        } else if tok.starts_with('$') {
            out.extend([0x1A, tok.len() as u8]);
            out.extend(xor(tok));
        } else if let Ok(n) = tok.parse::<u32>() {
            out.extend([0x10, 2, 0, 1, 0, 0x10]);
            out.extend(n.to_be_bytes());
            out.extend([0; 8]);
            out.extend([0, 0x20]);
        } else if let Some(flag) = tok.strip_prefix("Get(").and_then(|t| t.strip_suffix(')')) {
            let f = match flag {
                "LastError" => 0x03,
                "FoundCount" => 0x07,
                "RecordNumber" => 0x14,
                "LastMessageChoice" => 0x21,
                "ScriptParameter" => 0x3E,
                "ScriptResult" => 0x50,
                _ => panic!("flag {flag}"),
            };
            out.extend([0x9B, 0x04, 0x9C, f, 0x05]);
        } else if let Some(f) = tok.strip_prefix("People::") {
            let id = [("Name", NAME), ("Age", AGE), ("City", CITY)]
                .iter()
                .find(|(n, _)| *n == f)
                .unwrap()
                .1;
            out.extend([
                0x16,
                0x04,
                0x04,
                0x03,
                0xD0,
                0x00,
                (T - 128) as u8,
                0x02,
                0x01,
                id as u8,
                0x00,
                0x01,
            ]);
        } else {
            let op = match tok {
                "+" => 0x25,
                "-" => 0x26,
                "*" => 0x27,
                "<" => 0x41,
                "=" => 0x44,
                "≠" => 0x46,
                ">=" => 0x47,
                ">" => 0x49,
                "&" => 0x50,
                "and" => 0x1F,
                "or" => 0x20,
                _ => panic!("token {tok}"),
            };
            out.push(op);
        }
    }
    Calc(out)
}

fn step(id: u16, params: StepParams) -> Step {
    Step {
        id,
        enabled: true,
        params,
        ..Default::default()
    }
}

fn var(name: &str, value: &str) -> Step {
    step(
        id::SET_VARIABLE,
        StepParams::SetVariable {
            name: name.into(),
            value: Some(c(value)),
            repetition: None,
        },
    )
}

fn cond(id: u16, src: &str) -> Step {
    step(id, StepParams::Condition(Some(c(src))))
}

fn exit(src: &str) -> Step {
    step(id::EXIT_SCRIPT, StepParams::ExitScript(Some(c(src))))
}

fn plain(id: u16) -> Step {
    step(id, StepParams::None)
}

fn field(f: u32) -> FieldRef {
    FieldRef {
        to: T,
        field: f,
        repetition: 1,
    }
}

fn set_field(f: u32, src: &str) -> Step {
    step(
        id::SET_FIELD,
        StepParams::SetField {
            field: Some(field(f)),
            value: Some(c(src)),
            repetition: None,
        },
    )
}

fn perform(script: u32, param: Option<&str>) -> Step {
    step(
        id::PERFORM_SCRIPT,
        StepParams::PerformScript {
            script: Some(script),
            parameter: param.map(c),
        },
    )
}

fn script(id: u32, steps: Vec<Step>) -> Script {
    Script {
        id,
        name: format!("S{id}"),
        options: 8,
        steps,
    }
}

fn db(people: &[(&str, &str, &str)]) -> Database {
    use fmp12::schema::Field as Def;
    let def = |dt: u8| Def {
        options: vec![1, dt],
        ..Default::default()
    };
    let f = |id: u32, name: &str, dt: u8| engine::Field {
        id,
        name: name.into(),
        kind: def(dt).kind(),
        data_type: def(dt).data_type(),
        calc: None,
        auto_calc: None,
        context: 0,
        def: Def {
            name: name.into(),
            ..def(dt)
        },
    };
    let mut t = engine::Table::new(
        T,
        "People",
        vec![f(NAME, "Name", 1), f(AGE, "Age", 2), f(CITY, "City", 1)],
    );
    for (n, a, ci) in people {
        let r = t.new_record();
        t.records[r].values = [(NAME, n), (AGE, a), (CITY, ci)]
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
    }
    let layout = |id: u32, name: &str| fmp12::layouts::Layout {
        id,
        name: name.into(),
        table_occurrence: 0x104000 | T as u64,
        ..Default::default()
    };
    Database {
        tables: vec![t],
        occurrences: vec![fmp12::graph::TableOccurrence {
            id: T,
            name: "People".into(),
            base_table: T,
            ..Default::default()
        }],
        relationships: vec![],
        layouts: vec![layout(1, "People"), layout(2, "People List")],
        themes: vec![],
        accounts: vec![],
        privilege_sets: vec![],
        value_lists: vec![fmp12::valuelists::ValueList {
            id: 1,
            name: "Cities".into(),
            source: fmp12::valuelists::Source::Custom("Rome\rParis\rOslo".into()),
            raw: Vec::new(),
        }],
        locale: Default::default(),
    }
}

struct TestHost {
    db: Database,
    win: Window,
    answers: VecDeque<DialogAnswer>,
    dialogs: Vec<Dialog>,
    errors: Vec<u32>,
}

impl TestHost {
    fn new(people: &[(&str, &str, &str)]) -> TestHost {
        let db = db(people);
        let win = Window::new(&db, 1);
        TestHost {
            db,
            win,
            answers: VecDeque::new(),
            dialogs: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Names of the found set, in order.
    fn names(&self) -> Vec<String> {
        self.win
            .found
            .iter()
            .map(|&r| self.db.tables[0].records[r].values[&NAME].clone())
            .collect()
    }
}

impl Host for TestHost {
    fn db(&mut self) -> &mut Database {
        &mut self.db
    }
    fn window(&mut self) -> &mut Window {
        &mut self.win
    }
    fn dialog(&mut self, d: &Dialog) -> DialogAnswer {
        self.dialogs.push(d.clone());
        self.answers.pop_front().unwrap_or(DialogAnswer {
            button: 1,
            inputs: d.inputs.iter().map(|i| i.2.clone()).collect(),
        })
    }
    fn error(&mut self, _: &Step, code: u32) -> bool {
        self.errors.push(code);
        true
    }
}

const PEOPLE: &[(&str, &str, &str)] = &[
    ("Ann", "34", "Paris"),
    ("Bob", "27", "Oslo"),
    ("Cid", "41", "Rome"),
    ("Dee", "27", "Paris"),
];

/// Runs script 1 of `scripts` with a parameter; returns (result text, host).
fn run(scripts: Vec<Script>, param: &str) -> (String, Outcome, TestHost) {
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(scripts);
    let o = e.perform(&mut h, 1, Some(Value::text(param)));
    (o.result.to_text(), o, h)
}

#[test]
fn control_flow_table() {
    let if_chain = vec![
        cond(id::IF, "Get(ScriptParameter) = 1"),
        var("$r", "\"one\""),
        cond(id::ELSE_IF, "Get(ScriptParameter) = 2"),
        var("$r", "\"two\""),
        plain(id::ELSE),
        var("$r", "\"other\""),
        plain(id::END_IF),
        exit("$r"),
    ];
    let nested = vec![
        var("$n", "0"),
        var("$i", "0"),
        plain(id::LOOP),
        var("$i", "$i + 1"),
        cond(id::EXIT_LOOP_IF, "$i > 4"),
        cond(id::IF, "$i = 2 or $i = 4"),
        var("$n", "$n + 10"),
        plain(id::ELSE),
        var("$n", "$n + 1"),
        plain(id::END_IF),
        plain(id::END_LOOP),
        exit("$n"),
    ];
    let count = vec![
        var("$i", "0"),
        plain(id::LOOP),
        var("$i", "$i + 1"),
        cond(id::EXIT_LOOP_IF, "$i >= Get(ScriptParameter)"),
        plain(id::END_LOOP),
        exit("$i"),
    ];
    let disabled = vec![
        var("$x", "1"),
        Step {
            enabled: false,
            ..var("$x", "2")
        },
        exit("$x"),
    ];
    let cases: &[(&str, &Vec<Step>, &str, &str)] = &[
        ("if branch", &if_chain, "1", "one"),
        ("else if branch", &if_chain, "2", "two"),
        ("else branch", &if_chain, "7", "other"),
        ("loop with nested if", &nested, "", "22"),
        ("loop count", &count, "5", "5"),
        ("loop runs at least once", &count, "0", "1"),
        ("disabled step skipped", &disabled, "", "1"),
    ];
    for (name, steps, param, want) in cases {
        let (got, o, _) = run(vec![script(1, steps.to_vec())], param);
        assert_eq!(got, *want, "{name}");
        assert!(!o.halted, "{name}");
    }
}

#[test]
fn variables_scope_and_repetitions() {
    let main = vec![
        var("$local", "\"main\""),
        var("$$global", "\"set by main\""),
        perform(2, None),
        exit("Get(ScriptResult) & \"|\" & $local & \"|\" & $$GLOBAL & \"|\" & $x"),
    ];
    let sub = vec![
        // $local of the caller is not visible here; $$global is.
        var("$$global", "$$global & \"+sub\""),
        step(
            id::SET_VARIABLE,
            StepParams::SetVariable {
                name: "$x".into(),
                value: Some(c("\"rep2\"")),
                repetition: Some(c("2")),
            },
        ),
        exit("\"[\" & $local & \"]\""),
    ];
    let (got, _, _) = run(vec![script(1, main), script(2, sub)], "");
    assert_eq!(got, "[]|main|set by main+sub|");
}

#[test]
fn perform_script_parameter_and_result() {
    let main = vec![
        perform(2, Some("Get(ScriptParameter) * 2")),
        perform(2, Some("Get(ScriptResult) + 1")),
        exit("Get(ScriptResult)"),
    ];
    let double = vec![exit("Get(ScriptParameter) * 3")];
    let (got, o, _) = run(vec![script(1, main), script(2, double)], "5");
    // (5 * 2) * 3 = 30, then (30 + 1) * 3 = 93
    assert_eq!(got, "93");
    assert_eq!(o.last_error, 0);
    // Missing script: error 104.
    let (_, o, h) = run(vec![script(1, vec![perform(9, None)])], "");
    assert_eq!((o.last_error, h.errors.as_slice()), (104, &[104][..]));
}

#[test]
fn halt_stops_every_level() {
    let main = vec![perform(2, None), var("$$after", "1"), exit("\"no\"")];
    let sub = vec![plain(id::HALT_SCRIPT), exit("\"sub\"")];
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(vec![script(1, main), script(2, sub)]);
    let o = e.perform(&mut h, 1, None);
    assert!(o.halted);
    assert!(e.globals.is_empty());
}

#[test]
fn set_field_commit_and_revert() {
    let add = vec![
        plain(id::NEW_RECORD),
        set_field(NAME, "\"Eve\""),
        set_field(AGE, "Get(ScriptParameter) + 1"),
        plain(id::COMMIT),
        exit("Get(FoundCount) & \"/\" & Get(RecordNumber) & \"/\" & People::Name"),
    ];
    let (got, _, h) = run(vec![script(1, add)], "29");
    assert_eq!(got, "5/5/Eve");
    let r = &h.db.tables[0].records[4];
    assert_eq!(
        (r.values[&NAME].as_str(), r.values[&AGE].as_str()),
        ("Eve", "30")
    );

    // Revert a new record: it goes away. Revert an edit: old value back.
    let revert = vec![
        plain(id::NEW_RECORD),
        set_field(NAME, "\"Tmp\""),
        plain(id::REVERT_RECORD),
        step(
            id::GO_TO_RECORD,
            StepParams::GoToRecord {
                dest: RecordDest::First,
                exit_after_last: false,
                dialog: false,
            },
        ),
        set_field(NAME, "\"Changed\""),
        plain(id::REVERT_RECORD),
        exit("Get(FoundCount) & \"/\" & People::Name"),
    ];
    let (got, _, h) = run(vec![script(1, revert)], "");
    assert_eq!(got, "4/Ann");
    assert_eq!(h.db.tables[0].records.len(), 4);
}

fn find_step(id: u16, requests: Vec<FindRequest>) -> Step {
    step(
        id,
        StepParams::Find {
            pause: false,
            restore: true,
            requests,
        },
    )
}

fn request(omit: bool, criteria: &[(u32, &str)]) -> FindRequest {
    FindRequest {
        omit,
        criteria: criteria
            .iter()
            .map(|(f, t)| (field(*f), t.to_string()))
            .collect(),
    }
}

#[test]
fn perform_find_table() {
    let cases: &[(&str, Vec<FindRequest>, &[&str], u32)] = &[
        (
            "one criterion",
            vec![request(false, &[(CITY, "Paris")])],
            &["Ann", "Dee"],
            0,
        ),
        (
            "and within a request",
            vec![request(false, &[(CITY, "Paris"), (AGE, "27")])],
            &["Dee"],
            0,
        ),
        (
            "or across requests",
            vec![
                request(false, &[(NAME, "a*")]),
                request(false, &[(AGE, ">40")]),
            ],
            &["Ann", "Cid"],
            0,
        ),
        (
            "omit request",
            vec![
                request(false, &[(AGE, "27")]),
                request(true, &[(CITY, "Oslo")]),
            ],
            &["Dee"],
            0,
        ),
        ("no match", vec![request(false, &[(NAME, "Zed")])], &[], 401),
        (
            "no criteria",
            vec![request(false, &[])],
            &["Ann", "Bob", "Cid", "Dee"],
            400,
        ),
    ];
    for (name, reqs, want, err) in cases {
        let steps = vec![
            step(id::SET_ERROR_CAPTURE, StepParams::Switch(true)),
            find_step(id::PERFORM_FIND, reqs.clone()),
            exit("Get(LastError)"),
        ];
        let (got, _, h) = run(vec![script(1, steps)], "");
        assert_eq!(h.names(), *want, "{name}");
        assert_eq!(got, err.to_string(), "{name}");
        assert!(h.errors.is_empty(), "{name}: error capture is on");
    }
}

#[test]
fn find_mode_requests_from_set_field() {
    let steps = vec![
        find_step(id::ENTER_FIND_MODE, vec![]),
        set_field(CITY, "\"Paris\""),
        plain(id::NEW_RECORD),
        set_field(NAME, "\"Bob\""),
        find_step(id::PERFORM_FIND, vec![]),
    ];
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(vec![script(
        1,
        steps
            .into_iter()
            .map(|mut s| {
                // Perform Find without stored requests uses the window's.
                if let StepParams::Find { restore, .. } = &mut s.params {
                    *restore = s.id == id::ENTER_FIND_MODE;
                }
                s
            })
            .collect(),
    )]);
    e.perform(&mut h, 1, None);
    assert_eq!(h.win.mode, Mode::Browse);
    assert_eq!(h.names(), ["Ann", "Bob", "Dee"]);
}

#[test]
fn sort_omit_and_show_all() {
    let sort = |keys: Vec<(u32, SortOrder)>| {
        step(
            id::SORT,
            StepParams::Sort {
                dialog: false,
                restore: true,
                keys: keys.into_iter().map(|(f, o)| (field(f), o)).collect(),
            },
        )
    };
    let cases: &[(&str, Vec<Step>, &[&str])] = &[
        (
            "by name descending",
            vec![sort(vec![(NAME, SortOrder::Descending)])],
            &["Dee", "Cid", "Bob", "Ann"],
        ),
        (
            "age ascending then name descending",
            vec![sort(vec![
                (AGE, SortOrder::Ascending),
                (NAME, SortOrder::Descending),
            ])],
            &["Dee", "Bob", "Ann", "Cid"],
        ),
        (
            "custom order by value list",
            vec![sort(vec![
                (CITY, SortOrder::Custom(1)),
                (NAME, SortOrder::Ascending),
            ])],
            &["Cid", "Ann", "Dee", "Bob"],
        ),
        (
            "unsort",
            vec![sort(vec![(NAME, SortOrder::Descending)]), plain(id::UNSORT)],
            &["Ann", "Bob", "Cid", "Dee"],
        ),
        (
            "omit current",
            vec![plain(id::OMIT_RECORD)],
            &["Bob", "Cid", "Dee"],
        ),
        (
            "show omitted only",
            vec![plain(id::OMIT_RECORD), plain(id::SHOW_OMITTED)],
            &["Ann"],
        ),
        (
            "show all",
            vec![plain(id::OMIT_RECORD), plain(id::SHOW_ALL)],
            &["Ann", "Bob", "Cid", "Dee"],
        ),
    ];
    for (name, steps, want) in cases {
        let (_, _, h) = run(vec![script(1, steps.clone())], "");
        assert_eq!(h.names(), *want, "{name}");
    }
}

#[test]
fn go_to_record_loop_over_found_set() {
    let next = step(
        id::GO_TO_RECORD,
        StepParams::GoToRecord {
            dest: RecordDest::Next,
            exit_after_last: true,
            dialog: false,
        },
    );
    let steps = vec![
        var("$names", "\"\""),
        plain(id::LOOP),
        var("$names", "$names & People::Name"),
        next,
        plain(id::END_LOOP),
        exit("$names & Get(RecordNumber)"),
    ];
    let (got, o, _) = run(vec![script(1, steps)], "");
    assert_eq!(got, "AnnBobCidDee4");
    assert_eq!(o.last_error, 0);
}

#[test]
fn error_capture() {
    let steps = |capture: bool| {
        vec![
            step(id::SET_ERROR_CAPTURE, StepParams::Switch(capture)),
            step(
                id::GO_TO_LAYOUT,
                StepParams::GoToLayout {
                    dest: LayoutDest::Layout(99),
                    animation: 0,
                },
            ),
            var("$e", "Get(LastError)"),
            var("$after", "Get(LastError)"),
            exit("$e & \"/\" & $after"),
        ]
    };
    let (got, _, h) = run(vec![script(1, steps(true))], "");
    assert_eq!(got, "105/0");
    assert!(h.errors.is_empty());
    let (got, _, h) = run(vec![script(1, steps(false))], "");
    assert_eq!(got, "105/0");
    assert_eq!(h.errors, [105]);
}

#[test]
fn custom_dialog_buttons_and_input() {
    let dialog = step(
        id::SHOW_CUSTOM_DIALOG,
        StepParams::ShowCustomDialog {
            title: Some(c("\"Rename\"")),
            message: Some(c("\"New name for \" & People::Name")),
            buttons: vec![
                DialogButton {
                    label: Some(c("\"OK\"")),
                    commit: true,
                },
                DialogButton {
                    label: Some(c("\"Cancel\"")),
                    commit: false,
                },
                DialogButton {
                    label: None,
                    commit: false,
                },
            ],
            input: Some((field(NAME), Some(c("\"Name\"")))),
        },
    );
    for (button, want) in [(1, "1/Zoe"), (2, "2/Ann")] {
        let mut h = TestHost::new(PEOPLE);
        h.answers.push_back(DialogAnswer {
            button,
            inputs: vec!["Zoe".into()],
        });
        let mut e = Engine::new(vec![script(
            1,
            vec![
                dialog.clone(),
                exit("Get(LastMessageChoice) & \"/\" & People::Name"),
            ],
        )]);
        let o = e.perform(&mut h, 1, None);
        assert_eq!(o.result.to_text(), want);
        let d = &h.dialogs[0];
        assert_eq!(
            (d.title.as_str(), d.message.as_str()),
            ("Rename", "New name for Ann")
        );
        assert_eq!(d.buttons, ["OK", "Cancel"]);
        assert_eq!(d.inputs[0].1, "Name");
    }
}

#[test]
fn delete_and_layouts() {
    let steps = vec![
        step(
            id::GO_TO_LAYOUT,
            StepParams::GoToLayout {
                dest: LayoutDest::Layout(2),
                animation: 0,
            },
        ),
        step(
            id::GO_TO_RECORD,
            StepParams::GoToRecord {
                dest: RecordDest::ByCalc(c("2")),
                exit_after_last: false,
                dialog: false,
            },
        ),
        step(
            id::DELETE_RECORD,
            StepParams::Records {
                dialog: false,
                count: None,
            },
        ),
        step(
            id::GO_TO_LAYOUT,
            StepParams::GoToLayout {
                dest: LayoutDest::Original,
                animation: 0,
            },
        ),
        exit("Get(FoundCount) & People::Name"),
    ];
    let (got, _, h) = run(vec![script(1, steps)], "");
    assert_eq!(got, "3Cid");
    assert_eq!(h.names(), ["Ann", "Cid", "Dee"]);
    assert_eq!(h.win.layout, 1);
}

#[test]
fn button_runs_one_step() {
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(vec![script(
        1,
        vec![set_field(NAME, "Get(ScriptParameter)")],
    )]);
    let o = e.run_step(&mut h, &perform(1, Some("\"Button\"")));
    assert_eq!(o.last_error, 0);
    assert_eq!(h.names()[0], "Button");
}

/// Resumable runs: a dialog inside a sub-script suspends the whole stack;
/// the answer resumes it where it stopped.
#[test]
fn resumable_dialog_in_sub_script() {
    let dialog = step(
        id::SHOW_CUSTOM_DIALOG,
        StepParams::ShowCustomDialog {
            title: Some(c("\"Q\"")),
            message: Some(c("Get(ScriptParameter)")),
            buttons: vec![
                DialogButton {
                    label: Some(c("\"Yes\"")),
                    commit: true,
                },
                DialogButton {
                    label: Some(c("\"No\"")),
                    commit: false,
                },
            ],
            input: Some((field(CITY), Some(c("\"City\"")))),
        },
    );
    let scripts = vec![
        script(
            1,
            vec![
                perform(2, Some("\"Sure?\"")),
                exit("Get(ScriptResult) & \"/\" & People::City"),
            ],
        ),
        script(2, vec![dialog, exit("Get(LastMessageChoice)")]),
    ];
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(scripts);
    let mut run = e.start(&mut h, 1, None);
    let Status::Suspended(Suspend::Dialog(d)) = run.resume(&mut e, &mut h, Answer::Start) else {
        panic!("no dialog")
    };
    assert_eq!(
        (d.message.as_str(), d.inputs[0].2.as_str()),
        ("Sure?", "Paris")
    );
    assert!(run.is_suspended());
    let answer = Answer::Dialog(DialogAnswer {
        button: 1,
        inputs: vec!["Lyon".into()],
    });
    let Status::Finished(o) = run.resume(&mut e, &mut h, answer) else {
        panic!("not finished")
    };
    assert_eq!(o.result.to_text(), "1/Lyon");
    assert!(h.dialogs.is_empty(), "the host's own dialog is not used");
}

#[test]
fn pause_continue_and_cancel() {
    let pause = step(id::PAUSE_RESUME, StepParams::Pause(None));
    let steps = vec![var("$a", "1"), pause, var("$a", "2"), exit("$a")];
    for (answer, result, halted) in [(Answer::Continue, "2", false), (Answer::Cancel, "", true)] {
        let mut h = TestHost::new(PEOPLE);
        let mut e = Engine::new(vec![script(1, steps.clone())]);
        let mut run = e.start(&mut h, 1, None);
        assert_eq!(
            run.resume(&mut e, &mut h, Answer::Start),
            Status::Suspended(Suspend::Pause(None))
        );
        let Status::Finished(o) = run.resume(&mut e, &mut h, answer) else {
            panic!()
        };
        assert_eq!((o.result.to_text().as_str(), o.halted), (result, halted));
    }
    // perform() runs through pauses.
    let (got, _, _) = run(vec![script(1, steps)], "");
    assert_eq!(got, "2");
}

#[test]
fn delete_confirmation_cancel_keeps_record() {
    let del = step(
        id::DELETE_RECORD,
        StepParams::Records {
            dialog: true,
            count: None,
        },
    );
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(vec![script(1, vec![del, exit("Get(LastError)")])]);
    let mut run = e.start(&mut h, 1, None);
    let Status::Suspended(Suspend::Dialog(d)) = run.resume(&mut e, &mut h, Answer::Start) else {
        panic!()
    };
    assert_eq!(d.buttons, ["Delete", "Cancel"]);
    let cancel = Answer::Dialog(DialogAnswer {
        button: 2,
        inputs: Vec::new(),
    });
    let Status::Finished(o) = run.resume(&mut e, &mut h, cancel) else {
        panic!()
    };
    assert_eq!(o.result.to_text(), "1");
    assert_eq!(h.names().len(), 4);
}

#[test]
fn missing_script_and_sub_script_error() {
    let mut h = TestHost::new(PEOPLE);
    let mut e = Engine::new(vec![script(
        1,
        vec![
            step(id::SET_ERROR_CAPTURE, StepParams::Switch(true)),
            perform(9, None),
            exit("Get(LastError)"),
        ],
    )]);
    assert_eq!(e.perform(&mut h, 7, None).last_error, error::SCRIPT_MISSING);
    let o = e.perform(&mut h, 1, None);
    assert_eq!(o.result.to_text(), "104");
}
