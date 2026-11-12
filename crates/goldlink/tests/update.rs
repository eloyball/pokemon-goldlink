//! The launcher's update (UC10) end to end against a local server standing in for GitHub: the
//! releases list, a release signed with a key made here, its download; installed into a scratch
//! program folder. Then the same release with its files tampered with, or signed by another key:
//! refused, the folder untouched.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use goldlink::update::{self, Stage};

/// A tiny HTTP server: GET <path> → its bytes (404 otherwise). Returns its address.
fn serve(files: Arc<Mutex<HashMap<String, Vec<u8>>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let files = files.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                reader.read_line(&mut request).ok();
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let path = path.split('?').next().unwrap_or("").to_owned();
                let body = files.lock().unwrap().get(&path).cloned();
                let mut out = &stream;
                match body {
                    Some(b) => {
                        write!(
                            out,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            b.len()
                        )
                        .ok();
                        out.write_all(&b).ok();
                    }
                    None => {
                        write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").ok();
                    }
                }
            });
        }
    });
    addr
}

fn archive(tag: &str, program: &[u8]) -> (String, Vec<u8>) {
    let top = format!("GoldLink-{tag}");
    let entries: [(String, &[u8]); 3] = [
        (format!("{top}/{}", update::program_name()), program),
        (format!("{top}/release.json"), br#"{"version": "new"}"#),
        (format!("{top}/LICENSES/COPYING"), b"GPL"),
    ];
    if cfg!(windows) {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, data) in &entries {
            z.start_file(path.as_str(), zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data).unwrap();
        }
        (
            format!("GoldLink-{tag}-windows.zip"),
            z.finish().unwrap().into_inner(),
        )
    } else {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, data) in &entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            b.append_data(&mut h, path, *data).unwrap();
        }
        (
            format!("GoldLink-{tag}-linux.tar.gz"),
            b.into_inner().unwrap().finish().unwrap(),
        )
    }
}

fn hex_sha256(data: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(data))
}

fn scratch_program_folder() -> PathBuf {
    let d = std::env::temp_dir().join(format!("goldlink-update-e2e-{}", std::process::id()));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(update::program_name()), b"the old program").unwrap();
    std::fs::write(d.join("release.json"), br#"{"version": "0.6.1"}"#).unwrap();
    d
}

fn run(addr: &str, dir: &Path, key: &[u8; 32]) -> Result<PathBuf, String> {
    let found = update::find_at(addr, "owner/repo", "0.6.1").ok_or("no newer release found")?;
    assert_eq!(found.tag, "v0.6.2");
    assert!(found.installable(), "{found:?}");
    let (tx, rx) = channel();
    let result = update::install_release(&found, dir, key, &tx);
    drop(tx);
    let stages: Vec<Stage> = rx.iter().collect();
    if result.is_ok() {
        assert!(
            matches!(stages.first(), Some(Stage::Downloading(..))),
            "{stages:?}"
        );
        assert_eq!(
            &stages[stages.len() - 2..],
            &[Stage::Checking, Stage::Installing]
        );
    }
    result
}

#[test]
fn updates_from_a_signed_release_and_refuses_anything_else() {
    if !cfg!(any(windows, target_os = "linux")) {
        return; // no download for this platform: the launcher offers the page
    }
    let (pkcs8, public) = goldlink_sign::generate().unwrap();
    let files = Arc::new(Mutex::new(HashMap::new()));
    let addr = serve(files.clone());
    let tag = "v0.6.2";
    let (name, bytes) = archive(tag, b"the new program");
    let sums = format!("{}  {name}\n", hex_sha256(&bytes));
    let sig = goldlink_sign::sign(&pkcs8, tag, sums.as_bytes()).unwrap();
    let asset = |n: &str| serde_json::json!({ "name": n, "browser_download_url": format!("{addr}/dl/{n}") });
    let list = serde_json::json!([
        { "tag_name": "relay-v0.9", "html_url": "x", "assets": [] },
        { "tag_name": tag, "html_url": format!("{addr}/page"),
          "assets": [asset(&name), asset("SHA256SUMS"), asset("SHA256SUMS.sig")] },
        { "tag_name": "v0.6.1", "html_url": "x", "assets": [] },
    ]);
    {
        let mut f = files.lock().unwrap();
        f.insert(
            "/repos/owner/repo/releases".into(),
            list.to_string().into_bytes(),
        );
        f.insert(format!("/dl/{name}"), bytes.clone());
        f.insert("/dl/SHA256SUMS".into(), sums.clone().into_bytes());
        f.insert("/dl/SHA256SUMS.sig".into(), sig.clone().into_bytes());
    }
    let program = update::program_name();

    // Signed by another key: refused before the download, nothing changed.
    let dir = scratch_program_folder();
    let (_, other) = goldlink_sign::generate().unwrap();
    let e = run(&addr, &dir, &other).unwrap_err();
    assert!(e.contains("signature"), "{e}");
    assert_eq!(
        std::fs::read(dir.join(program)).unwrap(),
        b"the old program"
    );

    // The archive swapped for another (the signed sums no longer match): refused.
    let (_, evil) = archive(tag, b"someone else's program");
    files.lock().unwrap().insert(format!("/dl/{name}"), evil);
    let e = run(&addr, &dir, &public).unwrap_err();
    assert!(e.contains("damaged"), "{e}");
    assert_eq!(
        std::fs::read(dir.join(program)).unwrap(),
        b"the old program"
    );
    files
        .lock()
        .unwrap()
        .insert(format!("/dl/{name}"), bytes.clone());

    // Sums edited to match the swapped archive: the signature no longer matches.
    files.lock().unwrap().insert(
        "/dl/SHA256SUMS".into(),
        format!("{}  {name}\n", "00".repeat(32)).into_bytes(),
    );
    assert!(run(&addr, &dir, &public).unwrap_err().contains("signature"));
    files
        .lock()
        .unwrap()
        .insert("/dl/SHA256SUMS".into(), sums.into_bytes());

    // The real thing: installed, the old program kept aside until the next start.
    let new = run(&addr, &dir, &public).unwrap();
    assert_eq!(new, dir.join(program));
    assert_eq!(
        std::fs::read(dir.join(program)).unwrap(),
        b"the new program"
    );
    assert_eq!(std::fs::read(dir.join("LICENSES/COPYING")).unwrap(), b"GPL");
    assert!(dir.join(format!("{program}.goldlink-old")).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(program))
            .unwrap()
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "the program runs: {mode:o}");
    }
    update::clean_up(&dir);
    assert!(!dir.join(format!("{program}.goldlink-old")).exists());
    std::fs::remove_dir_all(&dir).ok();
}
