//! Tests of the M5 step groups (`steps.rs`) and of the debugger's
//! stepping, on the People database of `tests.rs`. Parameters are laid out
//! as FileMaker 26 stores them (FORMAT.md §8).

use super::*;
use crate::steps::sid;

/// Compiles a calculation in script context.
fn calc(h: &TestHost, src: &str) -> Calc {
    text::Names {
        db: &h.db,
        scripts: &[],
    }
    .compile(src)
    .unwrap()
}

/// A raw step without inline reference: calculations at `[128 + i]`,
/// option bits, bytes 22–23.
fn rawp(id: u16, calcs: &[Calc], options: u32, arg: u16) -> Step {
    let mut s = raw_step(id, T, NAME, calcs, options);
    s.raw[6..20].fill(0);
    s.arg = arg;
    s
}

/// A raw step on field `f` (inline reference).
fn rawf(id: u16, f: u32, calcs: &[Calc], options: u32) -> Step {
    raw_step(id, T, f, calcs, options)
}

/// Puts the three fields on layout 1, named after them.
fn with_fields(h: &mut TestHost) {
    h.db.layouts[0].objects = [(NAME, "Name"), (AGE, "Age"), (CITY, "City")]
        .iter()
        .enumerate()
        .map(|(i, (f, n))| fmp12::layouts::LayoutObject {
            id: i as u64 + 1,
            kind: 1,
            field: Some((T as u64, *f as u64, 1)),
            name: n.to_string(),
            ..Default::default()
        })
        .collect();
}

/// Runs `steps` as script 1 with error capture on; the result is the
/// last step's Exit Script value.
fn go(h: &mut TestHost, steps: Vec<Step>) -> (Engine, Outcome) {
    let mut all = vec![step(id::SET_ERROR_CAPTURE, StepParams::Switch(true))];
    all.extend(steps);
    let mut e = Engine::new(vec![script(1, all)]);
    let o = e.perform(h, 1, None);
    (e, o)
}

fn exit_c(h: &TestHost, src: &str) -> Step {
    step(id::EXIT_SCRIPT, StepParams::ExitScript(Some(calc(h, src))))
}

fn var_c(h: &TestHost, name: &str, src: &str) -> Step {
    step(
        id::SET_VARIABLE,
        StepParams::SetVariable {
            name: name.into(),
            value: Some(calc(h, src)),
            repetition: None,
        },
    )
}

/// Every step id of the catalogue: what the engine does with a new step
/// (FileMaker's default parameters). Prints the table of PROGRESS.md
/// with `--nocapture`.
#[test]
fn step_coverage() {
    struct Count(TestHost, Vec<u16>);
    impl Host for Count {
        fn db(&mut self) -> &mut Database {
            &mut self.0.db
        }
        fn window(&mut self) -> &mut Window {
            &mut self.0.win
        }
        fn other(&mut self, s: &Step) -> u32 {
            self.1.push(s.id);
            error::UNAVAILABLE
        }
    }
    let (mut runs, mut stub, mut host) = (Vec::new(), Vec::new(), Vec::new());
    for id in 1..=250u16 {
        let Some(name) = fmp12::scripts::step_name(id) else {
            continue;
        };
        let mut h = Count(TestHost::new(PEOPLE), Vec::new());
        let mut e = Engine::new(vec![script(1, vec![Step::new(id)])]);
        e.perform(&mut h, 1, None);
        let line = format!("{id} {name}");
        if !h.1.is_empty() {
            host.push(line);
        } else if UNSUPPORTED.contains(&id)
            || [sid::SPEAK, sid::DIAL_PHONE, sid::EXECUTE_SQL].contains(&id)
            || [
                sid::RECOVER_FILE,
                sid::SET_ALLOWED_ORIENTATIONS,
                sid::REGION_MONITOR,
                sid::LOCAL_NOTIFICATION,
                sid::CONFIGURE_AI_ACCOUNT,
                crate::datafiles::INSERT_OBJECT,
            ]
            .contains(&id)
        {
            stub.push(line);
        } else {
            runs.push(line);
        }
    }
    println!("RUNS {}: {}", runs.len(), runs.join(", "));
    println!("DESKTOP ERROR {}: {}", stub.len(), stub.join(", "));
    println!("NOT RUN {}: {}", host.len(), host.join(", "));
    let partial: Vec<String> = PARTIAL_STEPS
        .iter()
        .map(|(id, why)| {
            let line = format!("{id} {}", fmp12::scripts::step_name(*id).unwrap_or("?"));
            assert!(
                runs.contains(&line),
                "{line} is listed as partial but does not run"
            );
            format!("{line} ({why})")
        })
        .collect();
    println!("PARTIAL {}: {}", partial.len(), partial.join(", "));
    assert_eq!(runs.len() + stub.len() + host.len(), 222);
    // Every step runs; none is left to `Host::other` (PROGRESS.md M5).
    assert_eq!(
        (runs.len(), stub.len(), host.len()),
        (155, 67, 0),
        "{host:?}"
    );
}

