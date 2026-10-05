//! DIR-77: Product Planner queue reviews with service-judged findings.
use direct_core::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const MODEL: &str = "claude-opus-5-5";
const TOKEN: &str = "planner-run-token-0123456789";

fn send(s: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    if value.get("actor").is_none() {
        value["actor"] = json!(if role == Role::Human {
            "owner"
        } else {
            "direct-runner"
        });
    }
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn issue(s: &mut Store, title: &str, body: &str) -> String {
    let i = send(s, json!({"op":"create_issue","product":"DIR","title":title,"body":body,"planning_scope":"inbox",
        "intake":{"text":format!("Original words for {title}"),"images":[]}}), Role::Agent, 10).unwrap();
    i["key"].as_str().unwrap().to_string()
}

fn version(s: &mut Store, key: &str) -> u64 {
    send(s, json!({"op":"context","key":key}), Role::Agent, 1).unwrap()["issue"]["version"]
        .as_u64()
        .unwrap()
}

struct World {
    member: Value,
    role: Value,
    clear: String,
    vague: String,
    big: String,
    dup: String,
    blocked: String,
    submitted: String,
    done: String,
    stale: String,
}

fn world(s: &mut Store) -> World {
    let snap = send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap();
    let dir = snap["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "DIR")
        .unwrap()["id"]
        .clone();
    send(s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":1,
        "documents":[{"id":"dos","title":"t","category":"workflow","relative_path":"8.md","fingerprint":"a".repeat(64),"content":"x"},
                     {"id":"moving","title":"m","category":"workflow","relative_path":"9.md","fingerprint":"b".repeat(64),"content":"y"}]}), Role::Agent, 2).unwrap();
    let skill = send(s, json!({"op":"register_skill_package","name":"s","description":"d","trigger":"t","origin":"local","license":"l","files":[{"path":"SKILL.md","content":"---\nname: s\ndescription: d\n---\n"}]}), Role::Agent, 3).unwrap();
    let role = send(s, json!({"op":"register_agent_role","key":"product-planner","name":"Planner","responsibilities":["r"],"inputs":["i"],"outputs":["o"],
        "skills":[skill["id"]],"runtime_compatibility":["claude-code"],"guidance":[{"document_id":"dos","mandatory":true}],"owner_direction":"bounded"}), Role::Agent, 4).unwrap();
    send(
        s,
        json!({"op":"activate_agent_role","id":role["id"],"note":"ok"}),
        Role::Human,
        5,
    )
    .unwrap();
    let m = send(s, json!({"op":"create_agent_member","name":"Claude","runtime":"claude-code","connection_ref":"local","product_ids":[dir]}), Role::Human, 6).unwrap();
    let member = send(s, json!({"op":"record_member_capability","id":m["id"],"expected_version":1,"harness_version":"2.1.289","verified_models":[MODEL],"evidence":"e"}), Role::Agent, 7).unwrap();

    let clear = issue(s, "Clear", "Owners can export the run log as JSON.");
    let vague = issue(s, "Make it better", "Improve things.");
    let big = issue(
        s,
        "Everything",
        "Rewrite the app, the service and the desktop shell.",
    );
    let dup = issue(s, "Export run log", "Owners export the run log.");
    let blocked = issue(s, "Needs export first", "Depends on export.");
    send(s, json!({"op":"create_issue_link","key":blocked,"expected_version":1,"target_key":clear,"kind":"blocked_by"}), Role::Agent, 11).unwrap();
    // A submitted issue and a completed one.
    let submitted = issue(s, "Submitted", "b");
    for (key, finish) in [(&submitted, false), (&issue(s, "Done", "b"), true)] {
        send(s, json!({"op":"update_issue","key":key,"expected_version":1,"title":"t","body":"b","acceptance":"a","owner":"o","priority":"medium"}), Role::Agent, 12).unwrap();
        send(
            s,
            json!({"op":"ready","key":key,"expected_version":2}),
            Role::Human,
            13,
        )
        .unwrap();
        send(
            s,
            json!({"op":"claim","key":key,"expected_version":3,"lease_seconds":600,"actor":"impl"}),
            Role::Agent,
            14,
        )
        .unwrap();
        let sub = json!({"op":"submit","actor":"impl","key":key,"expected_version":4,"build_ref":"commit:abc","delivery_ref":"main","summary":"s","checks":"c",
            "e2e":{"build_ref":"commit:abc","delivered_build_ref":"commit:abc","environment":"f","entrypoint":"f","scenarios":"all","outcome":"passed","delivery_check":"i"},
            "steps":[{"instruction":"i","expected":"e"}]});
        let r = send(s, sub, Role::Agent, 15).unwrap();
        if finish {
            send(s, json!({"op":"review","key":key,"expected_version":5,"run_id":r["current_run"],"outcome":"passed","results":[{"outcome":"passed"}]}), Role::Human, 16).unwrap();
        }
    }
    let done = send(s, json!({"op":"snapshot"}), Role::Agent, 17).unwrap()["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == "t" && i["status"] == "done")
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let stale = issue(s, "Pinned to moving guidance", "b");
    send(
        s,
        json!({"op":"link_theoria","key":stale,"expected_version":1,"document_id":"moving"}),
        Role::Human,
        18,
    )
    .unwrap();
    send(s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":2,
        "documents":[{"id":"moving","title":"m","category":"workflow","relative_path":"9.md","fingerprint":"c".repeat(64),"content":"y2"}]}), Role::Agent, 19).unwrap();
    World {
        member,
        role,
        clear,
        vague,
        big,
        dup,
        blocked,
        submitted,
        done,
        stale,
    }
}

