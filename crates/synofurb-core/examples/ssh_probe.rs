//! Dev helper: exercise the SSH layer against a real server.
//! usage: ssh_probe <host> <port> <user> <password>
use synofurb_core::logs::Logger;
use synofurb_core::remote::{Auth, RemoteShell, SshSession, fetch_host_fingerprint};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let log = Logger::memory(None);
    let fp = fetch_host_fingerprint(&a[1], a[2].parse().unwrap(), std::time::Duration::from_secs(5)).unwrap();
    println!("fingerprint-only: {fp}");
    let mut s = SshSession::connect(&a[1], a[2].parse().unwrap(), &a[3], &Auth::Password(a[4].clone()), &log).unwrap();
    let r = s.exec("echo hello; id -un; echo err >&2; exit 3", None).unwrap();
    println!("exec: code={} out={:?} err={:?}", r.code, r.stdout, r.stderr);
    let r = s.exec("cat", Some(b"stdin works\n")).unwrap();
    println!("stdin: {:?}", r.stdout);
    // 40 MiB upload, checksum compare
    let data: Vec<u8> = (0..40 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let mut sent = 0;
    s.upload("/tmp/sf-upload-test.bin", &mut &data[..], data.len() as u64, 0o640, &mut |n| sent = n).unwrap();
    let r = s.exec("sha256sum /tmp/sf-upload-test.bin; stat -c %a /tmp/sf-upload-test.bin", None).unwrap();
    println!("upload sent={sent} remote: {}", r.stdout.replace('\n', " | "));
    use sha2::Digest;
    println!("local sha: {}", hex::encode(sha2::Sha256::digest(&data)));
    let bad = s.upload("/tmp/x'y", &mut &b"x"[..], 1, 0o600, &mut |_| {});
    println!("unsafe path rejected: {}", bad.is_err());
    let w = SshSession::connect(&a[1], a[2].parse().unwrap(), &a[3], &Auth::Password("wrong".into()), &log);
    println!("wrong password rejected: {}", w.is_err());
}