/// `Get()` flags a script on a real file answers (docs/COVERAGE.md):
/// each one of docs/reference/get_functions.txt is evaluated by Exit
/// Script; "needs a host" (1225) means no BaseCraft layer answers it.
#[test]
fn get_coverage() {
    let blank =
        fmp12::File::from_bytes(include_bytes!("../../../fmp12/assets/blank.fmp12").to_vec())
            .unwrap();
    let db = Database::load(&blank).unwrap();
    let layout = db.layouts.first().map_or(0, |l| l.id);
    let mut win = Window::new(&db, layout);
    // What the app fills in for every run (`App::script_window`).
    win.host = crate::HostInfo {
        file_name: "blank".into(),
        file_path: "file:/C:/blank.fmp12".into(),
        file_size: Some(1),
        content: Some((800.0, 560.0)),
        screen: Some((1920.0, 1080.0)),
        scale: Some(1.0),
        modifier_keys: Some(0),
        quick_find: String::new(),
    };
    let mut h = TestHost {
        db,
        win,
        answers: VecDeque::new(),
        dialogs: Vec::new(),
        errors: Vec::new(),
    };
    let flags: Vec<&str> = calc::functions::GET_REFERENCE
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Get ( ")?.strip_suffix(" )"))
        .filter(|f| *f != "flag")
        .collect();
    let (mut answered, mut missing, mut uncompiled) = (Vec::new(), Vec::new(), Vec::new());
    for f in &flags {
        let names = text::Names {
            db: &h.db,
            scripts: &[],
        };
        // A flag the script compiler does not know cannot be answered.
        let Ok(c) = names.compile(&format!("Get ( {f} )")) else {
            missing.push(*f);
            uncompiled.push(*f);
            continue;
        };
        let exit = step(id::EXIT_SCRIPT, StepParams::ExitScript(Some(c)));
        let (_, o) = go(&mut h, vec![exit]);
        if o.last_error == calc::error::code::NOT_SUPPORTED {
            missing.push(*f);
        } else {
            answered.push(*f);
        }
    }
    for p in PARTIAL_GET {
        assert!(flags.contains(&p), "{p} is not a Get flag");
    }
    answered.retain(|f| !PARTIAL_GET.contains(f));
    missing.retain(|f| !PARTIAL_GET.contains(f));
    println!("ANSWERED {}: {}", answered.len(), answered.join(", "));
    println!("PARTIAL {}: {}", PARTIAL_GET.len(), PARTIAL_GET.join(", "));
    println!("MISSING {}: {}", missing.len(), missing.join(", "));
    println!(
        "NOT COMPILED {}: {}",
        uncompiled.len(),
        uncompiled.join(", ")
    );
    assert_eq!(flags.len(), 139);
    assert_eq!(
        (answered.len(), PARTIAL_GET.len(), missing.len()),
        (86, 13, 40)
    );
}

/// Flags answered with a fixed or approximate value, or only in some
/// states (docs/COVERAGE.md).
const PARTIAL_GET: [&str; 13] = [
    // Only fields and objects a script entered: the app's own focus is
    // not passed to the engine.
    "ActiveFieldName",
    "ActiveFieldTableName",
    "ActiveFieldContents",
    "ActiveRepetitionNumber",
    "ActiveLayoutObjectName",
    // Only a record a script opened.
    "ModifiedFields",
    // From LANG / LC_ALL only (English otherwise).
    "SystemLanguage",
    // No Caps Lock (egui does not report it).
    "ActiveModifierKeys",
    // UTC: the engine has no time-zone data.
    "CurrentDate",
    "CurrentTime",
    "CurrentTimestamp",
    "CurrentHostTimestamp",
    // Empty, but after Revert Transaction with a custom error.
    "LastErrorDetail",
];

/// Steps that run but ignore stored options or do nothing where FileMaker
/// acts (docs/COVERAGE.md), with the reason.
const PARTIAL_STEPS: [(u16, &str); 27] = [
    (18, "no spelling check"),
    (19, "no spelling check"),
    (20, "no spelling check"),
    (35, "stored import options not used (dialog)"),
    (36, "stored export options not used (dialog)"),
    (37, "stored path not used"),
    (56, "stored path and options not decoded (file picker)"),
    (63, "mailto: only, no SMTP"),
    (81, "does nothing"),
    (84, "does nothing"),
    (94, "does nothing"),
    (120, "does nothing"),
    (128, "a script's \"current field\" searches every field"),
    (131, "stored path and options not decoded (file picker)"),
    (132, "stored path not decoded (save dialog)"),
    (138, "\"with dialog\" shows no dialog"),
    (146, "web viewers are not drawn: only checks the name"),
    (169, "popover not closed"),
    (209, "no dictionaries"),
    (205, "Skip auto-enter / Skip data entry validation ignored"),
    (
        207,
        "condition, error code, message read as parameters 0-2: not checked",
    ),
    (237, "no web viewer"),
    (43, "prints to PDF only"),
    (
        3,
        "schema, layouts, scripts only: no UUIDs, DDR_INFO, step parameters",
    ),
    (160, "http and https only; some cURL options ignored"),
    (203, "no portalData writes, modId always 0"),
    (225, "no predefined system prompt; option slots not checked"),
];

