//! Script edits for checking `scripts_save` in FileMaker
//! (`tools/oracle/script_check.ps1`).
//! `cargo run -p fmp12 --example scriptedit -- <Tasks en.fmp12> <out.fmp12> [steps]`
//! On a copy of the Tasks template: edits "Choose Assignee" (a Set Field
//! calculation, a disabled step, a Show Custom Dialog at the end), deletes
//! "Go to Entry Field", adds "BC Flow" (If/Else/End If, Set Variable, Go to
//! Layout, Perform Script) and "BC Calc" (Let, nested Let, `$v[2]`, Case,
//! Choose, List; the result in a dialog). `steps` picks the edits
//! (default `edit,delete,flow,calc`; `none` = an unchanged save). Also:
//! `call` (Sort by Status, run by Task List's OnLayoutEnter, performs BC
//! Calc), `rename:Old:New`, `drop:Name`, `beep` (a new script named by the
//! 4th argument). Prints the calculations written.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::calc::{FileNames, compile};
use fmp12::scripts::{Calc, DialogButton, LayoutDest, Script, Step, StepParams, id, scripts};
use fmp12::scripts_save::save_scripts;

/// The calculation of the BC Calc dialog.
const CALC: &str = "Let ( [ a = 1 ; b = Let ( c = a + 1 ; c * 10 ) ] ; a + b ) & \"|\" & $v[2] & \"|\" & Case ( $v[2] = \"two\" ; \"yes\" ; \"no\" ) & \"|\" & Choose ( 2 ; \"zero\" ; \"one\" ; \"two\" ) & \"|\" & List ( \"x\" ; \"y\" )";

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = fmp12::File::open(&a[0]).unwrap();
    let ops = a.get(2).map_or("edit,delete,flow,calc", |s| s.as_str());
    let on = |op: &str| ops.split(',').any(|o| o == op);
    let names = FileNames::load(&file).unwrap();
    let calc = |s: &str| {
        println!("{s}");
        Some(Calc(compile(s, 0, &names).unwrap().tokens))
    };
    let dialog = |title: &str, message: &str| {
        let mut d = Step::new(id::SHOW_CUSTOM_DIALOG);
        d.params = StepParams::ShowCustomDialog {
            title: calc(title),
            message: calc(message),
            buttons: vec![DialogButton {
                label: calc("\"OK\""),
                commit: true,
            }],
            input: None,
        };
        d
    };
    let set_var = |name: &str, value: &str, rep: Option<&str>| {
        let mut s = Step::new(id::SET_VARIABLE);
        s.params = StepParams::SetVariable {
            name: name.into(),
            value: calc(value),
            repetition: rep.and_then(calc),
        };
        s
    };
    let mut list = scripts(&file).unwrap();
    let next = list.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    // `inplace`: the new scripts' steps go into existing scripts instead.
    let inplace = on("inplace");
    let id_of = |l: &[Script], n: &str| l.iter().find(|s| s.name == n).map(|s| s.id).unwrap();
    let (flow_id, calc_id) = if inplace {
        (id_of(&list, "Sort by Name"), id_of(&list, "Sort by Status"))
    } else {
        (next, next + 1)
    };
    let place = |l: &mut Vec<Script>, id: u32, name: &str, steps: Vec<Step>| match l
        .iter_mut()
        .find(|s| s.id == id)
    {
        Some(s) => s.steps = steps,
        None => l.push(Script {
            id,
            name: name.into(),
            options: 8,
            steps,
        }),
    };

    if on("edit") {
        let s = list
            .iter_mut()
            .find(|s| s.name == "Choose Assignee")
            .unwrap();
        let sf = s.steps.iter_mut().find(|s| s.id == id::SET_FIELD).unwrap();
        if let StepParams::SetField { value, .. } = &mut sf.params {
            *value = calc("\"BaseCraft \" & Get ( ScriptName )");
        }
        s.steps
            .iter_mut()
            .find(|s| s.id == id::CLOSE_WINDOW)
            .unwrap()
            .enabled = false;
        s.steps.push(dialog(
            "\"Edited\"",
            "\"Choose Assignee edited by BaseCraft\"",
        ));
    }
    if on("delete") {
        list.retain(|s| s.name != "Go to Entry Field");
    }
    if on("flow") {
        let mut if_ = Step::new(id::IF);
        if_.params = StepParams::Condition(calc("Get ( FoundCount ) > 0"));
        let mut go = Step::new(id::GO_TO_LAYOUT);
        go.params = StepParams::GoToLayout {
            dest: LayoutDest::Layout(2),
            animation: 0,
        };
        let mut perform = Step::new(id::PERFORM_SCRIPT);
        perform.params = StepParams::PerformScript {
            script: Some(calc_id),
            parameter: calc("\"from BC Flow\""),
        };
        place(
            &mut list,
            flow_id,
            "BC Flow",
            vec![
                if_,
                set_var("$found", "Get ( FoundCount ) & \" records\"", None),
                Step::new(id::ELSE),
                set_var("$found", "\"none\"", None),
                Step::new(id::END_IF),
                go,
                perform,
            ],
        );
    }
    if on("calc") {
        place(
            &mut list,
            calc_id,
            "BC Calc",
            vec![
                set_var("$v", "\"two\"", Some("2")),
                set_var("$r", CALC, None),
                dialog("\"BC Calc\"", "$r"),
            ],
        );
    }
    if on("call") {
        let mut perform = Step::new(id::PERFORM_SCRIPT);
        perform.params = StepParams::PerformScript {
            script: Some(calc_id),
            parameter: None,
        };
        let s = list
            .iter_mut()
            .find(|s| s.name == "Sort by Status")
            .unwrap();
        s.steps = vec![perform];
    }
    for op in ops.split(',') {
        let parts: Vec<&str> = op.split(':').collect();
        match parts.as_slice() {
            ["rename", old, new] => {
                list.iter_mut().find(|s| s.name == *old).unwrap().name = (*new).into()
            }
            ["drop", name] => list.retain(|s| s.name != *name),
            _ => {}
        }
    }
    if on("clear") {
        list.clear();
    }
    if on("beep") {
        // As FileMaker's Script Workspace › New Script + Beep (French UI).
        list.push(Script {
            id: next,
            name: a.get(3).map_or("Nouveau script", |s| s.as_str()).into(),
            options: 8,
            // `say`: a dialog instead, to see that the script runs.
            steps: vec![if on("say") {
                dialog("\"BC\"", "Get ( ScriptName ) & \" ran\"")
            } else {
                Step::new(id::BEEP)
            }],
        });
    }
    std::fs::write(&a[1], save_scripts(&file, &list).unwrap()).unwrap();
}
