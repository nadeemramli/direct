//! DIR-79: implementation dispatch of owner-Ready work, one writer per product.
use direct_core::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const MODEL: &str = "claude-opus-5-5";
const TOKEN: &str = "dispatch-run-token-0123456789";
const BASE: &str = "1111111111111111111111111111111111111111";
const HEAD: &str = "2222222222222222222222222222222222222222";

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

fn version(s: &mut Store, key: &str) -> u64 {
    context(s, key)["issue"]["version"].as_u64().unwrap()
}

fn context(s: &mut Store, key: &str) -> Value {
    send(s, json!({"op":"context","key":key}), Role::Agent, 1).unwrap()
}

fn snapshot(s: &mut Store, field: &str) -> Vec<Value> {
    send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap()[field]
        .as_array()
        .unwrap()
        .clone()
}

fn product_id(s: &mut Store, key: &str) -> Value {
    snapshot(s, "products")
        .into_iter()
        .find(|p| p["key"] == key)
        .unwrap()["id"]
        .clone()
}

/// An issue in `product`; Ready unless `ready` is false.
fn issue(s: &mut Store, product: &str, title: &str, priority: &str, ready: bool) -> String {
    let i = send(
        s,
        json!({"op":"create_issue","product":product,"title":title,"planning_scope":"inbox"}),
        Role::Agent,
        10,
    )
    .unwrap();
    let key = i["key"].as_str().unwrap().to_string();
    send(s, json!({"op":"update_issue","key":key,"expected_version":1,"title":title,"body":format!("Brief for {title}"),"acceptance":"It works","owner":"Owner","priority":priority}), Role::Agent, 11).unwrap();
    if ready {
        send(
            s,
            json!({"op":"ready","key":key,"expected_version":2}),
            Role::Human,
            12,
        )
        .unwrap();
    }
    key
}

/// Pin the shared `dos` guidance the way an agent does: under its own claim.
fn pin(s: &mut Store, key: &str, at: i64) {
    let v = version(s, key);
    send(
        s,
        json!({"op":"claim","actor":"pinner","key":key,"expected_version":v,"lease_seconds":60}),
        Role::Agent,
        at,
    )
    .unwrap();
    send(s, json!({"op":"link_theoria","actor":"pinner","key":key,"expected_version":v + 1,"document_id":"dos"}), Role::Agent, at).unwrap();
    send(
        s,
        json!({"op":"release","actor":"pinner","key":key,"expected_version":v + 2}),
        Role::Agent,
        at,
    )
    .unwrap();
}

struct World {
    member: String,
    role: String,
}

fn world(s: &mut Store) -> World {
    send(
        s,
        json!({"op":"create_product","key":"INS","name":"Inspo"}),
        Role::Human,
        1,
    )
    .unwrap();
    let dir = product_id(s, "DIR");
    let ins = product_id(s, "INS");
    send(s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":1,
        "documents":[{"id":"dos","title":"Workflow","category":"workflow","relative_path":"8.md","fingerprint":"a".repeat(64),"content":"x"}]}), Role::Agent, 2).unwrap();
    let skill = send(s, json!({"op":"register_skill_package","name":"s","description":"d","trigger":"t","origin":"local","license":"l","files":[{"path":"SKILL.md","content":"---\nname: s\ndescription: d\n---\n"}]}), Role::Agent, 3).unwrap();
    let role = send(s, json!({"op":"register_agent_role","key":"implementer","name":"Implementer","responsibilities":["r"],"inputs":["i"],"outputs":["o"],
        "skills":[skill["id"]],"runtime_compatibility":["claude-code"],"guidance":[{"document_id":"dos","mandatory":true}],"owner_direction":"bounded"}), Role::Agent, 4).unwrap();
    send(
        s,
        json!({"op":"activate_agent_role","id":role["id"],"note":"ok"}),
        Role::Human,
        5,
    )
    .unwrap();
    let m = send(s, json!({"op":"create_agent_member","name":"Claude","runtime":"claude-code","connection_ref":"local","product_ids":[dir, ins]}), Role::Human, 6).unwrap();
    verify(s, m["id"].as_str().unwrap(), 7);
    World {
        member: m["id"].as_str().unwrap().into(),
        role: role["id"].as_str().unwrap().into(),
    }
}