#[test]
fn control_steps() {
    let mut h = TestHost::new(PEOPLE);
    // Perform Script by name ([130]) with a parameter ([128]).
    let mut by_name = rawp(id::PERFORM_SCRIPT, &[calc(&h, "\"p\"")], 0x4800, 0);
    by_name
        .raw_params
        .kv
        .insert((vec![130, 5], 5), calc(&h, "\"S2\"").0);
    by_name.params = StepParams::PerformScript {
        script: None,
        parameter: Some(calc(&h, "\"p\"")),
    };
    let sub = vec![exit_c(&h, "Get ( ScriptParameter ) & Get ( ScriptName )")];
    let mut timer = rawp(sid::INSTALL_ON_TIMER, &[calc(&h, "1"), calc(&h, "5")], 0, 0);
    timer.raw[6..9].copy_from_slice(&[2, 1, 2]);
    let main = vec![
        step(id::SET_ERROR_CAPTURE, StepParams::Switch(true)),
        by_name,
        timer,
        rawp(sid::SET_ERROR_LOGGING, &[calc(&h, "\"ctx\"")], 0x3_4000, 0),
        step(id::ALLOW_USER_ABORT, StepParams::Switch(false)),
        exit_c(&h, "Get ( ScriptResult ) & Get ( AllowAbortState )"),
    ];
    let mut e = Engine::new(vec![script(1, main), script(2, sub)]);
    let o = e.perform(&mut h, 1, None);
    assert_eq!(o.result.to_text(), "pS20");
    assert_eq!(
        e.timer.as_ref().map(|t| (t.script, t.interval)),
        Some((2, 5.0))
    );
    assert!(e.error_logging.is_some());
    // Halt Script stops every level.
    let mut e = Engine::new(vec![
        script(1, vec![perform(2, None), var("$x", "1")]),
        script(2, vec![plain(id::HALT_SCRIPT)]),
    ]);
    assert!(e.perform(&mut h, 1, None).halted);
}

/// Window, focus and open-record flags (docs/COVERAGE.md, `Get()`).
#[test]
fn get_window_and_record_flags() {
    let mut h = TestHost::new(PEOPLE);
    with_fields(&mut h);
    h.win.host.file_name = "People".into();
    h.win.host.quick_find = "ann".into();
    let all = "Get ( ActiveFieldName ) & \"|\" & Get ( ActiveRepetitionNumber ) & \"|\" & \
               Get ( ActiveLayoutObjectName ) & \"|\" & Get ( ModifiedFields )";
    let steps = vec![
        // No active field: empty, not "needs a host".
        var_c(&h, "$none", all),
        step(
            id::GO_TO_OBJECT,
            StepParams::GoToObject {
                name: Some(calc(&h, "\"Age\"")),
                repetition: None,
            },
        ),
        set_field(CITY, "\"Lyon\""),
        set_field(NAME, "\"Ann\""),
        var_c(&h, "$on", all),
        exit_c(
            &h,
            "$none & \"/\" & $on & \"/\" & Get ( FileName ) & Get ( QuickFindText ) & \
             Get ( LastError )",
        ),
    ];
    let (_, o) = go(&mut h, steps);
    // Setting Name to its own value does not modify it.
    assert_eq!(
        (o.result.to_text(), o.last_error),
        ("|0||/Age|1|Age|City/Peopleann0".to_string(), 0)
    );
    // The old flag name compiles (to CurrentPrivilegeSetName).
    assert!(
        text::Names {
            db: &h.db,
            scripts: &[]
        }
        .compile("Get ( PrivilegeSetName )")
        .is_ok()
    );
}

#[test]
fn navigation_steps() {
    let mut h = TestHost::new(PEOPLE);
    with_fields(&mut h);
    let steps = vec![
        rawp(sid::NEXT_FIELD, &[], 0, 0),
        rawp(sid::NEXT_FIELD, &[], 0, 0),
        var_c(&h, "$a", "Get ( ActiveFieldName )"),
        rawp(sid::PREVIOUS_FIELD, &[], 0, 0),
        rawp(sid::PREVIOUS_FIELD, &[], 0, 0),
        var_c(&h, "$b", "Get ( ActiveFieldName )"),
        step(
            id::GO_TO_OBJECT,
            StepParams::GoToObject {
                name: Some(calc(&h, "\"Age\"")),
                repetition: None,
            },
        ),
        var_c(&h, "$c", "Get ( ActiveFieldName )"),
        step(
            id::GO_TO_OBJECT,
            StepParams::GoToObject {
                name: Some(calc(&h, "\"Nope\"")),
                repetition: None,
            },
        ),
        exit_c(
            &h,
            "$a & \"/\" & $b & \"/\" & $c & \"/\" & Get ( LastError )",
        ),
    ];
    let (_, o) = go(&mut h, steps);
    // Previous from the first field wraps to the last.
    assert_eq!(o.result.to_text(), "Age/City/Age/116");
}

