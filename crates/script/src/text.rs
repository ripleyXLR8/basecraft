//! Script steps as FileMaker shows them in the Script Workspace:
//! `Set Field [ Contacts::Name ; "x" ]`, `If [ … ]`, `# comment`,
//! disabled steps prefixed with `//`, and the indentation of blocks.

use engine::Database;
use fmp12::scripts::{
    Calc, FieldRef, LayoutDest, RecordDest, Script, SortOrder, Step, StepParams, Target, id,
};

/// Names of fields, occurrences, layouts and scripts for step text, and
/// the reverse lookup for compiling calculations typed by the user.
pub struct Names<'a> {
    pub db: &'a Database,
    pub scripts: &'a [Script],
}

impl fmp12::calc::Names for Names<'_> {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        Some(self.db.table_of(to)?.field(field)?.name.clone())
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(
            self.db
                .occurrences
                .iter()
                .find(|o| o.id == to)?
                .name
                .clone(),
        )
    }
}

impl fmp12::calc_compile::Resolve for Names<'_> {
    fn field(&self, to: Option<&str>, name: &str) -> Option<(u32, u32)> {
        let o = self
            .db
            .occurrences
            .iter()
            .find(|o| to.is_some_and(|t| o.name.eq_ignore_ascii_case(t)))?;
        let f = self.db.table_of(o.id)?.field_by_name(name)?;
        Some((o.id, f.id))
    }
}

impl Names<'_> {
    /// Source text of a calculation (`<error>` if it does not decode).
    pub fn calc(&self, c: &Calc) -> String {
        c.text(self).unwrap_or_else(|_| "<error>".into())
    }

    pub fn field_ref(&self, f: &FieldRef) -> String {
        let to = fmp12::calc::Names::occurrence(self, f.to).unwrap_or_else(|| "?".into());
        let name = fmp12::calc::Names::field(self, f.to, f.field).unwrap_or_else(|| "?".into());
        if f.repetition > 1 {
            format!("{to}::{name}[{}]", f.repetition)
        } else {
            format!("{to}::{name}")
        }
    }

    pub fn layout(&self, id: u32) -> String {
        self.db
            .layouts
            .iter()
            .find(|l| l.id == id)
            .map_or_else(|| "<unknown>".into(), |l| l.name.clone())
    }

    pub fn script(&self, id: u32) -> String {
        self.scripts
            .iter()
            .find(|s| s.id == id)
            .map_or_else(|| "<unknown>".into(), |s| s.name.clone())
    }

    /// Compiles `src` (script context: field references name their TO).
    pub fn compile(&self, src: &str) -> Result<Calc, fmp12::calc_compile::CompileError> {
        fmp12::calc_compile::compile(src, 0, self).map(|c| Calc(c.tokens))
    }
}

fn on(b: bool) -> &'static str {
    if b { "On" } else { "Off" }
}