fn verify(s: &mut Store, member: &str, at: i64) {
    let v = snapshot(s, "agent_members")
        .into_iter()
        .find(|m| m["id"] == member)
        .unwrap()["version"]
        .clone();
    send(s, json!({"op":"record_member_capability","id":member,"expected_version":v,"harness_version":"2.1.289","verified_models":[MODEL],"evidence":"e"}), Role::Agent, at).unwrap();
}

fn dispatch_config(w: &World, product: &str) -> Value {
    json!({"product":product,"states":["ready"],"member_id":w.member,"role_id":w.role,"requested_model":MODEL,
        "policy":"dispatch_ready","objective":"Implement Ready work","trigger":{"kind":"daily","time":"09:00","timezone":"Asia/Kuala_Lumpur"},
        "limits":{"max_issues":5,"max_minutes":60},
        "dispatch":{"repository":"owner/repo","base_ref":"origin/main","checkout":"C:/repo","worktree_root":"C:/worktrees","allow_commands":["cargo test","npm test"]}})
}

/// Create, activate and run a routine now; returns the routine.
fn routine(s: &mut Store, config: Value, name: &str, at: i64) -> Value {
    let r = send(
        s,
        json!({"op":"create_routine","name":name,"config":config}),
        Role::Human,
        at,
    )
    .unwrap();
    let r = send(
        s,
        json!({"op":"set_routine_status","id":r["id"],"expected_version":1,"status":"active"}),
        Role::Human,
        at,
    )
    .unwrap();
    send(
        s,
        json!({"op":"run_routine_now","id":r["id"]}),
        Role::Human,
        at,
    )
    .unwrap();
    r
}

fn occurrences(s: &mut Store, routine: &Value) -> Vec<Value> {
    let mut o: Vec<Value> = snapshot(s, "routine_occurrences")
        .into_iter()
        .filter(|o| o["routine_id"] == routine["id"])
        .collect();
    o.sort_by_key(|o| o["created_at"].as_i64());
    o
}

fn run(s: &mut Store, id: &str) -> Value {
    snapshot(s, "agent_runs")
        .into_iter()
        .find(|r| r["id"] == id)
        .unwrap()
}

/// Start a dispatch run the way the runner does and return its version.
fn launch(s: &mut Store, id: &str, at: i64) -> Value {
    let r = run(s, id);
    let r = send(s, json!({"op":"start_agent_run","id":id,"expected_version":r["version"],"launcher":"pid 1 on test","harness_version":"2.1.289","credential_sha256":sha(TOKEN),
        "worktree":"C:/worktrees/dir-1","base_sha":BASE}), Role::Agent, at).unwrap();
    send(s, json!({"op":"record_run_session","id":id,"expected_version":r["version"],"session_id":id,"actual_model":MODEL}), Role::Agent, at).unwrap()
}

fn act(s: &mut Store, id: &str, index: u32, proposal: Value, at: i64) -> (String, String) {
    let v = send(
        s,
        json!({"op":"run_action","run_id":id,"credential":TOKEN,"index":index,"proposal":proposal}),
        Role::Agent,
        at,
    )
    .unwrap();
    (
        v["outcome"].as_str().unwrap().into(),
        v["detail"].as_str().unwrap().into(),
    )
}

fn finish(s: &mut Store, id: &str, at: i64) -> Value {
    let r = run(s, id);
    send(s, json!({"op":"finish_agent_run","id":id,"expected_version":r["version"],"succeeded":true,"summary":"done"}), Role::Agent, at).unwrap()
}

fn tick(s: &mut Store, at: i64) -> Vec<String> {
    s.tick_routines(at).unwrap()
}