#[test]
fn go_to_related_record() {
    let (mut h, [inv, ito, ..]) = lookup_host();
    let cto =
        h.db.occurrences
            .iter()
            .find(|o| o.name == "Clients")
            .unwrap()
            .id;
    let clients =
        h.db.layouts
            .iter()
            .find(|l| l.name == "Clients")
            .unwrap()
            .id;
    let lo = (cto - 128) as u16;
    let gtrr = |options: u32| {
        let mut s = rawp(sid::GO_TO_RELATED_RECORD, &[], options, 5);
        s.raw[6..9].copy_from_slice(&[2, 1, clients as u8]);
        s.raw_params
            .kv
            .insert((vec![128], 17), vec![3, 0xD0, (lo >> 8) as u8, lo as u8]);
        s
    };
    let (_, o) = go(&mut h, vec![gtrr(0x100)]);
    assert_eq!(o.last_error, 0);
    assert_eq!(h.win.layout, clients);
    // Only the related client (Ada), not Bob.
    assert_eq!(h.win.found.len(), 1);
    // From Clients back to Invoices on the current layout: 414.
    let back = {
        let lo = (ito - 128) as u16;
        let mut s = rawp(sid::GO_TO_RELATED_RECORD, &[], 0x100, 1);
        s.raw_params
            .kv
            .insert((vec![128], 17), vec![3, 0xD0, (lo >> 8) as u8, lo as u8]);
        s
    };
    let (_, o) = go(&mut h, vec![back]);
    assert_eq!(o.last_error, 414);
    // In a new window: the first window stays behind it.
    let invoices =
        h.db.layouts
            .iter()
            .find(|l| l.name == "Invoices")
            .unwrap()
            .id;
    h.win = Window::new(&h.db, invoices);
    let (e, o) = go(&mut h, vec![gtrr(0x2_0100)]);
    assert_eq!(o.last_error, 0);
    assert_eq!(e.windows.len(), 1);
    assert_eq!(e.windows[0].layout, invoices);
    let _ = inv;
}

/// Perform Find on a related field: clients having a matching invoice.
#[test]
fn perform_find_related_field() {
    let (mut h, [_, ito, cid, ..]) = lookup_host();
    let clients =
        h.db.layouts
            .iter()
            .find(|l| l.name == "Clients")
            .unwrap()
            .id;
    let crit = |omit: bool, t: &str| FindRequest {
        omit,
        criteria: vec![(
            FieldRef {
                to: ito,
                field: cid,
                repetition: 1,
            },
            t.to_string(),
        )],
    };
    for (omit, t, want, err) in [
        (false, "1", vec![0], 0),
        (true, "1", vec![1], 0),
        (false, "3", vec![], 401),
    ] {
        h.win = Window::new(&h.db, clients);
        let find = step(
            id::PERFORM_FIND,
            StepParams::Find {
                pause: false,
                restore: true,
                requests: vec![crit(omit, t)],
            },
        );
        let (_, o) = go(&mut h, vec![find]);
        assert_eq!(o.last_error, err, "{omit} {t}");
        assert_eq!(h.win.found, want, "{omit} {t}");
    }
}

#[test]
fn editing_steps() {
    let mut h = TestHost::new(PEOPLE);
    with_fields(&mut h);
    let steps = vec![
        rawf(sid::COPY, NAME, &[], 0),
        rawf(id::PASTE, CITY, &[], 0x1000),
        step(
            id::GO_TO_FIELD,
            StepParams::GoToField {
                field: Some(field(CITY)),
                select: false,
            },
        ),
        rawp(sid::SELECT_ALL, &[], 0, 0),
        var_c(
            &h,
            "$a",
            "Get ( ActiveSelectionStart ) & \"/\" & Get ( ActiveSelectionSize )",
        ),
        rawf(sid::SET_SELECTION, CITY, &[calc(&h, "2"), calc(&h, "3")], 0),
        var_c(
            &h,
            "$b",
            "Get ( ActiveSelectionStart ) & \"/\" & Get ( ActiveSelectionSize )",
        ),
        rawf(sid::CLEAR, AGE, &[], 0),
        var_c(&h, "$c", "People::Age"),
        rawp(sid::UNDO_REDO, &[], 0, 1),
        var_c(&h, "$d", "People::Age"),
        rawp(sid::UNDO_REDO, &[], 0, 2),
        exit_c(
            &h,
            "People::City & \"|\" & $a & \"|\" & $b & \"|\" & $c & \"|\" & $d & \"|\" & People::Age",
        ),
    ];
    let (e, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "Ann|1/3|2/2||34|");
    assert_eq!(e.clipboard.as_deref(), Some("Ann"));
}

