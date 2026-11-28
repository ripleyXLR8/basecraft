//! Save a Copy as XML: the file's structure in FileMaker's Save as XML
//! grammar (`FMSaveAsXML` 2.3.0.0, the element and attribute names of
//! FileMaker Pro 26's own exports, `tools/oracle`): base tables, table
//! occurrences, relationships, custom value lists, fields (type, data
//! type, global, repetitions, calculation text), the script catalog,
//! layouts (table occurrence, width, parts, objects with their kind, type,
//! name, bounds, field reference and text) and the steps of each script
//! (id, name, enabled, options).
//!
//! Left out (see docs/COVERAGE.md): object UUIDs and modification stamps,
//! `DDR_INFO` (step text, calculation chunks) and `Metadata`, themes,
//! custom menus, accounts and privilege sets, step parameters, auto-enter
//! and validation options, value lists from fields, objects inside
//! portals, tab and slide panels, button bars and groups, and layout
//! object styles.

use std::fmt::Write as _;

use super::*;

/// XML attribute text.
fn attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\r' => o.push_str("&#13;"),
            '\n' => o.push_str("&#10;"),
            c => o.push(c),
        }
    }
    o
}

/// Element text as FileMaker writes it: CDATA.
fn cdata(s: &str) -> String {
    format!("<![CDATA[{}]]>", s.replace("]]>", "]]]]><![CDATA[>"))
}

/// Occurrence id as the XML shows it.
fn to_id(id: u32) -> u32 {
    0x104000 | id
}

fn data_type(dt: fmp12::schema::DataType) -> &'static str {
    use fmp12::schema::DataType::*;
    match dt {
        Text | Unknown(_) => "Text",
        Number => "Number",
        Date => "Date",
        Time => "Time",
        Timestamp => "Timestamp",
        Container => "Binary",
    }
}

fn part_type(kind: u64) -> &'static str {
    use fmp12::layouts::part::*;
    match kind {
        HEADER => "Header",
        LEADING_GRAND_SUMMARY => "Leading Grand Summary",
        LEADING_SUB_SUMMARY => "Leading Sub-summary",
        BODY => "Body",
        TRAILING_SUB_SUMMARY => "Trailing Sub-summary",
        TRAILING_GRAND_SUMMARY => "Trailing Grand Summary",
        FOOTER => "Footer",
        TITLE_HEADER => "Title Header",
        TITLE_FOOTER => "Title Footer",
        TOP_NAVIGATION => "Top Navigation",
        BOTTOM_NAVIGATION => "Bottom Navigation",
        _ => "Body",
    }
}

/// The object type FileMaker names in the XML (a field by its control
/// style).
fn object_type(o: &fmp12::layouts::LayoutObject, container: bool) -> &'static str {
    match o.kind {
        1 if container => "Container",
        1 => match o.display {
            1 => "Drop-down List",
            2 => "Pop-up Menu",
            3 => "Checkbox Set",
            4 => "Radio Button Set",
            6 => "Drop-down Calendar",
            _ => "Edit Box",
        },
        2 => "Text",
        3 => "Graphic",
        4 => "Line",
        5 => "Rectangle",
        6 => "Rounded Rectangle",
        7 => "Oval",
        8 => "Group",
        9 => "Portal",
        10 => "Button",
        11 => "Tab Control",
        12 => "Panel",
        13 => "Web Viewer",
        14 => "Popover Button",
        15 => "Popover",
        16 => "Slide Control",
        17 => "Slide Panel",
        18 => "Button Bar",
        _ => "Unknown",
    }
}

fn to_ref(db: &Database, id: u32) -> String {
    let name = db
        .occurrences
        .iter()
        .find(|o| o.id == id)
        .map_or("", |o| o.name.as_str());
    format!(
        "<TableOccurrenceReference id=\"{}\" name=\"{}\"></TableOccurrenceReference>",
        to_id(id),
        attr(name)
    )
}

fn field_ref(db: &Database, to: u32, field: u32, rep: Option<u64>) -> String {
    let name = db
        .table_of(to)
        .and_then(|t| t.field(field))
        .map_or("", |f| f.name.as_str());
    let rep = rep.map_or(String::new(), |r| format!(" repetition=\"{r}\""));
    format!(
        "<FieldReference id=\"{field}\" name=\"{}\"{rep}>{}</FieldReference>",
        attr(name),
        to_ref(db, to)
    )
}