#[test]
fn configuration_is_owner_bounded_and_dispatch_is_a_separate_policy() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);
    let create = |s: &mut Store, c: Value| {
        send(
            s,
            json!({"op":"create_routine","name":"x","config":c}),
            Role::Human,
            20,
        )
    };
    let mut c = dispatch_config(&w, "DIR");
    c["policy"] = json!("inspect_only");
    assert!(create(&mut s, c)
        .unwrap_err()
        .message
        .contains("Only the dispatch_ready"));
    let mut c = dispatch_config(&w, "DIR");
    c.as_object_mut().unwrap().remove("dispatch");
    assert!(create(&mut s, c)
        .unwrap_err()
        .message
        .contains("repository"));
    let mut c = dispatch_config(&w, "DIR");
    c["states"] = json!(["backlog"]);
    assert!(create(&mut s, c).unwrap_err().message.contains("Ready"));
    let mut c = dispatch_config(&w, "DIR");
    c["dispatch"]["allow_commands"] = json!(["gh pr merge"]);
    assert!(create(&mut s, c)
        .unwrap_err()
        .message
        .contains("cannot be allowed"));
    let mut c = dispatch_config(&w, "DIR");
    c["dispatch"]["worktree_root"] = json!("relative/path");
    assert!(create(&mut s, c).is_err());
    // Agents cannot configure dispatch; queue reviews cannot dispatch.
    assert!(send(
        &mut s,
        json!({"op":"create_routine","name":"x","config":dispatch_config(&w, "DIR")}),
        Role::Agent,
        20
    )
    .is_err());
    let key = issue(&mut s, "DIR", "Ready one", "medium", true);
    assert!(send(&mut s, json!({"op":"create_queue_run","product":"DIR","keys":[key],"member_id":w.member,"role_id":w.role,"requested_model":MODEL,"policy":"dispatch_ready","objective":"x"}), Role::Human, 21)
        .unwrap_err().message.contains("routine policy"));
    let ok = create(&mut s, dispatch_config(&w, "DIR")).unwrap();
    assert_eq!(ok["revisions"][0]["dispatch"]["checkout"], "C:/repo");
    assert_eq!(ok["status"], "paused");
}

#[test]
fn eligible_ready_work_dispatches_with_reasons_one_writer_per_product() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);
    let a = issue(&mut s, "DIR", "Urgent work", "high", true);
    let b = issue(&mut s, "DIR", "Second", "medium", true);
    let c = issue(&mut s, "DIR", "Third", "medium", true);
    let d = issue(&mut s, "DIR", "Fourth", "low", true);
    let open = issue(&mut s, "DIR", "Open dependency", "medium", false);
    let blocked = issue(&mut s, "DIR", "Blocked", "high", true);
    let v = version(&mut s, &blocked);
    send(&mut s, json!({"op":"create_issue_link","key":blocked,"expected_version":v,"target_key":open,"kind":"blocked_by"}), Role::Agent, 13).unwrap();
    // A stale guidance pin excludes its issue.
    let stale = issue(&mut s, "DIR", "Stale guidance", "high", true);
    pin(&mut s, &stale, 14);
    send(&mut s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":2,
        "documents":[{"id":"dos","title":"Workflow","category":"workflow","relative_path":"8.md","fingerprint":"b".repeat(64),"content":"y"}]}), Role::Agent, 15).unwrap();
    // An abandoned claim excludes its issue without blocking the lane.
    let abandoned = issue(&mut s, "DIR", "Abandoned", "medium", true);
    let v = version(&mut s, &abandoned);
    send(&mut s, json!({"op":"claim","actor":"gone-agent","key":abandoned,"expected_version":v,"lease_seconds":30}), Role::Agent, 16).unwrap();

    let preview = send(
        &mut s,
        json!({"op":"preview_routine","config":dispatch_config(&w, "DIR")}),
        Role::Human,
        100,
    )
    .unwrap();
    assert_eq!(preview["lane"], "ready");
    assert_eq!(preview["cap"], 3);
    assert_eq!(
        preview["keys"],
        json!([a, b, c]),
        "priority, then age; capped at three"
    );

    let r = routine(&mut s, dispatch_config(&w, "DIR"), "DIR lane", 100);
    let launched = tick(&mut s, 101);
    assert_eq!(launched.len(), 1);
    let o = occurrences(&mut s, &r).pop().unwrap();
    assert_eq!(o["lane"], "dispatched");
    assert_eq!(o["keys"], json!([a, b, c]));
    let reason = |key: &str| {
        o["excluded"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == key)
            .map(|e| e["reason"].as_str().unwrap().to_string())
            .unwrap_or_default()
    };
    assert!(reason(&blocked).contains(&format!("Blocked by {open}")));
    assert!(reason(&stale).contains("changed since it was linked"));
    assert!(reason(&abandoned).contains("expired"));
    assert_eq!(reason(&d), "", "left for a later session, not excluded");
    let dispatch_run = run(&mut s, &launched[0]);
    assert_eq!(dispatch_run["dispatch"]["cap"], 3);
    assert_eq!(dispatch_run["state"], "intent");
    assert!(dispatch_run["queue"].is_null());

    // A second occurrence waits for the writer: running, not a duplicate launch.
    send(
        &mut s,
        json!({"op":"run_routine_now","id":r["id"]}),
        Role::Human,
        102,
    )
    .unwrap();
    assert!(tick(&mut s, 103).is_empty());
    let o = occurrences(&mut s, &r).pop().unwrap();
    assert_eq!(o["state"], "deferred");
    assert_eq!(o["lane"], "running");
    assert!(o["reason"].as_str().unwrap().contains("still in flight"));

    // An independent product lane replenishes meanwhile.
    let ins = issue(&mut s, "INS", "Inspo work", "medium", true);
    let r2 = routine(&mut s, dispatch_config(&w, "INS"), "INS lane", 104);
    let launched2 = tick(&mut s, 105);
    assert_eq!(launched2.len(), 1);
    assert_eq!(
        occurrences(&mut s, &r2).pop().unwrap()["keys"],
        json!([ins])
    );

    // An active claim by any writer in the product also holds the lane.
    let preview = send(
        &mut s,
        json!({"op":"preview_routine","config":dispatch_config(&w, "INS")}),
        Role::Human,
        106,
    )
    .unwrap();
    assert_eq!(preview["lane"], "running");
}