#[test]
fn field_steps() {
    let mut h = TestHost::new(PEOPLE);
    // Set Next Serial Value on a field that is not a serial: 5.
    let serial = vec![rawf(sid::SET_NEXT_SERIAL, AGE, &[calc(&h, "100")], 0x4000)];
    let (_, o) = go(&mut h, serial);
    assert_eq!(o.last_error, 5);
    // Insert from Last Visited: the record committed last.
    let steps = vec![
        plain(id::COMMIT),
        step(
            id::GO_TO_RECORD,
            StepParams::GoToRecord {
                dest: RecordDest::Next,
                exit_after_last: false,
                dialog: false,
            },
        ),
        rawf(sid::INSERT_FROM_LAST_VISITED, CITY, &[], 0x1000),
        exit_c(&h, "People::City"),
    ];
    let (_, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "Paris");
    // Insert from Index: the user picks a value of the field's index.
    let mut e = Engine::new(vec![script(
        1,
        vec![rawf(sid::INSERT_FROM_INDEX, NAME, &[], 0x1000)],
    )]);
    let mut run = e.start(&mut h, 1, None);
    let Status::Suspended(Suspend::Ui(UiRequest::Index { values })) =
        run.resume(&mut e, &mut h, Answer::Start)
    else {
        panic!("no index")
    };
    assert_eq!(values, ["Ann", "Bob", "Cid", "Dee"]);
    run.resume(&mut e, &mut h, Answer::Text("Zed".into()));
    assert_eq!(h.db.tables[0].records[1].values[&NAME], "Zed");
    // Export Field Contents: the text and the stored path.
    let mut s = rawf(sid::EXPORT_FIELD, NAME, &[], 0);
    // As FileMaker 26 writes it: only the strings are XOR 0x5A.
    let x = |s: &[u8]| s.iter().map(|b| b ^ 0x5A).collect::<Vec<u8>>();
    let path: Vec<u8> = [&[0u8, 0, 0, 1, 4][..], &x(b"file"), &[0, 7], &x(b"out.txt")].concat();
    s.raw_params.kv.insert((vec![128], 130), path);
    let mut e = Engine::new(vec![script(1, vec![s])]);
    let mut run = e.start(&mut h, 1, None);
    assert_eq!(
        run.resume(&mut e, &mut h, Answer::Start),
        Status::Suspended(Suspend::Ui(UiRequest::ExportField {
            path: Some("file:out.txt".into()),
            text: "Zed".into(),
        }))
    );
}

#[test]
fn record_steps() {
    let mut h = TestHost::new(PEOPLE);
    let steps = vec![
        rawp(sid::OPEN_RECORD, &[], 0, 0),
        set_field(NAME, "\"Xavier\""),
        plain(id::REVERT_RECORD),
        rawp(sid::COPY_RECORD, &[], 0, 0),
        exit("People::Name"),
    ];
    with_fields(&mut h);
    let (e, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "Ann");
    assert_eq!(e.clipboard.as_deref(), Some("Ann\t34\tParis"));
    let (e, _) = go(&mut h, vec![rawp(sid::COPY_ALL_RECORDS, &[], 0, 0)]);
    assert_eq!(e.clipboard.unwrap().split('\r').count(), 4);
    // Truncate Table: no table chosen is 106; a table, no dialog, empties it.
    let (_, o) = go(&mut h, vec![rawp(sid::TRUNCATE_TABLE, &[], 0x80, 0)]);
    assert_eq!((o.last_error, h.db.tables[0].records.len()), (106, 4));
    let mut tt = rawp(sid::TRUNCATE_TABLE, &[], 0x80, 1);
    tt.raw[6..10].copy_from_slice(&[3, 2, 0x80, (T - 128) as u8]);
    let (_, o) = go(&mut h, vec![tt]);
    assert_eq!((o.last_error, h.db.tables[0].records.len()), (0, 0));
    assert!(h.win.found.is_empty());
}

#[test]
fn found_set_steps() {
    let mut h = TestHost::new(PEOPLE);
    with_fields(&mut h);
    // Find Matching Records on Age (27): Bob and Dee; constrain by City.
    let steps = vec![
        step(
            id::GO_TO_RECORD,
            StepParams::GoToRecord {
                dest: RecordDest::Next,
                exit_after_last: false,
                dialog: false,
            },
        ),
        rawf(sid::FIND_MATCHING, AGE, &[], 0),
        var_c(&h, "$a", "Get ( FoundCount )"),
        step(
            id::ENTER_FIND_MODE,
            StepParams::Find {
                restore: false,
                requests: Vec::new(),
                pause: false,
            },
        ),
        set_field(CITY, "\"Paris\""),
        rawp(sid::CONSTRAIN, &[], 0, 0),
        var_c(&h, "$b", "Get ( FoundCount ) & People::Name"),
        rawp(sid::MODIFY_LAST_FIND, &[], 0, 0),
        var_c(&h, "$c", "Get ( WindowMode )"),
        step(id::ENTER_BROWSE_MODE, StepParams::Mode { pause: false }),
        rawp(sid::QUICK_FIND, &[calc(&h, "\"oslo\"")], 0x4000, 0),
        var_c(&h, "$d", "Get ( FoundCount ) & People::Name"),
        rawp(sid::QUICK_FIND, &[calc(&h, "\"zzz\"")], 0x4000, 0),
        exit_c(
            &h,
            "$a & \"|\" & $b & \"|\" & $c & \"|\" & $d & \"|\" & Get ( LastError ) & Get ( FoundCount )",
        ),
    ];
    let (_, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "2|1Dee|1|1Bob|4010");
    // Quick Find skips a field whose "Include field for Quick Find" is off.
    for o in h.db.layouts.iter_mut().flat_map(|l| &mut l.objects) {
        if o.field.is_some_and(|f| f.1 == CITY as u64) {
            o.field_flags |= fmp12::layouts::field_flags::NO_QUICK_FIND;
        }
    }
    let quick = rawp(sid::QUICK_FIND, &[calc(&h, "\"oslo\"")], 0x4000, 0);
    let (_, o) = go(&mut h, vec![quick]);
    assert_eq!(o.last_error, 401);
    // Find Matching Records with no field and no active field: 401.
    h.win.field = None;
    let (_, o) = go(&mut h, vec![rawp(sid::FIND_MATCHING, &[], 0, 0)]);
    assert_eq!(o.last_error, 401);
}

