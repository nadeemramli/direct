//! DIR-76: dispatched agent runs with service-enforced, run-scoped authority.
use direct_core::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const MODEL: &str = "claude-opus-5-5";
const TOKEN: &str = "run-secret-token-0123456789";

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

/// A Backlog issue assigned to a verified Claude member with an active role.
fn assigned(s: &mut Store) -> (String, Value) {
    let snap = send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap();
    let dir = snap["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "DIR")
        .unwrap()["id"]
        .clone();
    send(s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":1,
        "documents":[{"id":"dos","title":"t","category":"workflow","relative_path":"8.md","fingerprint":"a".repeat(64),"content":"x"}]}), Role::Agent, 2).unwrap();
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
    send(s, json!({"op":"record_member_capability","id":m["id"],"expected_version":1,"harness_version":"2.1.289","verified_models":[MODEL],"evidence":"e"}), Role::Agent, 7).unwrap();
    let i = send(s, json!({"op":"create_issue","product":"DIR","title":"Shape this","body":"rough","planning_scope":"inbox"}), Role::Agent, 8).unwrap();
    let key = i["key"].as_str().unwrap().to_string();
    let a = send(s, json!({"op":"assign_issue_agent","key":key,"member_id":m["id"],"role_id":role["id"],"requested_model":MODEL}), Role::Human, 9).unwrap();
    (key, a)
}

fn intent(s: &mut Store, key: &str, a: &Value, at: i64) -> Value {
    send(s, json!({"op":"create_agent_run","key":key,"assignment_id":a["id"],"objective":"Tighten the brief and acceptance"}), Role::Human, at).unwrap()
}

fn start(s: &mut Store, r: &Value, at: i64) -> Value {
    send(s, json!({"op":"start_agent_run","id":r["id"],"expected_version":r["version"],"launcher":"pid 1 on test","harness_version":"2.1.289","credential_sha256":sha(TOKEN)}), Role::Agent, at).unwrap()
}

fn session(s: &mut Store, r: &Value, model: &str, at: i64) -> Result<Value> {
    send(
        s,
        json!({"op":"record_run_session","id":r["id"],"expected_version":r["version"],"session_id":r["id"],"actual_model":model}),
        Role::Agent,
        at,
    )
}

fn act(
    s: &mut Store,
    r: &Value,
    index: u32,
    proposal: Value,
    token: &str,
    at: i64,
) -> Result<Value> {
    send(
        s,
        json!({"op":"run_action","request_id":format!("run-{}-action-{index}", r["id"].as_str().unwrap()),
        "run_id":r["id"],"credential":token,"index":index,"proposal":proposal}),
        Role::Agent,
        at,
    )
}