#[test]
fn a_session_claims_reports_and_hands_off_but_cannot_merge_or_submit() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let w = world(&mut s);
    let a = issue(&mut s, "DIR", "First", "high", true);
    let b = issue(&mut s, "DIR", "Second", "medium", true);
    let c = issue(&mut s, "DIR", "Third", "low", true);
    routine(&mut s, dispatch_config(&w, "DIR"), "lane", 100);
    let id = tick(&mut s, 101).pop().unwrap();
    // The runner must start a dispatch run with its worktree.
    let r = run(&mut s, &id);
    assert!(send(&mut s, json!({"op":"start_agent_run","id":id,"expected_version":r["version"],"launcher":"pid 1","harness_version":"2","credential_sha256":sha(TOKEN)}), Role::Agent, 110)
        .unwrap_err().message.contains("worktree"));
    let started = launch(&mut s, &id, 110);
    assert_eq!(started["state"], "running");
    assert_eq!(started["dispatch"]["base_sha"], BASE);

    let mut i = 0;
    let mut next = |s: &mut Store, p: Value, at: i64| {
        i += 1;
        act(s, &id, i - 1, p, at)
    };
    assert_eq!(
        next(&mut s, json!({"op":"comment","key":a,"body":"hi"}), 111).0,
        "denied"
    );
    assert_eq!(
        next(&mut s, json!({"op":"claim","key":"DIR-999"}), 111).0,
        "denied"
    );
    let (outcome, _) = next(&mut s, json!({"op":"claim","key":a}), 112);
    assert_eq!(outcome, "applied");
    let ctx = context(&mut s, &a);
    assert_eq!(ctx["issue"]["status"], "doing");
    assert_eq!(ctx["issue"]["claim"]["actor"], format!("dispatch:{id}"));
    let (outcome, detail) = next(&mut s, json!({"op":"claim","key":b}), 113);
    assert_eq!(outcome, "denied");
    assert!(detail.contains(&format!("Report {a}")));
    for op in ["merge", "submit", "ready", "deliver"] {
        let (outcome, detail) = next(&mut s, json!({"op":op,"key":a}), 114);
        assert_eq!(outcome, "denied");
        assert!(detail.contains("Outside this session's authority"));
    }
    let branch = dispatch_branch(&a, &id);
    let report = |branch: &str, pr: &str| json!({"op":"report","key":a,"outcome":"pr_opened","branch":branch,"base_sha":BASE,"head_sha":HEAD,"pr_url":pr,"summary":"criteria met; cargo test passed"});
    assert!(next(
        &mut s,
        report("main", "https://github.com/owner/repo/pull/7"),
        115
    )
    .1
    .contains("pushed to dispatch/"));
    assert_eq!(
        next(
            &mut s,
            report(&branch, "https://github.com/other/repo/pull/7"),
            115
        )
        .0,
        "denied"
    );
    assert_eq!(
        next(
            &mut s,
            report(&branch, "https://github.com/owner/repo/pull/7"),
            116
        )
        .0,
        "applied"
    );
    let ctx = context(&mut s, &a);
    assert_eq!(
        ctx["git_traces"].as_array().unwrap().len(),
        2,
        "commit and push"
    );
    assert!(ctx["comments"].to_string().contains("Dispatch handoff"));
    assert_eq!(
        ctx["delivery"]["facts"][0]["detail"]["kind"],
        "pull_request"
    );
    assert_eq!(
        ctx["issue"]["claim"]["actor"],
        format!("dispatch:{id}"),
        "the claim stays for the coordinator"
    );

    // A changed issue is skipped as stale, never claimed at a newer version.
    let v = version(&mut s, &b);
    send(&mut s, json!({"op":"update_issue","key":b,"expected_version":v,"title":"Second (edited)","body":"Changed","acceptance":"It works","owner":"Owner","priority":"medium"}), Role::Agent, 117).unwrap();
    let (outcome, detail) = next(&mut s, json!({"op":"claim","key":b}), 118);
    assert_eq!(outcome, "denied");
    assert!(detail.starts_with("Stale"));
    assert_eq!(
        next(&mut s, json!({"op":"claim","key":c}), 119).0,
        "applied"
    );

    // The coordinator cannot take over while the session runs.
    let v = version(&mut s, &a);
    let takeover = json!({"op":"take_over_dispatch","actor":"claude-coordinator","run_id":id,"key":a,"expected_version":v,"lease_seconds":3600});
    assert!(send(&mut s, takeover.clone(), Role::Agent, 120)
        .unwrap_err()
        .message
        .contains("still running"));
    let rv = run(&mut s, &id)["version"].clone();
    let push = send(&mut s, json!({"op":"verify_dispatch_push","id":id,"expected_version":rv,"key":a,"verified":true,"detail":"origin is at head"}), Role::Agent, 121).unwrap();
    assert_eq!(push["dispatch"]["objectives"][0]["verified_push"], true);
    let done = finish(&mut s, &id, 122);
    let states: Vec<&str> = done["dispatch"]["objectives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["pr_opened", "skipped", "unreported"]);
    // After the session, the credential is dead and the lane stays single-writer.
    assert!(send(&mut s, json!({"op":"run_action","run_id":id,"credential":TOKEN,"index":50,"proposal":{"op":"renew","key":c}}), Role::Agent, 123).is_err());
    let preview = send(
        &mut s,
        json!({"op":"preview_routine","config":dispatch_config(&w, "DIR")}),
        Role::Human,
        124,
    )
    .unwrap();
    assert_eq!(preview["lane"], "running");
    assert!(preview["lane_detail"]
        .as_str()
        .unwrap()
        .contains("dispatch:"));

    let taken = send(&mut s, takeover, Role::Agent, 125).unwrap();
    assert_eq!(taken["claim"]["actor"], "claude-coordinator");
    let ctx = context(&mut s, &a);
    assert!(ctx["comments"]
        .to_string()
        .contains("Took over from dispatch session"));
    let v = version(&mut s, &a);
    assert!(send(&mut s, json!({"op":"take_over_dispatch","actor":"other","run_id":id,"key":a,"expected_version":v}), Role::Agent, 126).is_err(), "taken over once");

    // Persistence: restart and archive keep the dispatch record; old formats reject it.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 26);
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    let back = restored.export().unwrap();
    let find = |a: &Archive| {
        a.agent_runs
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .dispatch
            .clone()
    };
    assert_eq!(find(&back), find(&archive));
    let mut old = archive;
    old.format = 25;
    let mut again = Store::open(&dir.path().join("old")).unwrap();
    assert!(again.restore(old).is_err());
}

