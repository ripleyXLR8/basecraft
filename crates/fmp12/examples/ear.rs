//! Encrypts or decrypts a file (test files for FMDeveloperTool, FORMAT.md
//! §17):
//!
//! ```text
//! cargo run -p fmp12 --example ear -- in.fmp12 out.fmp12 password [hint] [shared id]
//! cargo run -p fmp12 --example ear -- --decrypt in.fmp12 out.fmp12 password
//! ```
//!
//! An encrypted input is opened with the same password and re-encrypted.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let decrypt = a.first().is_some_and(|s| s == "--decrypt");
    if decrypt {
        a.remove(0);
    }
    let [inp, out, pw, rest @ ..] = a.as_slice() else {
        return Err("usage: ear [--decrypt] in.fmp12 out.fmp12 password [hint] [shared id]".into());
    };
    let f = fmp12::File::open_encrypted(inp, pw)?;
    if decrypt {
        std::fs::write(out, f.bytes())?;
        return Ok(());
    }
    let enc = match f.encryption() {
        Some(e) => e.clone(),
        None => fmp12::ear::Encryption::new(
            pw,
            rest.first().map_or("", String::as_str),
            rest.get(1).map_or("", String::as_str),
        )?,
    };
    std::fs::write(out, fmp12::ear::encrypt(f.bytes(), &enc)?)?;
    Ok(())
}
