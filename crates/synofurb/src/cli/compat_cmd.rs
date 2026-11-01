//! `compat`: look models up in the compatibility list.

use super::args::CompatArgs;
use super::exit::{CliError, CliResult};
use super::facts::{SCHEMA, config_dir, load_compat_db, parse_status, status_key};
use super::out::Out;
use serde_json::{Value, json};
use synofurb_core::compat::{CompatDb, DEFAULT_URL, DbOrigin, ModelEntry, RefreshOutcome, Status};

pub fn refresh_note(r: &anyhow::Result<RefreshOutcome>) -> (bool, String) {
    match r {
        Ok(RefreshOutcome::Updated { models, updated }) => (true, format!("Updated: {models} models, list dated {updated}.")),
        Ok(RefreshOutcome::NotModified) => (true, "The cached list is already current.".into()),
        Ok(RefreshOutcome::Offline { reason }) => {
            (false, format!("Could not reach the server ({reason}). The cached or built-in list stays in use."))
        }
        Ok(RefreshOutcome::Rejected { reason }) => (false, format!("The downloaded list was rejected ({reason}). Nothing was changed.")),
        Err(e) => (false, format!("Refresh failed: {e:#}")),
    }
}

pub fn entry_lines(e: &ModelEntry) -> Vec<String> {
    vec![
        format!("{} ({}, {}): {}", e.model, e.arch.debian(), e.status.label(), e.status.explain()),
        format!("  {}", e.notes.clone().unwrap_or_default()),
        format!(
            "  platform {} | cpu {} | boot {:?} | evidence items: {}",
            e.platform,
            e.cpu.clone().unwrap_or_default(),
            e.boot,
            e.evidence.len()
        ),
    ]
}

pub fn model_json(db: &CompatDb, query: &str) -> Value {
    match db.lookup(query) {
        Some(e) => json!({
            "schema": SCHEMA, "query": query, "found": true,
            "status": status_key(e.status), "status_label": e.status.label(), "explanation": e.status.explain(),
            "blocking": e.status == Status::Unsupported, "entry": e,
        }),
        None => json!({
            "schema": SCHEMA, "query": query, "found": false,
            "status": status_key(Status::Untested), "status_label": Status::Untested.label(),
            "explanation": "not in the compatibility list; treated as untested", "blocking": false, "entry": Value::Null,
        }),
    }
}

pub fn summary_json(db: &CompatDb) -> Value {
    let by: serde_json::Map<String, Value> = db.count_by_status().into_iter().map(|(s, n)| (status_key(s).to_string(), json!(n))).collect();
    json!({
        "schema": SCHEMA,
        "models": db.len(),
        "updated": db.updated(),
        "origin": match db.origin() { DbOrigin::Embedded => "embedded", DbOrigin::Cache => "cache" },
        "by_status": by,
    })
}

pub fn compat(out: &Out, a: CompatArgs) -> CliResult<()> {
    let mut refresh_json = Value::Null;
    if a.refresh {
        let r = CompatDb::refresh(&config_dir(), DEFAULT_URL);
        let (ok, msg) = refresh_note(&r);
        refresh_json = json!({"ok": ok, "message": msg});
        out.info(&format!("refresh: {msg}"));
    }
    let db = load_compat_db();
    if a.list {
        let want = match &a.status {
            Some(s) => Some(parse_status(s).ok_or_else(|| {
                CliError::usage(format!("unknown status '{s}' (verified, community, untested, known-issues, unsupported)"))
            })?),
            None => None,
        };
        let rows: Vec<&ModelEntry> = db.models().iter().filter(|m| want.is_none_or(|w| m.status == w)).collect();
        if a.json {
            out.json_doc(&json!({"schema": SCHEMA, "models": rows}));
        } else {
            for m in rows {
                out.data(&format!(
                    "{:12} {:20} {:10} {:12} {}",
                    m.model,
                    m.status.label(),
                    m.arch.debian(),
                    m.platform,
                    m.cpu.clone().unwrap_or_default()
                ));
            }
        }
        return Ok(());
    }
    match a.model {
        Some(m) => {
            let blocking = db.lookup(&m).is_some_and(|e| e.status == Status::Unsupported);
            if a.json {
                let mut v = model_json(&db, &m);
                if !refresh_json.is_null() {
                    v["refresh"] = refresh_json;
                }
                out.json_doc(&v);
            } else {
                match db.lookup(&m) {
                    Some(e) => out.data_lines(entry_lines(e)),
                    None => out.data(&format!("{m}: not in the list. Treated as untested; please file a compatibility report.")),
                }
            }
            if blocking {
                return Err(CliError::Unsupported(format!("{m} is listed as unsupported: the hardware cannot run Debian 13")));
            }
        }
        None => {
            if a.json {
                let mut v = summary_json(&db);
                if !refresh_json.is_null() {
                    v["refresh"] = refresh_json;
                }
                out.json_doc(&v);
            } else {
                out.data(&format!(
                    "{} models (list updated {}, from the {} copy)",
                    db.len(),
                    db.updated(),
                    if db.origin() == DbOrigin::Cache { "cached" } else { "built-in" }
                ));
                for (s, n) in db.count_by_status() {
                    out.data(&format!("  {:<18} {n}", s.label()));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_document_shapes() {
        let db = CompatDb::embedded();
        let v = model_json(&db, "DS923+");
        assert_eq!((v["found"].as_bool(), v["schema"].as_u64()), (Some(true), Some(1)));
        assert_eq!(v["entry"]["model"], "DS923+");
        assert!(v["status"].is_string() && v["status_label"].is_string() && v["blocking"].is_boolean());
        let v = model_json(&db, "DS999-nothing");
        assert_eq!((v["found"].as_bool(), v["status"].as_str(), v["blocking"].as_bool()), (Some(false), Some("untested"), Some(false)));
        assert!(v["entry"].is_null());
    }

    #[test]
    fn summary_document_shape() {
        let db = CompatDb::embedded();
        let v = summary_json(&db);
        assert!(v["models"].as_u64().unwrap() > 100);
        assert_eq!(v["origin"], "embedded");
        let by = v["by_status"].as_object().unwrap();
        for k in ["verified", "community", "untested", "known-issues", "unsupported"] {
            assert!(by.contains_key(k), "{k}");
        }
        assert_eq!(by.values().map(|n| n.as_u64().unwrap()).sum::<u64>(), v["models"].as_u64().unwrap());
    }

    #[test]
    fn unsupported_models_exit_5() {
        let db = CompatDb::embedded();
        let unsupported =
            db.models().iter().find(|m| m.status == Status::Unsupported).expect("the list has unsupported models").model.clone();
        let out = Out::plain();
        let r = compat(&out, CompatArgs { model: Some(unsupported), refresh: false, json: true, list: false, status: None });
        assert_eq!(r.err().unwrap().code(), 5);
        let r = compat(&out, CompatArgs { model: Some("DS923+".into()), refresh: false, json: true, list: false, status: None });
        assert!(r.is_ok());
    }
}