#[test]
fn constrain_with_omit_only() {
    let mut h = TestHost::new(PEOPLE);
    let steps = vec![
        step(
            id::PERFORM_FIND,
            StepParams::Find {
                restore: true,
                requests: vec![FindRequest {
                    omit: false,
                    criteria: vec![(field(CITY), "Paris".into())],
                }],
                pause: false,
            },
        ),
        step(
            id::ENTER_FIND_MODE,
            StepParams::Find {
                restore: false,
                requests: Vec::new(),
                pause: false,
            },
        ),
        set_field(AGE, "\"27\""),
        plain(id::OMIT_RECORD),
        rawp(sid::CONSTRAIN, &[], 0, 0),
        exit_c(&h, "Get ( FoundCount ) & People::Name"),
    ];
    let (_, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "1Ann");
}

#[test]
fn window_steps() {
    let mut h = TestHost::new(PEOPLE);
    h.win.name = "People".into();
    let geo = |h: &TestHost, v: [&str; 4]| -> Vec<Calc> {
        std::iter::once(calc(h, "\"W2\""))
            .chain(v.iter().map(|s| calc(h, s)))
            .collect()
    };
    let steps = vec![
        rawp(
            sid::NEW_WINDOW,
            &geo(&h, ["300", "400", "30", "40"]),
            0x4002,
            1,
        ),
        var_c(
            &h,
            "$a",
            "Get ( WindowName ) & Get ( WindowWidth ) & Get ( WindowHeight )",
        ),
        rawp(sid::SELECT_WINDOW, &[calc(&h, "\"People\"")], 0x4002, 1),
        var_c(&h, "$b", "Get ( WindowName )"),
        rawp(sid::SELECT_WINDOW, &[calc(&h, "\"Nope\"")], 0x4002, 1),
        var_c(&h, "$c", "Get ( LastError )"),
        rawp(sid::CLOSE_WINDOW, &[calc(&h, "\"W2\"")], 0x4002, 1),
        rawp(
            sid::MOVE_RESIZE_WINDOW,
            &[calc(&h, "\"\""), calc(&h, "500")],
            0x4002,
            0,
        ),
        rawp(sid::VIEW_AS, &[], 2, 3),
        rawp(sid::SET_ZOOM_LEVEL, &[], 2, 5),
        rawp(sid::SET_ZOOM_LEVEL, &[], 2, 9),
        rawp(sid::SHOW_HIDE_TOOLBARS, &[], 2, 2),
        rawp(
            sid::SET_WINDOW_TITLE,
            &[calc(&h, "\"\""), calc(&h, "\"T9\"")],
            0x4002,
            0,
        ),
        exit_c(
            &h,
            "$a & \"|\" & $b & \"|\" & $c & \"|\" & Get ( WindowHeight ) & \"|\" & Get ( LayoutViewState ) & \"|\" & Get ( WindowZoomLevel ) & \"|\" & Get ( StatusAreaState ) & \"|\" & Get ( WindowName )",
        ),
    ];
    let (e, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "W2400300|People|112|500|2|200|0|T9");
    assert!(e.windows.is_empty());
    // Adjust Window is the app's: maximize.
    let mut e = Engine::new(vec![script(1, vec![rawp(sid::ADJUST_WINDOW, &[], 2, 2)])]);
    let mut run = e.start(&mut h, 1, None);
    assert_eq!(
        run.resume(&mut e, &mut h, Answer::Start),
        Status::Suspended(Suspend::Ui(UiRequest::Adjust(Adjust::Maximize)))
    );
    // Resize to Fit: the window's content takes the layout's size.
    h.db.layouts[0].width = 600.0;
    h.db.layouts[0].parts = vec![
        fmp12::layouts::Part {
            kind: fmp12::layouts::part::BODY,
            top: 0.0,
            height: 200.0,
            ..Default::default()
        },
        fmp12::layouts::Part {
            kind: fmp12::layouts::part::FOOTER,
            top: 200.0,
            height: 50.0,
            ..Default::default()
        },
    ];
    let mut e = Engine::new(vec![script(1, vec![rawp(sid::ADJUST_WINDOW, &[], 2, 1)])]);
    let mut run = e.start(&mut h, 1, None);
    assert_eq!(
        run.resume(&mut e, &mut h, Answer::Start),
        Status::Suspended(Suspend::Ui(UiRequest::Adjust(Adjust::ResizeToFit)))
    );
    assert_eq!((h.win.width, h.win.height), (600.0, 250.0));
    // Closing the last window closes the file and stops the script.
    let (_, o) = go(
        &mut h,
        vec![rawp(sid::CLOSE_WINDOW, &[], 2, 0), var("$x", "1")],
    );
    assert!(o.halted);
}