fn queue(w: &World, policy: &str) -> Value {
    json!({"op":"create_queue_run","product":"DIR","keys":[w.clear,w.vague,w.big,w.dup,w.blocked,w.submitted,w.done,w.stale],
        "member_id":w.member["id"],"role_id":w.role["id"],"requested_model":MODEL,"policy":policy,"objective":"Review the queue"})
}

/// Launch a recorded queue run up to Running.
fn running(s: &mut Store, r: &Value, at: i64) -> Value {
    let r = send(s, json!({"op":"start_agent_run","id":r["id"],"expected_version":r["version"],"launcher":"pid 1 on test","harness_version":"2.1.289","credential_sha256":sha(TOKEN)}), Role::Agent, at).unwrap();
    send(s, json!({"op":"record_run_session","id":r["id"],"expected_version":r["version"],"session_id":r["id"],"actual_model":MODEL}), Role::Agent, at).unwrap()
}

fn act(s: &mut Store, r: &Value, index: u32, proposal: Value, at: i64) -> Value {
    send(s, json!({"op":"run_action","run_id":r["id"],"credential":TOKEN,"index":index,"proposal":proposal}), Role::Agent, at).unwrap()
}

fn finding(key: &str, kind: &str) -> Value {
    json!({"op":"finding","issue_key":key,"kind":kind,"route":"bounded_brief","summary":format!("{kind} on {key}"),
        "evidence":[format!("{key} brief")],"recommendation":"Write observable criteria"})
}

