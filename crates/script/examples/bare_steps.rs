//! Adds a script "BC Steps" to a copy of a file, for FileMaker's XML export
//! (`tools/oracle/fms_check.sh out.fmp12 Admin "" out.xml`) or to run it in
//! FileMaker Pro. Used to decode step parameters by writing candidates.
//! `cargo run -p script --example bare_steps -- <in.fmp12> <out.fmp12> <spec>`
//! `spec` is a list of ids (`188,189`: new steps with no parameters) or a
//! file of lines:
//!   `step <id> [opt=<hex>] [arg=<n>] [inline=<hex, bytes 6–19>]`
//!   `calc <p> <calculation>` → `[128 + p].[5]`
//!   `text <p> <text>` → `[128 + p]` key 1
//!   `kv <path.dotted> <key> <hex>` → a raw key of the step's tree
//!   `script <name>` → the next lines go to a new script
//! applying to the step above.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::calc::{FileNames, compile};
use fmp12::scripts::{Calc, Script, Step, scripts};
use fmp12::scripts_save::save_scripts;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = fmp12::File::open(&a[0]).unwrap();
    let names = FileNames::load(&file).unwrap();
    let mut list = scripts(&file).unwrap();
    let id = list.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    // (name, steps) of each script added; `script <name>` starts the next
    // one (ids follow each other: a step can call the next one).
    let mut parts: Vec<(String, Vec<Step>)> = vec![("BC Steps".into(), Vec::new())];
    match std::fs::read_to_string(&a[2]) {
        Err(_) => {
            parts[0].1 = a[2]
                .split(',')
                .map(|i| Step::new(i.trim().parse().unwrap()))
                .collect()
        }
        Ok(spec) => {
            for line in spec.lines().map(str::trim) {
                let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
                let steps = &mut parts.last_mut().unwrap().1;
                match cmd {
                    "script" => parts.push((rest.into(), Vec::new())),
                    "step" => {
                        let mut w = rest.split_whitespace();
                        let mut s = Step::new(w.next().unwrap().parse().unwrap());
                        s.raw = vec![0; 28];
                        // Raw parameters only: typed ones would overwrite them.
                        s.params = fmp12::scripts::StepParams::Other;
                        for kv in w {
                            let (k, v) = kv.split_once('=').unwrap();
                            match k {
                                "opt" => s.options = u32::from_str_radix(v, 16).unwrap(),
                                "arg" => s.arg = v.parse().unwrap(),
                                "inline" => {
                                    let b = hex(v);
                                    s.raw[6..6 + b.len()].copy_from_slice(&b);
                                }
                                _ => panic!("{k}"),
                            }
                        }
                        steps.push(s);
                    }
                    "calc" => {
                        let (p, src) = rest.split_once(' ').unwrap();
                        let c = Calc(compile(src, 0, &names).unwrap().tokens);
                        let s = steps.last_mut().unwrap();
                        s.raw_params.set_calc(p.parse().unwrap(), Some(&c));
                    }
                    "text" => {
                        let (p, t) = rest.split_once(' ').unwrap();
                        let s = steps.last_mut().unwrap();
                        s.raw_params.set_text(p.parse().unwrap(), Some(t));
                    }
                    "path" => {
                        // `path <p> <kind> <path>`: a path list in key 130.
                        let w: Vec<&str> = rest.splitn(3, ' ').collect();
                        let kind = w.get(1).copied().unwrap_or("");
                        let path = w.get(2).copied().unwrap_or("");
                        let full = if kind.is_empty() {
                            path.to_string()
                        } else {
                            format!("{kind}:{path}")
                        };
                        let b = script::files::encode_path_list(&[&full]);
                        let p: u32 = w[0].parse().unwrap();
                        steps.last_mut().unwrap().raw_params.put(&[128 + p], 130, b);
                    }
                    "kv" => {
                        let w: Vec<&str> = rest.split_whitespace().collect();
                        let path: Vec<u32> = w[0]
                            .split('.')
                            .filter(|x| !x.is_empty())
                            .map(|x| x.parse().unwrap())
                            .collect();
                        let s = steps.last_mut().unwrap();
                        s.raw_params.put(&path, w[1].parse().unwrap(), hex(w[2]));
                    }
                    _ => {}
                }
            }
        }
    }
    for (i, (name, steps)) in parts.into_iter().enumerate() {
        list.push(Script {
            id: id + i as u32,
            name,
            options: 8,
            steps,
        });
    }
    std::fs::write(&a[1], save_scripts(&file, &list).unwrap()).unwrap();
}