#[test]
fn file_and_misc_steps() {
    let mut h = TestHost::new(PEOPLE);
    let ui = |h: &mut TestHost, s: Step| {
        let mut e = Engine::new(vec![script(1, vec![s])]);
        let mut run = e.start(h, 1, None);
        match run.resume(&mut e, h, Answer::Start) {
            Status::Suspended(Suspend::Ui(r)) => Some(r),
            _ => None,
        }
    };
    let url = rawp(
        sid::OPEN_URL,
        &[calc(&h, "\"https://example.com\"")],
        0x4080,
        0,
    );
    assert_eq!(
        ui(&mut h, url),
        Some(UiRequest::OpenUrl {
            url: "https://example.com".into(),
            dialog: false,
        })
    );
    // Send Mail: To [128], CC [129], subject [131], message [132].
    let mut cs = vec![
        calc(&h, "\"a@b.c\""),
        calc(&h, "\"c@d.e\""),
        calc(&h, "\"\""),
    ];
    cs.push(calc(&h, "\"Hi there\""));
    cs.push(calc(&h, "\"Body\""));
    let mail = rawp(sid::SEND_MAIL, &cs, 0x4080, 0);
    assert_eq!(
        ui(&mut h, mail),
        Some(UiRequest::OpenUrl {
            url: "mailto:a@b.c?cc=c%40d.e&subject=Hi%20there&body=Body".replace("c%40d.e", "c@d.e"),
            dialog: false,
        })
    );
    assert_eq!(
        ui(&mut h, rawp(sid::NEW_FILE, &[], 0, 0)),
        Some(UiRequest::NewFile)
    );
    // Get Folder Path: the answer goes into the variable.
    let mut gf = rawp(sid::GET_FOLDER_PATH, &[], 0x4104, 0);
    let name: Vec<u8> = b"$folder".iter().map(|b| b ^ 0x5A).collect();
    gf.raw_params.kv.insert((vec![128], 1), name);
    let mut e = Engine::new(vec![script(1, vec![gf, exit("$folder")])]);
    let mut run = e.start(&mut h, 1, None);
    run.resume(&mut e, &mut h, Answer::Start);
    let Status::Finished(o) = run.resume(&mut e, &mut h, Answer::Text("/C:/x/".into())) else {
        panic!()
    };
    assert_eq!(o.result.to_text(), "/C:/x/");
    // Stubs and refused steps return FileMaker's desktop errors.
    for (id, code) in [
        (sid::SPEAK, 3),
        (sid::DIAL_PHONE, 3),
        (sid::EXECUTE_SQL, 5),
        (sid::RECOVER_FILE, 3),
        (sid::SET_ALLOWED_ORIENTATIONS, 4),
        (164, 3), // Perform Script on Server
        (212, 5), // Configure AI Account, no parameters
    ] {
        let (_, o) = go(&mut h, vec![rawp(id, &[], 0x80, 0)]);
        assert_eq!(o.last_error, code, "step {id}");
    }
    let (_, o) = go(&mut h, vec![rawp(sid::SET_MULTI_USER, &[], 2, 3)]);
    assert_eq!(o.last_error, 0);
}

/// The debugger: stop at the first step, step over a sub-script, step
/// into one, step out, run to a breakpoint, halt.
#[test]
fn debugger_stepping() {
    let mut h = TestHost::new(PEOPLE);
    let main = vec![
        var("$a", "1"),
        perform(2, None),
        var("$b", "2"),
        perform(2, None),
        var("$c", "3"),
    ];
    let sub = vec![var("$s", "1"), var("$t", "2")];
    let mut e = Engine::new(vec![script(1, main), script(2, sub)]);
    let mut run = e.start(&mut h, 1, None);
    run.set_debug(true);
    let go = |run: &mut Run, e: &mut Engine, h: &mut TestHost, s: Stepping| {
        run.set_stepping(s);
        let st = run.resume(e, h, Answer::Continue);
        (st, run.current())
    };
    assert_eq!(
        run.resume(&mut e, &mut h, Answer::Start),
        Status::Suspended(Suspend::Breakpoint)
    );
    assert_eq!(run.current(), Some((1, 0)));
    // Step: $a, then over Perform Script.
    assert_eq!(go(&mut run, &mut e, &mut h, Stepping::Over).1, Some((1, 1)));
    assert_eq!(go(&mut run, &mut e, &mut h, Stepping::Over).1, Some((1, 2)));
    assert_eq!(run.locals()[0].0, "$a");
    // Into the second call, then out of it.
    assert_eq!(go(&mut run, &mut e, &mut h, Stepping::Over).1, Some((1, 3)));
    assert_eq!(go(&mut run, &mut e, &mut h, Stepping::Into).1, Some((2, 0)));
    assert_eq!(run.stack().len(), 2);
    assert_eq!(go(&mut run, &mut e, &mut h, Stepping::Out).1, Some((1, 4)));
    // A breakpoint, then halt there.
    let mut run = e.start(&mut h, 1, None);
    run.set_debug(true);
    run.breakpoints.insert((2, 1));
    run.resume(&mut e, &mut h, Answer::Start);
    assert_eq!(go(&mut run, &mut e, &mut h, Stepping::Run).1, Some((2, 1)));
    let Status::Finished(o) = run.resume(&mut e, &mut h, Answer::Cancel) else {
        panic!()
    };
    assert!(o.halted);
    // Debugger off: breakpoints are ignored.
    let mut run = e.start(&mut h, 1, None);
    run.breakpoints.insert((1, 2));
    assert!(matches!(
        run.resume(&mut e, &mut h, Answer::Start),
        Status::Finished(_)
    ));
}

