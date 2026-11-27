//! Reads back a file FileMaker changed (`tools/oracle/security_check.ps1`):
//! accounts, whether their digests match, whether the given test
//! passwords are theirs, and the stored form of the account script steps.
//!
//!   cargo run -p fmp12 --example secprobe -- <file.fmp12> [name:password …]

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fmp12::security::Security;

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = fmp12::File::open(&args[1]).unwrap();
    let sec = Security::load(&file).unwrap();
    println!("digest mismatches: {:?}", Security::check(&file).unwrap());
    for a in &sec.accounts {
        println!(
            "account {} {:?} set={} active={} must_change={} stamp={:?}",
            a.id, a.name, a.privilege_set, a.active, a.must_change, a.stamp
        );
    }
    for pair in &args[2..] {
        let (n, p) = pair.split_once(':').unwrap();
        let ok = sec.account_named(n).map(|a| a.check_password(p));
        println!("password {n}:{p} -> {ok:?}");
    }
    for s in fmp12::scripts::scripts(&file).unwrap() {
        for st in &s.steps {
            if matches!(st.id, 83 | 134..=138) {
                println!(
                    "step {} options={:#x} raw={}",
                    st.id,
                    st.options,
                    hex(&st.raw)
                );
                for ((p, k), v) in &st.raw_params.kv {
                    println!("    {p:?} k{k} = {}", hex(v));
                }
            }
        }
    }
}