#[test]
fn runs_are_recorded_before_launch_and_only_scoped_proposals_apply() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let (key, a) = assigned(&mut s);
    let other = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Other","planning_scope":"inbox"}),
        Role::Agent,
        10,
    )
    .unwrap();

    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_agent_run","key":key,"assignment_id":a["id"],"objective":"x"}),
            Role::Agent,
            11
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let r = intent(&mut s, &key, &a, 11);
    assert_eq!(r["state"], "intent");
    assert_eq!(r["requested_model"], MODEL);
    assert_eq!(r["guidance"][0]["recorded_fingerprint"], "a".repeat(64));
    assert_eq!(r["skill_bundles"].as_array().unwrap().len(), 1);
    // One run at a time per issue.
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_agent_run","key":key,"assignment_id":a["id"],"objective":"again"}),
            Role::Human,
            12
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    let r = start(&mut s, &r, 13);
    assert_eq!(r["state"], "launching");
    // The intent launches once; the session must be the run ID.
    assert_eq!(send(&mut s, json!({"op":"start_agent_run","id":r["id"],"expected_version":r["version"],"launcher":"x","harness_version":"x","credential_sha256":sha("other")}), Role::Agent, 14).unwrap_err().code, "conflict");
    let wrong = send(&mut s, json!({"op":"record_run_session","id":r["id"],"expected_version":r["version"],"session_id":"some-other-session","actual_model":MODEL}), Role::Agent, 14).unwrap_err();
    assert!(wrong.message.contains("run ID"));
    // Proposals are refused before the session is accepted.
    assert_eq!(
        act(
            &mut s,
            &r,
            0,
            json!({"op":"comment","body":"early"}),
            TOKEN,
            14
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let r = session(&mut s, &r, MODEL, 15).unwrap();
    assert_eq!(r["state"], "running");
    assert_eq!(r["actual_model"], MODEL);

    // Wrong credential: forbidden, nothing recorded.
    assert_eq!(
        act(
            &mut s,
            &r,
            0,
            json!({"op":"comment","body":"x"}),
            "guess",
            16
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let applied = act(
        &mut s,
        &r,
        0,
        json!({"op":"comment","body":"Acceptance should name the owner check."}),
        TOKEN,
        16,
    )
    .unwrap();
    assert_eq!(applied["outcome"], "applied");
    // Exact retry replays; a changed payload under the same request ID conflicts.
    assert_eq!(
        act(
            &mut s,
            &r,
            0,
            json!({"op":"comment","body":"Acceptance should name the owner check."}),
            TOKEN,
            17
        )
        .unwrap(),
        applied
    );
    assert_eq!(
        act(
            &mut s,
            &r,
            0,
            json!({"op":"comment","body":"different"}),
            TOKEN,
            17
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    let updated = act(
        &mut s,
        &r,
        1,
        json!({"op":"update_backlog","acceptance":"1. Owner can see X"}),
        TOKEN,
        18,
    )
    .unwrap();
    assert_eq!(updated["outcome"], "applied");

    // Service-side denials are recorded, never applied.
    for (index, proposal, why) in [
        (2, json!({"op":"ready"}), "not a planning action"),
        (
            3,
            json!({"op":"review","outcome":"passed"}),
            "not a planning action",
        ),
        (
            4,
            json!({"op":"comment","key":other["key"],"body":"elsewhere"}),
            "Outside this run's scope",
        ),
        (
            5,
            json!({"op":"update_backlog","title":"Renamed"}),
            "Unknown field title",
        ),
        (
            6,
            json!({"op":"activate_agent_role","id":"x"}),
            "not a planning action",
        ),
    ] {
        let denied = act(&mut s, &r, index, proposal, TOKEN, 19).unwrap();
        assert_eq!(denied["outcome"], "denied");
        assert!(
            denied["detail"].as_str().unwrap().contains(why),
            "{why}: {}",
            denied["detail"]
        );
    }
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 20).unwrap();
    assert_eq!(ctx["issue"]["acceptance"], "1. Owner can see X");
    assert_eq!(ctx["issue"]["status"], "backlog", "readiness never changes");
    assert!(ctx["comments"].to_string().contains("run:"));
    assert!(send(
        &mut s,
        json!({"op":"context","key":other["key"]}),
        Role::Agent,
        20
    )
    .unwrap()["comments"]
        .as_array()
        .unwrap()
        .is_empty());
    let run_now = &ctx["agent_runs"][0];
    assert_eq!(run_now["actions"].as_array().unwrap().len(), 7);

    let done = send(&mut s, json!({"op":"finish_agent_run","id":r["id"],"expected_version":run_now["version"],"succeeded":true,"summary":"Two changes applied, five denied"}), Role::Agent, 21).unwrap();
    assert_eq!(done["state"], "succeeded");
    // The credential dies with the run.
    assert_eq!(
        act(
            &mut s,
            &r,
            7,
            json!({"op":"comment","body":"late"}),
            TOKEN,
            22
        )
        .unwrap_err()
        .code,
        "forbidden"
    );

    // Restart and archive keep history; credentials are not archived and
    // capability must be re-checked after a restore.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 23);
    assert_eq!(archive.agent_runs.len(), 1);
    assert!(!serde_json::to_string(&archive)
        .unwrap()
        .contains(&sha(TOKEN)));
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let blocked = send(&mut restored, json!({"op":"create_agent_run","key":key,"assignment_id":a["id"],"objective":"after restore"}), Role::Human, now() + 10).unwrap_err();
    assert!(blocked.message.contains("predates the last restore"));
    let mut old = archive;
    old.format = 21;
    assert!(validate_archive(&old).is_err());
}

#[test]
fn substitution_blocks_unknown_outcomes_suspend_and_cancel_is_acknowledged() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let (key, a) = assigned(&mut s);

    // A substituted model blocks before any proposal can apply.
    let r = intent(&mut s, &key, &a, 11);
    let r = start(&mut s, &r, 12);
    let blocked = session(&mut s, &r, "claude-sonnet-5-5", 13).unwrap();
    assert_eq!(blocked["state"], "blocked");
    assert!(blocked["reason"]
        .as_str()
        .unwrap()
        .contains("Model substitution"));
    assert_eq!(
        act(&mut s, &r, 0, json!({"op":"comment","body":"x"}), TOKEN, 14)
            .unwrap_err()
            .code,
        "forbidden"
    );

    // A missing channel blocks an intent without dispatching.
    let r = intent(&mut s, &key, &a, 15);
    let b = send(&mut s, json!({"op":"block_agent_run","id":r["id"],"expected_version":1,"reason":"claude is not installed"}), Role::Agent, 16).unwrap();
    assert_eq!(b["state"], "blocked");
    assert!(b["launcher"].is_null());

    // An unreconcilable launch becomes unknown and suspends redispatch.
    let r = intent(&mut s, &key, &a, 17);
    let r = start(&mut s, &r, 18);
    send(&mut s, json!({"op":"mark_agent_run_unknown","id":r["id"],"expected_version":r["version"],"reason":"No session file for the run ID after restart"}), Role::Agent, 19).unwrap();
    let suspended = send(
        &mut s,
        json!({"op":"create_agent_run","key":key,"assignment_id":a["id"],"objective":"retry"}),
        Role::Human,
        20,
    )
    .unwrap_err();
    assert!(suspended.message.contains("unknown outcome"));

    // Reassigning resets the suspension; cancel waits for the runner.
    let snap = send(&mut s, json!({"op":"snapshot"}), Role::Agent, 21).unwrap();
    let m = snap["agent_members"][0].clone();
    let role = snap["agent_roles"][0].clone();
    let a2 = send(&mut s, json!({"op":"assign_issue_agent","key":key,"expected_assignment_version":1,"member_id":m["id"],"role_id":role["id"],"requested_model":MODEL}), Role::Human, 22).unwrap();
    let r = intent(&mut s, &key, &a2, 23);
    let canceled_early = send(
        &mut s,
        json!({"op":"cancel_agent_run","id":r["id"],"expected_version":1}),
        Role::Human,
        24,
    )
    .unwrap();
    assert_eq!(
        canceled_early["state"], "canceled",
        "an intent that never launched cancels at once"
    );
    let r = intent(&mut s, &key, &a2, 25);
    let r = start(&mut s, &r, 26);
    let r = session(&mut s, &r, MODEL, 27).unwrap();
    act(
        &mut s,
        &r,
        0,
        json!({"op":"comment","body":"kept after cancel"}),
        TOKEN,
        28,
    )
    .unwrap();
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 29).unwrap();
    let live = ctx["agent_runs"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(
        send(
            &mut s,
            json!({"op":"cancel_agent_run","id":live["id"],"expected_version":live["version"]}),
            Role::Agent,
            30
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let pending = send(
        &mut s,
        json!({"op":"cancel_agent_run","id":live["id"],"expected_version":live["version"]}),
        Role::Human,
        30,
    )
    .unwrap();
    assert_eq!(pending["state"], "cancel_pending");
    assert_eq!(
        act(
            &mut s,
            &r,
            1,
            json!({"op":"comment","body":"after cancel"}),
            TOKEN,
            31
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let acked = send(&mut s, json!({"op":"acknowledge_run_cancel","id":live["id"],"expected_version":pending["version"]}), Role::Agent, 32).unwrap();
    assert_eq!(acked["state"], "canceled");
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 33).unwrap();
    assert!(ctx["comments"].to_string().contains("kept after cancel"));

    // An active claim by another writer must be reconciled before dispatch.
    let v = ctx["issue"]["version"].as_u64().unwrap();
    send(&mut s, json!({"op":"update_issue","key":key,"expected_version":v,"title":"Shape this","body":"b","acceptance":"a","owner":"o","priority":"medium"}), Role::Agent, 34).unwrap();
    send(
        &mut s,
        json!({"op":"ready","key":key,"expected_version":v + 1}),
        Role::Human,
        35,
    )
    .unwrap();
    send(&mut s, json!({"op":"claim","key":key,"expected_version":v + 2,"lease_seconds":600,"actor":"other-writer"}), Role::Agent, 36).unwrap();
    let busy = send(
        &mut s,
        json!({"op":"create_agent_run","key":key,"assignment_id":a2["id"],"objective":"x"}),
        Role::Human,
        37,
    )
    .unwrap_err();
    assert!(busy.message.contains("other-writer holds an active claim"));
}