#[test]
fn transactions() {
    let mut h = TestHost::new(PEOPLE);
    let open = || rawp(sid::OPEN_TRANSACTION, &[], 0, 0);
    let commit = || rawp(sid::COMMIT_TRANSACTION, &[], 0, 0);
    let steps = vec![
        open(),
        set_field(NAME, "\"Xavier\""),
        plain(id::NEW_RECORD),
        // Not reverted: the condition is false.
        rawp(sid::REVERT_TRANSACTION, &[calc(&h, "0")], 0, 0),
        var_c(&h, "$open", "Get(TransactionOpenState)"),
        rawp(sid::REVERT_TRANSACTION, &[], 0, 0),
        // Skipped: Revert goes on after Commit Transaction.
        var_c(&h, "$skipped", "1"),
        commit(),
        var_c(
            &h,
            "$a",
            "People::Name & Get(FoundCount) & $open & Get(TransactionOpenState) & $skipped",
        ),
        open(),
        set_field(NAME, "\"Yves\""),
        commit(),
        // No transaction open: error 3.
        rawp(sid::REVERT_TRANSACTION, &[], 0, 0),
        var_c(&h, "$b", "Get(LastError)"),
        open(),
        set_field(NAME, "\"Zoe\""),
        rawp(
            sid::REVERT_TRANSACTION,
            &[calc(&h, "1"), calc(&h, "5001"), calc(&h, "\"Nope\"")],
            0,
            0,
        ),
        commit(),
        exit_c(
            &h,
            "$a & People::Name & $b & Get(LastError) & Get(LastErrorDetail)",
        ),
    ];
    let (_, o) = go(&mut h, steps);
    assert_eq!(o.result.to_text(), "Ann410Yves35001Nope");
    assert_eq!(h.db.tables[0].records.len(), 4);
    // Halt Script reverts.
    let steps = vec![
        open(),
        set_field(NAME, "\"Xavier\""),
        plain(id::HALT_SCRIPT),
    ];
    go(&mut h, steps);
    assert_eq!(h.db.tables[0].records[0].values[&NAME], "Yves");
}

/// A validation error reverts the transaction unless Set Revert
/// Transaction on Error is off.
#[test]
fn transaction_revert_on_error() {
    let mut h = TestHost::new(PEOPLE);
    for f in h.db.tables[0].fields.iter_mut().filter(|f| f.id == NAME) {
        f.def.options.resize(26, 0);
        f.def.options[15] |= 1 << 3;
    }
    let on = 0x2_0000;
    let body = |h: &TestHost, revert: u32| {
        vec![
            rawp(sid::REVERT_ON_ERROR, &[], revert, 0),
            var_c(h, "$state", "Get(RevertTransactionOnErrorState)"),
            rawp(sid::OPEN_TRANSACTION, &[], 0, 0),
            set_field(NAME, "\"Xavier\""),
            plain(id::NEW_RECORD),
            // Name empty: 509.
            plain(id::COMMIT),
            set_field(NAME, "\"Zed\""),
            rawp(sid::COMMIT_TRANSACTION, &[], 0, 0),
            exit_c(h, "$state & Get(LastError) & Get(FoundCount)"),
        ]
    };
    let (_, o) = {
        let s = body(&h, on);
        go(&mut h, s)
    };
    // Reverted on the error: on after Commit Transaction.
    assert_eq!(o.result.to_text(), "15094");
    assert_eq!(h.db.tables[0].records.len(), 4);
    assert_eq!(h.db.tables[0].records[0].values[&NAME], "Ann");
    let (_, o) = {
        let s = body(&h, 0);
        go(&mut h, s)
    };
    assert_eq!(o.result.to_text(), "005");
    assert_eq!(h.db.tables[0].records.len(), 5);
    assert_eq!(h.db.tables[0].records[0].values[&NAME], "Xavier");
}
