//! Dev helper: run the DSM inspection script on THIS machine and show how it parses and plans.
use synofurb_core::{inspect::*, plan::*};

fn main() {
    let out = std::process::Command::new("sh").arg("-c").arg(inspect_script()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    if std::env::args().any(|a| a == "--raw") {
        print!("{text}");
        return;
    }
    let f = parse_inspect(&text).expect("parse");
    println!(
        "model={:?} arch={:?} fw={:?} mem={}MiB kexec={} root={} serial={:?}",
        f.model,
        f.arch,
        f.firmware,
        f.mem_kib / 1024,
        f.kexec,
        f.is_root,
        f.serial_console()
    );
    for d in &f.disks {
        println!(
            "disk {} {} serial={:?} {} sectors table={:?} candidate={} parts={}",
            d.name,
            d.model,
            d.serial,
            d.sectors,
            d.table,
            d.is_candidate(),
            d.parts.len()
        );
        for p in &d.parts {
            println!("   {} #{} start={} sectors={}", p.name, p.number, p.start, p.sectors);
        }
    }
    println!("arrays={} lvs={} mounts={} nics={:?}", f.arrays.len(), f.lvs.len(), f.mounts.len(), f.nics);
    let p = build_plan(&f, &PlanRequest::default());
    println!("plan ok={} errors={:?}", p.is_ok(), p.errors);
    for l in p.summary() {
        println!("  {l}");
    }
}