/// The step as one line of FileMaker step text (no indentation).
pub fn step_text(s: &Step, n: &Names) -> String {
    let name = s
        .name()
        .map_or_else(|| format!("<unknown> [{}]", s.id), str::to_string);
    let c = |c: &Option<Calc>| c.as_ref().map(|c| n.calc(c)).unwrap_or_default();
    let mut args: Vec<String> = Vec::new();
    match &s.params {
        StepParams::Comment(t) => {
            let t = if t.is_empty() {
                String::new()
            } else {
                format!("# {t}")
            };
            return if s.enabled { t } else { format!("// {t}") };
        }
        StepParams::Switch(b) => args.push(on(*b).into()),
        StepParams::Condition(x) => args.push(c(x)),
        StepParams::PerformScript { script, parameter } => {
            args.push(format!(
                "\u{201C}{}\u{201D}",
                script.map_or_else(|| "<unknown>".into(), |i| n.script(i))
            ));
            if parameter.is_some() {
                args.push(format!("Parameter: {}", c(parameter)));
            }
        }
        StepParams::ExitScript(x) => {
            if x.is_some() {
                args.push(format!("Text Result: {}", c(x)));
            }
        }
        StepParams::SetVariable {
            name,
            value,
            repetition,
        } => {
            let rep = repetition
                .as_ref()
                .map(|r| n.calc(r))
                .filter(|r| r != "1")
                .map(|r| format!("[{r}]"))
                .unwrap_or_default();
            args.push(format!("{name}{rep}"));
            args.push(format!("Value: {}", c(value)));
        }
        StepParams::SetField { field, value, .. } => {
            if let Some(f) = field {
                args.push(n.field_ref(f));
            }
            args.push(c(value));
        }
        StepParams::InsertText {
            target,
            text,
            select,
        } => {
            if *select {
                args.push("Select".into());
            }
            match target {
                Target::Field(f) => args.push(format!("Target: {}", n.field_ref(f))),
                Target::Variable(v) => args.push(format!("Target: {v}")),
                Target::None => {}
            }
            args.push(format!("\"{text}\""));
        }
        StepParams::GoToLayout { dest, .. } => args.push(match dest {
            LayoutDest::Original => "original layout".into(),
            LayoutDest::Layout(l) => format!("\u{201C}{}\u{201D}", n.layout(*l)),
            LayoutDest::NameByCalc(x) => format!("Name by calculation: {}", n.calc(x)),
            LayoutDest::NumberByCalc(x) => format!("Number by calculation: {}", n.calc(x)),
        }),
        StepParams::GoToRecord {
            dest,
            exit_after_last,
            dialog,
        } => {
            args.push(match dest {
                RecordDest::First => "First".into(),
                RecordDest::Last => "Last".into(),
                RecordDest::Previous => "Previous".into(),
                RecordDest::Next => "Next".into(),
                RecordDest::ByCalc(x) => format!("With dialog: {} ; {}", on(*dialog), n.calc(x)),
            });
            if matches!(dest, RecordDest::Previous | RecordDest::Next) {
                args.push(format!("Exit after last: {}", on(*exit_after_last)));
            }
        }
        StepParams::GoToField { field, select } => {
            if *select {
                args.push("Select/perform".into());
            }
            if let Some(f) = field {
                args.push(n.field_ref(f));
            }
        }
        StepParams::GoToObject { name, repetition } => {
            args.push(format!("Object Name: {}", c(name)));
            if repetition.is_some() {
                args.push(format!("Repetition: {}", c(repetition)));
            }
        }
        StepParams::ShowCustomDialog {
            title,
            message,
            input,
            ..
        } => {
            args.push(c(title));
            args.push(c(message));
            if let Some((f, _)) = input {
                args.push(n.field_ref(f));
            }
        }
        StepParams::Records { dialog, count } => {
            if s.id != id::NEW_RECORD && s.id != id::DUPLICATE_RECORD {
                args.push(format!("With dialog: {}", on(*dialog)));
            }
            if count.is_some() {
                args.push(c(count));
            }
        }
        StepParams::Commit {
            dialog,
            skip_validation,
            ..
        } => {
            if *skip_validation {
                args.push("Skip data entry validation".into());
            }
            args.push(format!("With dialog: {}", on(*dialog)));
        }
        StepParams::Find {
            pause,
            restore,
            requests,
        } => {
            if s.id == id::ENTER_FIND_MODE {
                args.push(format!("Pause: {}", on(*pause)));
            }
            if *restore && !requests.is_empty() {
                args.push("Restore".into());
            }
        }
        StepParams::Mode { pause } => args.push(format!("Pause: {}", on(*pause))),
        StepParams::Sort {
            dialog,
            restore,
            keys,
        } => {
            if *restore && !keys.is_empty() {
                args.push("Restore".into());
            }
            args.push(format!("With dialog: {}", on(*dialog)));
        }
        StepParams::RefreshWindow { flush_joins, .. } => {
            if *flush_joins {
                args.push("Flush cached join results".into());
            }
        }
        StepParams::Pause(d) => args.push(match d {
            Some(d) => format!("Duration (seconds): {}", n.calc(d)),
            None => "Indefinitely".into(),
        }),
        StepParams::None | StepParams::Other => {}
    }
    let text = if args.is_empty() {
        name
    } else {
        format!("{name} [ {} ]", args.join(" ; "))
    };
    if s.enabled {
        text
    } else {
        format!("// {text}")
    }
}

/// Indentation level of each step: one more inside If/Else If/Else and
/// Loop blocks.
pub fn indents(steps: &[Step]) -> Vec<usize> {
    let mut depth: usize = 0;
    steps
        .iter()
        .map(|s| match s.id {
            id::ELSE_IF | id::ELSE => depth.saturating_sub(1),
            id::END_IF | id::END_LOOP => {
                depth = depth.saturating_sub(1);
                depth
            }
            id::IF | id::LOOP => {
                depth += 1;
                depth - 1
            }
            _ => depth,
        })
        .collect()
}

/// Sort keys as text, for the Sort Records editor.
pub fn sort_key_text(k: &(FieldRef, SortOrder), n: &Names) -> String {
    let o = match k.1 {
        SortOrder::Ascending => "ascending",
        SortOrder::Descending => "descending",
        SortOrder::Custom(_) => "custom",
    };
    format!("{} ({o})", n.field_ref(&k.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation() {
        let st = |i: u16| Step {
            id: i,
            enabled: true,
            ..Default::default()
        };
        let steps: Vec<Step> = [
            id::IF,
            id::BEEP,
            id::ELSE_IF,
            id::LOOP,
            id::BEEP,
            id::END_LOOP,
            id::ELSE,
            id::END_IF,
            id::BEEP,
        ]
        .into_iter()
        .map(st)
        .collect();
        assert_eq!(indents(&steps), [0, 1, 0, 1, 2, 1, 0, 0, 0]);
    }
}