/// The engine's calculation source (`${Field}`) as FileMaker shows it.
fn calc_text(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find('}') {
            Some(j) => {
                out.push_str(&rest[i + 2..i + 2 + j]);
                rest = &rest[i + 3 + j..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The XML of the whole file.
pub fn save_as_xml(db: &Database, scripts: &[Script], file: &str) -> String {
    let mut x = String::new();
    let w = &mut x;
    let _ = writeln!(w, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        w,
        "<FMSaveAsXML version=\"2.3.0.0\" Source=\"26.0.1\" File=\"{}.fmp12\" locale=\"English\" Has_DDR_INFO=\"False\">",
        attr(file)
    );
    let _ = writeln!(w, "\t<Structure membercount=\"1\">");
    let _ = writeln!(w, "\t\t<AddAction membercount=\"8\">");

    let _ = writeln!(
        w,
        "\t\t\t<BaseTableCatalog membercount=\"{}\">",
        db.tables.len()
    );
    for t in &db.tables {
        let _ = writeln!(
            w,
            "\t\t\t\t<BaseTable id=\"{}\" comment=\"\" name=\"{}\"></BaseTable>",
            t.id,
            attr(&t.name)
        );
    }
    let _ = writeln!(w, "\t\t\t</BaseTableCatalog>");

    let _ = writeln!(
        w,
        "\t\t\t<TableOccurrenceCatalog membercount=\"{}\">",
        db.occurrences.len()
    );
    for o in &db.occurrences {
        let base = db.table(o.base_table).map_or("", |t| t.name.as_str());
        let _ = writeln!(
            w,
            "\t\t\t\t<TableOccurrence id=\"{}\" name=\"{}\" type=\"Local\"><BaseTableSourceReference type=\"BaseTableReference\"><BaseTableReference id=\"{}\" name=\"{}\"></BaseTableReference></BaseTableSourceReference></TableOccurrence>",
            to_id(o.id),
            attr(&o.name),
            o.base_table,
            attr(base)
        );
    }
    let _ = writeln!(w, "\t\t\t</TableOccurrenceCatalog>");

    let _ = writeln!(
        w,
        "\t\t\t<RelationshipCatalog membercount=\"{}\">",
        db.relationships.len()
    );
    for (i, r) in db.relationships.iter().enumerate() {
        let id = if r.id == 0 { i as u32 + 1 } else { r.id };
        let _ = writeln!(w, "\t\t\t\t<Relationship id=\"{id}\">");
        let _ = writeln!(
            w,
            "\t\t\t\t\t<LeftTable type=\"Local\">{}</LeftTable>",
            to_ref(db, r.left)
        );
        let _ = writeln!(
            w,
            "\t\t\t\t\t<RightTable type=\"Local\">{}</RightTable>",
            to_ref(db, r.right)
        );
        let _ = writeln!(
            w,
            "\t\t\t\t\t<JoinPredicateList membercount=\"{}\">",
            r.predicates.len()
        );
        for p in &r.predicates {
            use fmp12::graph::JoinOp::*;
            let op = match p.op {
                Equal | Unknown(_) => "Equal",
                NotEqual => "NotEqual",
                Less => "Less",
                LessOrEqual => "LessOrEqual",
                Greater => "Greater",
                GreaterOrEqual => "GreaterOrEqual",
                Cartesian => "CartesianProduct",
            };
            let _ = writeln!(
                w,
                "\t\t\t\t\t\t<JoinPredicate type=\"{op}\"><LeftField>{}</LeftField><RightField>{}</RightField></JoinPredicate>",
                field_ref(db, r.left, p.left_field, None),
                field_ref(db, r.right, p.right_field, None)
            );
        }
        let _ = writeln!(w, "\t\t\t\t\t</JoinPredicateList>");
        let _ = writeln!(w, "\t\t\t\t</Relationship>");
    }
    let _ = writeln!(w, "\t\t\t</RelationshipCatalog>");

    let _ = writeln!(
        w,
        "\t\t\t<ValueListCatalog membercount=\"{}\">",
        db.value_lists.len()
    );
    for v in &db.value_lists {
        let _ = write!(
            w,
            "\t\t\t\t<ValueList id=\"{}\" name=\"{}\">",
            v.id,
            attr(&v.name)
        );
        match &v.source {
            fmp12::valuelists::Source::Custom(text) => {
                let _ = write!(
                    w,
                    "<Source value=\"Custom\"></Source><CustomValues><Text>{}</Text></CustomValues>",
                    cdata(text)
                );
            }
            _ => {
                let _ = write!(w, "<Source value=\"Field\"></Source>");
            }
        }
        let _ = writeln!(w, "</ValueList>");
    }
    let _ = writeln!(w, "\t\t\t</ValueListCatalog>");

    let _ = writeln!(
        w,
        "\t\t\t<FieldsForTables membercount=\"{}\">",
        db.tables.len()
    );
    for t in &db.tables {
        let _ = writeln!(w, "\t\t\t\t<FieldCatalog>");
        let _ = writeln!(
            w,
            "\t\t\t\t\t<BaseTableReference id=\"{}\" name=\"{}\"></BaseTableReference>",
            t.id,
            attr(&t.name)
        );
        let _ = writeln!(
            w,
            "\t\t\t\t\t<ObjectList membercount=\"{}\">",
            t.fields.len()
        );
        for f in &t.fields {
            let kind = match f.kind {
                fmp12::schema::FieldKind::Calculated => "Calculated",
                fmp12::schema::FieldKind::Summary => "Summary",
                _ => "Normal",
            };
            let _ = write!(
                w,
                "\t\t\t\t\t\t<Field id=\"{}\" name=\"{}\" fieldtype=\"{kind}\" datatype=\"{}\" comment=\"{}\">",
                f.id,
                attr(&f.name),
                data_type(f.data_type),
                attr(&f.def.comment)
            );
            let _ = write!(
                w,
                "<Storage global=\"{}\" maxRepetitions=\"{}\"></Storage>",
                if f.def.global() { "True" } else { "False" },
                engine::repetitions(f)
            );
            if let Some(c) = &f.calc {
                let _ = write!(
                    w,
                    "<Calculation><Text>{}</Text></Calculation>",
                    cdata(&calc_text(c))
                );
            }
            let _ = writeln!(w, "</Field>");
        }
        let _ = writeln!(w, "\t\t\t\t\t</ObjectList>");
        let _ = writeln!(w, "\t\t\t\t</FieldCatalog>");
    }
    let _ = writeln!(w, "\t\t\t</FieldsForTables>");

    let _ = writeln!(w, "\t\t\t<ScriptCatalog membercount=\"{}\">", scripts.len());
    for s in scripts {
        let folder = if s.is_folder() {
            " isFolder=\"True\""
        } else if s.options & 2 != 0 {
            " isFolder=\"Marker\""
        } else {
            ""
        };
        let _ = writeln!(
            w,
            "\t\t\t\t<Script id=\"{}\" name=\"{}\"{folder}><Options>{}</Options></Script>",
            s.id,
            attr(&s.name),
            s.options
        );
    }
    let _ = writeln!(w, "\t\t\t</ScriptCatalog>");

    let _ = writeln!(
        w,
        "\t\t\t<LayoutCatalog membercount=\"{}\">",
        db.layouts.len()
    );
    for l in &db.layouts {
        layout(w, db, l);
    }
    let _ = writeln!(w, "\t\t\t</LayoutCatalog>");

    let with_steps: Vec<&Script> = scripts.iter().filter(|s| !s.steps.is_empty()).collect();
    let _ = writeln!(
        w,
        "\t\t\t<StepsForScripts membercount=\"{}\">",
        with_steps.len()
    );
    for s in with_steps {
        let _ = writeln!(w, "\t\t\t\t<Script>");
        let _ = writeln!(
            w,
            "\t\t\t\t\t<ScriptReference id=\"{}\" name=\"{}\"></ScriptReference>",
            s.id,
            attr(&s.name)
        );
        let _ = writeln!(
            w,
            "\t\t\t\t\t<ObjectList membercount=\"{}\">",
            s.steps.len()
        );
        for (i, st) in s.steps.iter().enumerate() {
            let name = st.name().unwrap_or("<unknown>");
            let _ = writeln!(
                w,
                "\t\t\t\t\t\t<Step index=\"{i}\" id=\"{}\" name=\"{}\" enable=\"{}\"><Options>{}</Options></Step>",
                st.id,
                attr(name),
                if st.enabled { "True" } else { "False" },
                st.options
            );
        }
        let _ = writeln!(w, "\t\t\t\t\t</ObjectList>");
        let _ = writeln!(w, "\t\t\t\t</Script>");
    }
    let _ = writeln!(w, "\t\t\t</StepsForScripts>");
    let _ = writeln!(w, "\t\t</AddAction>");
    let _ = writeln!(w, "\t</Structure>");
    let _ = writeln!(w, "</FMSaveAsXML>");
    x
}

fn layout(w: &mut String, db: &Database, l: &fmp12::layouts::Layout) {
    use fmp12::layouts::setup;
    if l.flags & setup::FOLDER != 0 {
        let _ = writeln!(
            w,
            "\t\t\t\t<Layout id=\"{}\" name=\"{}\" isFolder=\"True\"></Layout>",
            l.id,
            attr(&l.name)
        );
        return;
    }
    let _ = writeln!(
        w,
        "\t\t\t\t<Layout id=\"{}\" name=\"{}\" width=\"{}\">",
        l.id,
        attr(&l.name),
        l.width
    );
    if l.table_occurrence != 0 {
        let _ = writeln!(
            w,
            "\t\t\t\t\t{}",
            to_ref(db, (l.table_occurrence & 0xFFF) as u32)
        );
    }
    let _ = writeln!(w, "\t\t\t\t\t<PartsList membercount=\"{}\">", l.parts.len());
    let top_level: Vec<&fmp12::layouts::LayoutObject> =
        l.objects.iter().filter(|o| o.parent.is_none()).collect();
    for (i, p) in l.parts.iter().enumerate() {
        let ty = part_type(p.kind);
        let _ = writeln!(w, "\t\t\t\t\t\t<Part type=\"{ty}\" kind=\"{}\">", p.kind);
        let _ = writeln!(
            w,
            "\t\t\t\t\t\t\t<Definition type=\"{ty}\" kind=\"{}\" size=\"{}\" absolute=\"{}\"></Definition>",
            p.kind, p.height, p.top
        );
        // An object belongs to the part its top edge is in.
        let last = i + 1 == l.parts.len();
        let inside: Vec<_> = top_level
            .iter()
            .filter(|o| {
                o.bounds.top >= p.top && (last || o.bounds.top < p.top + p.height)
                    || (i == 0 && o.bounds.top < p.top)
            })
            .collect();
        let _ = writeln!(
            w,
            "\t\t\t\t\t\t\t<ObjectList membercount=\"{}\">",
            inside.len()
        );
        for o in inside {
            object(w, db, o);
        }
        let _ = writeln!(w, "\t\t\t\t\t\t\t</ObjectList>");
        let _ = writeln!(w, "\t\t\t\t\t\t</Part>");
    }
    let _ = writeln!(w, "\t\t\t\t\t</PartsList>");
    let _ = writeln!(w, "\t\t\t\t</Layout>");
}

fn object(w: &mut String, db: &Database, o: &fmp12::layouts::LayoutObject) {
    let field = o
        .field
        .map(|(to, f, rep)| ((to & 0xFFF) as u32, f as u32, rep));
    let container = field.is_some_and(|(to, f, _)| {
        db.table_of(to)
            .and_then(|t| t.field(f))
            .is_some_and(|x| x.data_type == fmp12::schema::DataType::Container)
    });
    let b = &o.bounds;
    let _ = write!(
        w,
        "\t\t\t\t\t\t\t\t<LayoutObject id=\"{}\" type=\"{}\" name=\"{}\" kind=\"{}\"><Bounds top=\"{}\" left=\"{}\" bottom=\"{}\" right=\"{}\"></Bounds><Options>{}</Options>",
        o.id,
        object_type(o, container),
        attr(&o.name),
        o.kind,
        b.top,
        b.left,
        b.bottom,
        b.right,
        o.options
    );
    if let Some((to, f, rep)) = field {
        let _ = write!(
            w,
            "<Field>{}</Field>",
            field_ref(db, to, f, Some(rep.max(1)))
        );
    }
    if let (2, Some(t)) = (o.kind, &o.text) {
        let _ = write!(
            w,
            "<Text><StyledText><Data>{}</Data></StyledText></Text>",
            cdata(t)
        );
    }
    let _ = writeln!(w, "</LayoutObject>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculations_and_escapes() {
        assert_eq!(
            calc_text("${First} & \" \" & ${Other::Last}"),
            "First & \" \" & Other::Last"
        );
        assert_eq!(attr("a<\"&>"), "a&lt;&quot;&amp;&gt;");
        assert_eq!(cdata("x]]>y"), "<![CDATA[x]]]]><![CDATA[>y]]>");
    }
}
