//! Dev helper: write the payload tarball for the fixture NAS. usage: dump_payload <out.tar.gz> <Bios|Uefi|UBoot>
use synofurb_core::{config::*, inspect::*, payload::*, plan::*};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let raw = fixtures::ds_4bay();
    let facts = parse_inspect(&raw).unwrap();
    let plan = build_plan(&facts, &PlanRequest::default());
    let mut cfg = InstallConfig::new("nastest", "nasadmin", hash_password("Test-Password-123").unwrap());
    cfg.ssh_authorized_keys.push(generate_ssh_keypair("test").unwrap().public_openssh);
    let p = build_payload(&cfg, &plan, &facts, &raw).unwrap();
    std::fs::write(&a[1], &p.bytes).unwrap();
    println!("{}", p.sha256);
}