#[test]
fn review_repairs_cap_the_session_at_two_and_the_next_session_inherits_the_handoff() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);
    let repair = issue(&mut s, "DIR", "Repair", "high", true);
    let other = issue(&mut s, "DIR", "Other", "medium", true);
    let third = issue(&mut s, "DIR", "Third", "low", true);
    // Mark the first as needing repair after review (via archive, as review does).
    let mut archive = s.export().unwrap();
    archive
        .issues
        .iter_mut()
        .find(|i| i.key == repair)
        .unwrap()
        .needs_fix = true;
    let mut s = Store::open(&dir.path().join("db2")).unwrap();
    s.restore(archive).unwrap();
    // Restore marks the workspace restored now; later steps happen after it.
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 1_000;
    verify(&mut s, &w.member, t + 50);
    let preview = send(
        &mut s,
        json!({"op":"preview_routine","config":dispatch_config(&w, "DIR")}),
        Role::Human,
        t + 100,
    )
    .unwrap();
    assert_eq!(preview["cap"], 2);
    assert_eq!(preview["keys"], json!([repair, other]));

    routine(&mut s, dispatch_config(&w, "DIR"), "lane", t + 100);
    let first = tick(&mut s, t + 101).pop().unwrap();
    assert_eq!(
        run(&mut s, &first)["dispatch"]["objectives"][0]["review_repair"],
        true
    );
    launch(&mut s, &first, t + 102);
    act(
        &mut s,
        &first,
        0,
        json!({"op":"claim","key":repair}),
        t + 103,
    );
    act(
        &mut s,
        &first,
        1,
        json!({"op":"report","key":repair,"outcome":"failed","summary":"could not reproduce"}),
        t + 104,
    );
    // The model must be the requested one: a fresh session cannot reuse a run.
    finish(&mut s, &first, t + 105);
    let v = version(&mut s, &repair);
    send(&mut s, json!({"op":"take_over_dispatch","actor":"coordinator","run_id":first,"key":repair,"expected_version":v}), Role::Agent, t + 106).unwrap();
    let v = version(&mut s, &repair);
    send(
        &mut s,
        json!({"op":"release","actor":"coordinator","key":repair,"expected_version":v}),
        Role::Agent,
        t + 107,
    )
    .unwrap();

    let routine_id = snapshot(&mut s, "routines")[0]["id"].clone();
    send(
        &mut s,
        json!({"op":"run_routine_now","id":routine_id}),
        Role::Human,
        t + 108,
    )
    .unwrap();
    let second = tick(&mut s, t + 109).pop().unwrap();
    assert_ne!(second, first);
    let d = run(&mut s, &second)["dispatch"].clone();
    assert_eq!(d["handoff_from"], first);
    assert!(d["objectives"].to_string().contains(&other));
    assert_eq!(d["cap"], 2, "the released repair is still a repair");
    assert!(!d["objectives"]
        .to_string()
        .contains(&format!("\"{third}\"")));

    // Model substitution blocks the fresh session before any work.
    let r = run(&mut s, &second);
    let r = send(&mut s, json!({"op":"start_agent_run","id":second,"expected_version":r["version"],"launcher":"pid 2","harness_version":"2","credential_sha256":sha(TOKEN),"worktree":"C:/w/2","base_sha":BASE}), Role::Agent, t + 110).unwrap();
    let blocked = send(&mut s, json!({"op":"record_run_session","id":second,"expected_version":r["version"],"session_id":second,"actual_model":"claude-sonnet-5-5"}), Role::Agent, t + 111).unwrap();
    assert_eq!(blocked["state"], "blocked");
}

