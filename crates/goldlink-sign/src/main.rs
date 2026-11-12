//! The maintainer's release signing (dev/docs/unified-client/UC10_UPDATER.md).
//!
//!     goldlink-sign keygen <key file>             a new key (PKCS#8, mode 600); prints its public key
//!     goldlink-sign public <key file>             prints the key's public key (hex)
//!     goldlink-sign sign <key file> <tag> <SHA256SUMS>   prints the signature (hex)
//!     goldlink-sign verify <public hex> <tag> <SHA256SUMS> <signature file>
//!
//! The key file never goes into a repository; back it up.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["keygen", file] => keygen(file),
        ["public", file] => read(file)
            .and_then(|k| goldlink_sign::public_key(&k))
            .map(|p| println!("{}", hex::encode(p))),
        ["sign", file, tag, sums] => (|| {
            let sig = goldlink_sign::sign(&read(file)?, tag, &read(sums)?)?;
            println!("{sig}");
            Ok(())
        })(),
        ["verify", public, tag, sums, sig] => (|| {
            let key: [u8; 32] = hex::decode(public)
                .ok()
                .and_then(|k| k.try_into().ok())
                .ok_or("the public key is 64 hex digits")?;
            let sig = String::from_utf8(read(sig)?).map_err(|_| "the signature isn't text")?;
            goldlink_sign::verify(&key, tag, &read(sums)?, &sig)?;
            println!("good signature");
            Ok(())
        })(),
        _ => {
            eprintln!(
                "usage: goldlink-sign keygen <key> | public <key> | sign <key> <tag> <SHA256SUMS> | verify <public> <tag> <SHA256SUMS> <sig>"
            );
            return ExitCode::from(64);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("goldlink-sign: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn keygen(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    if p.exists() {
        return Err(format!(
            "{path} exists already: a new key would lock out every installed GoldLink"
        ));
    }
    if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let (pkcs8, public) = goldlink_sign::generate()?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    use std::io::Write;
    options
        .open(p)
        .and_then(|mut f| f.write_all(&pkcs8))
        .map_err(|e| format!("{path}: {e}"))?;
    println!("{}", hex::encode(public));
    Ok(())
}