fn finish(s: &mut Store, r: &Value, at: i64) {
    let v = send(s, json!({"op":"snapshot"}), Role::Agent, at).unwrap()["agent_runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == r["id"])
        .unwrap()["version"]
        .clone();
    send(s, json!({"op":"finish_agent_run","id":r["id"],"expected_version":v,"succeeded":true,"summary":"done"}), Role::Agent, at).unwrap();
}

#[test]
fn inspect_only_reviews_record_findings_and_never_write_issues() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);
    assert_eq!(
        send(&mut s, queue(&w, "inspect_only"), Role::Agent, 20)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut unverified = queue(&w, "inspect_only");
    unverified["requested_model"] = json!("claude-sonnet-5-5");
    assert!(send(&mut s, unverified, Role::Human, 20)
        .unwrap_err()
        .message
        .contains("unverified"));

    let r = send(&mut s, queue(&w, "inspect_only"), Role::Human, 21).unwrap();
    let q = &r["queue"];
    assert_eq!(q["keys"].as_array().unwrap().len(), 7);
    assert_eq!(
        q["blocked"][0]["key"],
        w.stale.as_str(),
        "stale guidance blocks only that issue"
    );
    assert!(q["blocked"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("changed since it was linked"));
    assert_eq!(
        send(&mut s, queue(&w, "inspect_only"), Role::Human, 22)
            .unwrap_err()
            .code,
        "conflict"
    );

    let before: Vec<u64> = [&w.vague, &w.big, &w.clear]
        .iter()
        .map(|k| version(&mut s, k))
        .collect();
    let r = running(&mut s, &r, 23);
    assert_eq!(
        act(&mut s, &r, 0, finding(&w.vague, "unclear_outcome"), 24)["outcome"],
        "applied"
    );
    assert_eq!(
        act(&mut s, &r, 1, finding(&w.big, "oversized"), 24)["outcome"],
        "applied"
    );
    let mut dup = finding(&w.dup, "duplicate");
    dup["evidence"] = json!([format!(
        "{} and {} describe the same export",
        w.dup, w.clear
    )]);
    assert_eq!(act(&mut s, &r, 2, dup, 24)["outcome"], "applied");
    for (index, proposal, why) in [
        (
            3,
            json!({"op":"comment","issue_key":w.vague,"body":"x"}),
            "inspect-only",
        ),
        (
            4,
            json!({"op":"update_backlog","issue_key":w.vague,"expected_version":1,"body":"x"}),
            "inspect-only",
        ),
        (
            5,
            finding(&w.stale, "missing_criteria"),
            "excluded from this review",
        ),
        (
            6,
            finding("DIR-999", "missing_criteria"),
            "not in the reviewed queue",
        ),
        (
            7,
            json!({"op":"ready","issue_key":w.clear}),
            "not a planning action",
        ),
        (
            8,
            json!({"op":"merge_duplicates","issue_key":w.dup}),
            "not a planning action",
        ),
        (
            9,
            json!({"op":"finding","issue_key":w.big,"kind":"oversized","route":"reviewed_design","summary":"s","evidence":["e"],"recommendation":"r"}),
            "need an escalation",
        ),
    ] {
        let out = act(&mut s, &r, index, proposal, 25);
        assert_eq!(out["outcome"], "denied");
        assert!(
            out["detail"].as_str().unwrap().contains(why),
            "{why}: {}",
            out["detail"]
        );
    }
    let after: Vec<u64> = [&w.vague, &w.big, &w.clear]
        .iter()
        .map(|k| version(&mut s, k))
        .collect();
    assert_eq!(before, after, "inspect-only made no issue writes");
    let ctx = send(
        &mut s,
        json!({"op":"context","key":w.vague}),
        Role::Agent,
        26,
    )
    .unwrap();
    assert_eq!(ctx["planner_findings"][0]["kind"], "unclear_outcome");
    assert!(ctx["comments"].as_array().unwrap().is_empty());
    assert_eq!(ctx["agent_runs"][0]["queue"]["policy"], "inspect_only");
}

#[test]
fn refine_mode_is_bounded_versioned_preserving_and_deduplicated() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let w = world(&mut s);
    let r = send(&mut s, queue(&w, "refine_backlog"), Role::Human, 21).unwrap();
    let r = running(&mut s, &r, 22);
    let v = version(&mut s, &w.vague);

    // A rewrite needs the version the run saw and preserves the previous text.
    let edit = json!({"op":"update_backlog","issue_key":w.vague,"expected_version":v,"body":"Owners can see why a run failed.","acceptance":"1. Failure reason is shown"});
    assert_eq!(act(&mut s, &r, 0, edit, 23)["outcome"], "applied");
    let ctx = send(
        &mut s,
        json!({"op":"context","key":w.vague}),
        Role::Agent,
        24,
    )
    .unwrap();
    assert_eq!(ctx["issue"]["body"], "Owners can see why a run failed.");
    assert_eq!(
        ctx["issue"]["intake"]["text"], "Original words for Make it better",
        "intake untouched"
    );
    let preserved = ctx["comments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["body"].as_str().unwrap().contains("Previous brief"))
        .unwrap();
    assert!(preserved["body"]
        .as_str()
        .unwrap()
        .contains("Improve things."));
    // A second edit from the same stale read is refused, nothing overwritten.
    let stale = act(
        &mut s,
        &r,
        1,
        json!({"op":"update_backlog","issue_key":w.vague,"expected_version":v,"body":"Other text"}),
        25,
    );
    assert_eq!(stale["outcome"], "denied");
    assert!(stale["detail"]
        .as_str()
        .unwrap()
        .contains("changed since this review read it"));
    for (index, key, why) in [
        (2, &w.submitted, "not in Backlog"),
        (3, &w.done, "not in Backlog"),
    ] {
        let kv = version(&mut s, key);
        let out = act(
            &mut s,
            &r,
            index,
            json!({"op":"update_backlog","issue_key":key,"expected_version":kv,"acceptance":"rewritten"}),
            26,
        );
        assert_eq!(out["outcome"], "denied");
        assert!(out["detail"].as_str().unwrap().contains(why));
    }
    assert_eq!(
        act(
            &mut s,
            &r,
            4,
            json!({"op":"comment","issue_key":w.big,"body":"Consider splitting into service and UI issues."}),
            27
        )["outcome"],
        "applied"
    );
    // An escalation is kept unconfirmed until a coordinator (never the run) decides.
    let esc = json!({"op":"finding","issue_key":w.big,"kind":"oversized","route":"reviewed_design","summary":"Spans three surfaces","evidence":["brief names app, service and desktop"],
        "recommendation":"Split","escalation":{"criterion":"Scope fits one delivery","evidence":"three surfaces","impact":"cannot verify in one pass","options":["split","design review"],"recommendation":"split"}});
    assert_eq!(act(&mut s, &r, 5, esc, 28)["outcome"], "applied");
    finish(&mut s, &r, 29);
    let f = send(&mut s, json!({"op":"context","key":w.big}), Role::Agent, 30).unwrap()
        ["planner_findings"][0]
        .clone();
    assert!(f["confirmation"].is_null());
    assert_eq!(send(&mut s, json!({"op":"confirm_planner_finding","id":f["id"],"expected_version":f["version"],"confirmed":true,"note":"x","actor":format!("run:{}", r["id"].as_str().unwrap())}), Role::Agent, 31).unwrap_err().code, "forbidden");
    let c = send(&mut s, json!({"op":"confirm_planner_finding","id":f["id"],"expected_version":f["version"],"confirmed":true,"note":"Agreed: split it"}), Role::Human, 31).unwrap();
    assert_eq!(c["confirmation"]["confirmed"], true);

    // Repeating identical evidence retains one finding; changed evidence revises it.
    let r2 = send(&mut s, queue(&w, "inspect_only"), Role::Human, 32).unwrap();
    let r2 = running(&mut s, &r2, 33);
    assert_eq!(
        act(&mut s, &r2, 0, finding(&w.dup, "duplicate"), 34)["outcome"],
        "applied"
    );
    finish(&mut s, &r2, 35);
    let r3 = send(&mut s, queue(&w, "inspect_only"), Role::Human, 36).unwrap();
    let r3 = running(&mut s, &r3, 37);
    let retained = act(&mut s, &r3, 0, finding(&w.dup, "duplicate"), 38);
    assert_eq!(retained["outcome"], "retained");
    let dv = version(&mut s, &w.dup);
    send(&mut s, json!({"op":"update_issue","key":w.dup,"expected_version":dv,"title":"Export run log","body":"Owners export the run log as CSV too.","acceptance":"","owner":"","priority":"medium"}), Role::Agent, 39).unwrap();
    let mut changed = finding(&w.dup, "duplicate");
    changed["summary"] = json!("Overlaps but now adds CSV");
    assert_eq!(act(&mut s, &r3, 1, changed, 40)["outcome"], "applied");
    let ctx = send(&mut s, json!({"op":"context","key":w.dup}), Role::Agent, 41).unwrap();
    let findings = ctx["planner_findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "one finding per issue and kind");
    assert_eq!(findings[0]["seen"], 3);
    assert_eq!(findings[0]["revisions"].as_array().unwrap().len(), 1);
    assert_eq!(findings[0]["summary"], "Overlaps but now adds CSV");
    finish(&mut s, &r3, 42);

    // Restart and archive recovery keep runs, scope and findings.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 23);
    assert_eq!(archive.planner_findings.len(), 2);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut old = archive.clone();
    old.format = 22;
    assert!(validate_archive(&old).is_err());
    let mut doubled = archive;
    let copy = doubled.planner_findings[0].clone();
    doubled.planner_findings.push(PlannerFinding {
        id: "x".into(),
        ..copy
    });
    assert!(validate_archive(&doubled).is_err());
}

#[test]
fn refine_mode_never_overwrites_an_active_writer() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);
    let r = send(&mut s, queue(&w, "refine_backlog"), Role::Human, 21).unwrap();
    let r = running(&mut s, &r, 22);
    // Another actor takes the Backlog issue's text in hand mid-review.
    let v = version(&mut s, &w.big);
    send(&mut s, json!({"op":"update_issue","key":w.big,"expected_version":v,"title":"Everything","body":"Rewrite the app.","acceptance":"a","owner":"o","priority":"medium","actor":"human-editor"}), Role::Agent, 23).unwrap();
    let out = act(
        &mut s,
        &r,
        0,
        json!({"op":"update_backlog","issue_key":w.big,"expected_version":v,"body":"Split"}),
        24,
    );
    assert_eq!(out["outcome"], "denied");
    let ctx = send(&mut s, json!({"op":"context","key":w.big}), Role::Agent, 25).unwrap();
    assert_eq!(
        ctx["issue"]["body"], "Rewrite the app.",
        "the other actor's edit stands"
    );
    // The blocked_by dependency is visible to findings on the dependent issue.
    assert_eq!(
        act(&mut s, &r, 1, finding(&w.blocked, "dependency"), 26)["outcome"],
        "applied"
    );
}