#[test]
fn shared_guidance_pins_travel_with_cross_product_objectives_and_unknown_runs_exclude() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);
    send(
        &mut s,
        json!({"op":"set_theoria_sharing","document_id":"dos","shared":true}),
        Role::Human,
        20,
    )
    .unwrap();
    let key = issue(&mut s, "INS", "Inspo with shared guidance", "medium", true);
    pin(&mut s, &key, 21);
    routine(&mut s, dispatch_config(&w, "INS"), "INS lane", 100);
    let id = tick(&mut s, 101).pop().unwrap();
    let pin = run(&mut s, &id)["dispatch"]["objectives"][0]["guidance"][0].clone();
    assert_eq!(pin["document_id"], "dos");
    assert_eq!(pin["shared"], true);
    assert_eq!(pin["recorded_fingerprint"], "a".repeat(64));

    // A lost session that cannot be reconciled excludes its objectives.
    launch(&mut s, &id, 102);
    act(&mut s, &id, 0, json!({"op":"claim","key":key}), 103);
    let r = run(&mut s, &id);
    send(&mut s, json!({"op":"mark_agent_run_unknown","id":id,"expected_version":r["version"],"reason":"lost"}), Role::Agent, 104).unwrap();
    let v = version(&mut s, &key);
    send(&mut s, json!({"op":"take_over_dispatch","actor":"coordinator","run_id":id,"key":key,"expected_version":v}), Role::Agent, 105).unwrap();
    let v = version(&mut s, &key);
    send(
        &mut s,
        json!({"op":"release","actor":"coordinator","key":key,"expected_version":v}),
        Role::Agent,
        106,
    )
    .unwrap();
    let preview = send(
        &mut s,
        json!({"op":"preview_routine","config":dispatch_config(&w, "INS")}),
        Role::Human,
        107,
    )
    .unwrap();
    assert_eq!(preview["lane"], "blocked");
    assert!(preview["excluded"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("ended unknown"));
}
