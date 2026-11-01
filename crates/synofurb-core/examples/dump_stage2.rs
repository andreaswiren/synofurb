//! Dev helper: render the stage-2 script for a facts file (inspect output format).
//! usage: dump_stage2 <facts.txt> <wipe|keep> [--live]
use synofurb_core::{config::*, inspect::parse_inspect, plan::*, script::*};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let facts = parse_inspect(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let mode = if a[2] == "keep" { StorageMode::KeepVolumes } else { StorageMode::WipeAll };
    let plan = build_plan(&facts, &PlanRequest { mode, ..Default::default() });
    if !plan.is_ok() {
        eprintln!("plan errors: {:?}", plan.errors);
        std::process::exit(2);
    }
    let cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
    let p = ScriptParams {
        plan: &plan,
        facts: &facts,
        cfg: &cfg,
        run_id: "dev",
        http_base: "http://127.0.0.1:1/t",
        payload_sha256: "00",
        log_host: "127.0.0.1",
        log_tcp_port: 1,
        dry_run: !a.iter().any(|x| x == "--live"),
    };
    print!("{}", render_stage2(&p).unwrap());
}
