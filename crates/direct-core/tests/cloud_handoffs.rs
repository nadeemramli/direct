//! DIR-58: bounded cloud handoff packets and coordinator reconciliation.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

const MODEL: &str = "claude-opus-5-5";

fn send(s: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    if value.get("actor").is_none() {
        value["actor"] = json!(if role == Role::Human {
            "owner"
        } else {
            "coord"
        });
    }
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

fn version(s: &mut Store, key: &str) -> u64 {
    send(s, json!({"op":"context","key":key}), Role::Agent, 1).unwrap()["issue"]["version"]
        .as_u64()
        .unwrap()
}

/// A Ready issue in `product`, claimed by the coordinator, with one guidance pin.
fn assigned(s: &mut Store, product: &str, doc: &str, title: &str) -> String {
    let i = send(
        s,
        json!({"op":"create_issue","product":product,"title":title,"planning_scope":"inbox"}),
        Role::Agent,
        10,
    )
    .unwrap();
    let key = i["key"].as_str().unwrap().to_string();
    send(s, json!({"op":"update_issue","key":key,"expected_version":1,"title":title,"body":format!("Brief for {title}"),"acceptance":format!("Acceptance for {title}"),"owner":"Private Owner","priority":"high"}), Role::Agent, 11).unwrap();
    send(
        s,
        json!({"op":"ready","key":key,"expected_version":2}),
        Role::Human,
        12,
    )
    .unwrap();
    send(
        s,
        json!({"op":"claim","key":key,"expected_version":3,"lease_seconds":86400}),
        Role::Agent,
        13,
    )
    .unwrap();
    send(s, json!({"op":"link_theoria","key":key,"expected_version":4,"document_id":doc,"playbook_version":"v5"}), Role::Agent, 14).unwrap();
    key
}

fn prepare(key: &str, v: u64, repository: &str) -> Value {
    json!({"op":"prepare_cloud_handoff","key":key,"expected_version":v,"repository":repository,
        "base_ref":"main","required_model":MODEL,"evidence_plan":"Unit tests and fixture E2E per criterion",
        "constraints":["Keep the change behind the existing owner gate"]})
}

fn pushed(s: &mut Store, key: &str, repository: &str, sha: &str) {
    for kind in ["commit", "push"] {
        let v = version(s, key);
        let mut trace = json!({"op":"record_git_trace","key":key,"expected_version":v,"kind":kind,
            "repository":repository,"commit_sha":sha,"branch":"claude/cloud"});
        if kind == "push" {
            trace["remote"] = json!("origin");
            trace["remote_ref"] = json!("refs/heads/claude/cloud");
        }
        send(s, trace, Role::Agent, 30).unwrap();
    }
}

fn reconcile(id: &Value, v: u64, issue_v: u64, sha: &str, verdict: &str) -> Value {
    json!({"op":"reconcile_cloud_handoff","id":id,"expected_version":v,"issue_expected_version":issue_v,
        "session_id":"session_01ABC","model":MODEL,"pr_url":"https://github.com/acme/widgets/pull/42",
        "tested_sha":sha,"cloud_verdict":verdict,"summary":"Cloud run finished",
        "checks":[{"name":"cargo test","outcome":"passed","environment":"cloud"}]})
}

fn submission(key: &str, v: u64, sha: &str) -> Value {
    let build = format!("commit:{sha}");
    json!({"op":"submit","key":key,"expected_version":v,"build_ref":build,"delivery_ref":"main","summary":"s","checks":"c",
        "e2e":{"build_ref":build,"delivered_build_ref":build,"environment":"local fixture","entrypoint":"owner UI",
            "scenarios":"all criteria","outcome":"passed","delivery_check":"installed and smoke-tested locally"},
        "steps":[{"instruction":"i","expected":"e"}]})
}

fn sync(s: &mut Store, product: &str, doc: &str) {
    send(s, json!({"op":"sync_theoria","product":product,"source_root":"C:/private/dos","catalog_version":1,
        "documents":[{"id":doc,"title":"Workflow","category":"workflow","relative_path":"8. Workflow.md",
        "fingerprint":"a".repeat(64),"content":"# Private cached content"}]}), Role::Agent, 5).unwrap();
}

#[test]
fn packets_are_bounded_per_product_and_never_carry_private_data() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    send(
        &mut s,
        json!({"op":"create_product","key":"INS","name":"Inspo"}),
        Role::Human,
        1,
    )
    .unwrap();
    send(
        &mut s,
        json!({"op":"update_product_paths","product":"DIR","repo_windows":"C:/secret/checkout"}),
        Role::Human,
        2,
    )
    .unwrap();
    sync(&mut s, "DIR", "dir-workflow");
    sync(&mut s, "INS", "ins-workflow");
    let unrelated = assigned(&mut s, "DIR", "dir-workflow", "Unrelated secret work");
    let a = assigned(&mut s, "DIR", "dir-workflow", "Direct cloud task");
    let b = assigned(&mut s, "INS", "ins-workflow", "Inspo cloud task");
    let v = version(&mut s, &a);
    send(
        &mut s,
        json!({"op":"comment","key":a,"expected_version":v,"body":"Private discussion"}),
        Role::Agent,
        15,
    )
    .unwrap();

    for (key, product, doc) in [(&a, "DIR", "dir-workflow"), (&b, "INS", "ins-workflow")] {
        let v = version(&mut s, key);
        let h = send(&mut s, prepare(key, v, "acme/widgets"), Role::Agent, 20).unwrap();
        // Preparing never modifies the issue.
        assert_eq!(version(&mut s, key), v);
        let p = &h["packet"];
        assert_eq!(h["status"], "prepared");
        assert_eq!(p["issue_key"], *key);
        assert_eq!(p["product_key"], product);
        assert_eq!(p["issue_version"], v);
        assert_eq!(p["claim_actor"], "coord");
        assert_eq!(p["repository"], "acme/widgets");
        assert_eq!(p["required_model"], MODEL);
        assert_eq!(p["guidance"][0]["document_id"], doc);
        assert_eq!(p["guidance"][0]["recorded_fingerprint"], "a".repeat(64));
        assert_eq!(p["guidance"][0]["playbook_version"], "v5");
        let fields: Vec<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            fields,
            [
                "acceptance",
                "base_ref",
                "body",
                "claim_actor",
                "claim_expires_at",
                "constraints",
                "evidence_plan",
                "format",
                "guidance",
                "issue_key",
                "issue_version",
                "product_key",
                "repository",
                "required_model",
                "title"
            ]
        );
        let markdown = cloud_packet_markdown(&serde_json::from_value(h.clone()).unwrap());
        for text in [p.to_string(), markdown.clone()] {
            for private in [
                "Unrelated secret work",
                "Private discussion",
                "Private Owner",
                "C:/secret",
                "C:/private",
                "Private cached content",
                "#grant=",
                "/api/",
                "127.0.0.1",
                "localhost",
                &unrelated,
            ] {
                assert!(!text.contains(private), "packet leaked {private}");
            }
        }
        assert!(
            markdown.contains(MODEL)
                && markdown.contains("acme/widgets")
                && markdown.contains("no Direct service address")
        );
    }
}

#[test]
fn reconciliation_requires_matching_issue_model_pr_and_pushed_sha() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    sync(&mut s, "DIR", "dir-workflow");
    let key = assigned(&mut s, "DIR", "dir-workflow", "Cloud task");

    // Preparing needs a coordinator claim, a valid route, and one open packet.
    let other = send(
        &mut s,
        json!({"op":"create_issue","product":"DIR","title":"Backlog","planning_scope":"inbox"}),
        Role::Agent,
        10,
    )
    .unwrap();
    assert_eq!(
        send(
            &mut s,
            prepare(other["key"].as_str().unwrap(), 1, "acme/widgets"),
            Role::Human,
            20
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let v = version(&mut s, &key);
    let mut intruder = prepare(&key, v, "acme/widgets");
    intruder["actor"] = json!("someone-else");
    assert_eq!(
        send(&mut s, intruder, Role::Agent, 20).unwrap_err().code,
        "claim_required"
    );
    assert_eq!(
        send(&mut s, prepare(&key, v, "not a repo"), Role::Agent, 20)
            .unwrap_err()
            .code,
        "invalid"
    );
    let h = send(&mut s, prepare(&key, v, "acme/widgets"), Role::Agent, 20).unwrap();
    assert_eq!(
        send(&mut s, prepare(&key, v, "acme/widgets"), Role::Agent, 21)
            .unwrap_err()
            .code,
        "conflict"
    );
    let id = h["id"].clone();

    // An open handoff blocks submission.
    let sha = "b".repeat(40);
    assert!(send(&mut s, submission(&key, v, &sha), Role::Agent, 22)
        .unwrap_err()
        .message
        .contains("Reconcile or withdraw"));

    // The tested SHA must be pushed to the packet repository first.
    let refused = send(
        &mut s,
        reconcile(&id, 1, v, &sha, "passed"),
        Role::Agent,
        25,
    )
    .unwrap_err();
    assert!(refused.message.contains("record-push"));
    pushed(&mut s, &key, "acme/widgets", &sha);
    let v = version(&mut s, &key);

    let mut wrong_model = reconcile(&id, 1, v, &sha, "passed");
    wrong_model["model"] = json!("claude-sonnet-5-5");
    assert!(send(&mut s, wrong_model, Role::Agent, 31)
        .unwrap_err()
        .message
        .contains("requires claude-opus-5-5"));
    let mut wrong_pr = reconcile(&id, 1, v, &sha, "passed");
    wrong_pr["pr_url"] = json!("https://github.com/evil/fork/pull/1");
    assert_eq!(
        send(&mut s, wrong_pr, Role::Agent, 31).unwrap_err().code,
        "invalid"
    );
    let mut unpushed = reconcile(&id, 1, v, &"c".repeat(40), "passed");
    unpushed["request_id"] = json!("other-sha");
    assert_eq!(
        send(&mut s, unpushed, Role::Agent, 31).unwrap_err().code,
        "invalid"
    );
    let delivered = send(
        &mut s,
        reconcile(&id, 1, v, &sha, "delivered"),
        Role::Agent,
        31,
    )
    .unwrap_err();
    assert!(delivered.message.contains("cannot become a delivered Pass"));
    let mut failed_check = reconcile(&id, 1, v, &sha, "passed");
    failed_check["checks"][0]["outcome"] = json!("failed");
    assert_eq!(
        send(&mut s, failed_check, Role::Agent, 31)
            .unwrap_err()
            .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut s,
            reconcile(&id, 2, v, &sha, "passed"),
            Role::Agent,
            31
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(
        send(
            &mut s,
            reconcile(&id, 1, v - 1, &sha, "passed"),
            Role::Agent,
            31
        )
        .unwrap_err()
        .code,
        "conflict"
    );

    // Success, exact retry replays, a second reconciliation conflicts.
    let mut ok = reconcile(&id, 1, v, &sha.to_uppercase(), "passed");
    ok["request_id"] = json!("reconcile-1");
    let done = send(&mut s, ok.clone(), Role::Agent, 32).unwrap();
    assert_eq!(done["status"], "reconciled");
    assert_eq!(done["version"], 2);
    assert_eq!(done["reconciliation"]["tested_sha"], sha);
    assert_eq!(done["reconciliation"]["model"], MODEL);
    assert_eq!(done["reconciliation"]["checks"][0]["environment"], "cloud");
    assert_eq!(send(&mut s, ok, Role::Agent, 33).unwrap(), done);
    assert_eq!(
        send(
            &mut s,
            reconcile(&id, 2, v, &sha, "passed"),
            Role::Agent,
            33
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    // Reconciliation leaves the issue untouched; local submission is still required.
    assert_eq!(version(&mut s, &key), v);
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 34).unwrap();
    assert_eq!(ctx["issue"]["status"], "doing");
    assert_eq!(ctx["cloud_handoffs"][0]["id"], id);
    let submitted = send(&mut s, submission(&key, v, &sha), Role::Agent, 35).unwrap();
    assert_eq!(submitted["status"], "verify");

    // Restart and archive recovery keep the handoff, packet hash and evidence.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 24);
    assert_eq!(archive.cloud_handoffs.len(), 1);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut old = archive.clone();
    old.format = 18;
    assert!(validate_archive(&old).is_err());
    let mut tampered = archive;
    tampered.cloud_handoffs[0].packet.acceptance = "Rewritten".into();
    assert!(validate_archive(&tampered)
        .unwrap_err()
        .message
        .contains("packet hash"));
}

#[test]
fn stale_briefs_failed_cloud_builds_and_withdrawals_are_enforced() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    sync(&mut s, "DIR", "dir-workflow");

    // A brief edited after prepare makes the packet stale.
    let key = assigned(&mut s, "DIR", "dir-workflow", "Changing brief");
    let v = version(&mut s, &key);
    let h = send(&mut s, prepare(&key, v, "acme/widgets"), Role::Human, 20).unwrap();
    assert_eq!(h["prepared_by"], "owner");
    let sha = "d".repeat(40);
    pushed(&mut s, &key, "acme/widgets", &sha);
    let v = version(&mut s, &key);
    send(&mut s, json!({"op":"update_issue","key":key,"expected_version":v,"title":"Changing brief","body":"New scope","acceptance":"Different","owner":"Private Owner","priority":"high"}), Role::Agent, 40).unwrap();
    let v = version(&mut s, &key);
    let stale = send(
        &mut s,
        reconcile(&h["id"], 1, v, &sha, "passed"),
        Role::Agent,
        41,
    )
    .unwrap_err();
    assert!(stale
        .message
        .contains("changed after this packet was prepared"));
    // Withdraw needs a reason; then a fresh packet can be prepared.
    assert_eq!(
        send(
            &mut s,
            json!({"op":"withdraw_cloud_handoff","id":h["id"],"expected_version":1,"reason":" "}),
            Role::Agent,
            42
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let w = send(&mut s, json!({"op":"withdraw_cloud_handoff","id":h["id"],"expected_version":1,"reason":"Brief changed"}), Role::Agent, 42).unwrap();
    assert_eq!(w["status"], "withdrawn");
    assert_eq!(send(&mut s, json!({"op":"withdraw_cloud_handoff","id":h["id"],"expected_version":2,"reason":"again"}), Role::Agent, 43).unwrap_err().code, "conflict");
    let fresh = send(&mut s, prepare(&key, v, "acme/widgets"), Role::Agent, 44).unwrap();
    assert_eq!(fresh["packet"]["acceptance"], "Different");

    // A blocked cloud result for a build prevents submitting that build.
    send(
        &mut s,
        reconcile(&fresh["id"], 1, v, &sha, "blocked"),
        Role::Agent,
        45,
    )
    .unwrap();
    let refused = send(&mut s, submission(&key, v, &sha), Role::Agent, 46).unwrap_err();
    assert!(refused.message.contains("did not pass"));

    // The claim the packet was prepared under must still be held.
    let other = assigned(&mut s, "DIR", "dir-workflow", "Lost claim");
    let v = version(&mut s, &other);
    let h = send(&mut s, prepare(&other, v, "acme/widgets"), Role::Agent, 20).unwrap();
    let sha = "e".repeat(40);
    pushed(&mut s, &other, "acme/widgets", &sha);
    let v = version(&mut s, &other);
    let expired = send(
        &mut s,
        reconcile(&h["id"], 1, v, &sha, "passed"),
        Role::Human,
        200_000,
    )
    .unwrap_err();
    assert!(expired.message.contains("no longer holds the claim"));

    // Every handoff transition is recorded in the change feed.
    let kinds = send(&mut s, json!({"op":"changes","after":0}), Role::Agent, 50).unwrap()["events"]
        .to_string();
    for kind in [
        "cloud_handoff_prepared",
        "cloud_handoff_reconciled",
        "cloud_handoff_withdrawn",
    ] {
        assert!(kinds.contains(kind), "missing {kind}");
    }

    // Handoffs are retained evidence: even back in Backlog the issue cannot be
    // deleted around them.
    let v = version(&mut s, &key);
    send(&mut s, json!({"op":"cancel_issue","key":key,"expected_version":v,"reason":"Not needed","release_active_claim":true}), Role::Human, 60).unwrap();
    send(
        &mut s,
        json!({"op":"reopen","key":key,"expected_version":v + 1,"reason":"Check deletion"}),
        Role::Human,
        61,
    )
    .unwrap();
    let ctx = send(&mut s, json!({"op":"context","key":key}), Role::Agent, 62).unwrap();
    assert_eq!(ctx["issue"]["status"], "backlog");
    let blockers = ctx["deletion"]["blockers"].as_array().unwrap();
    let cloud = blockers
        .iter()
        .find(|b| b["kind"] == "cloud_handoffs")
        .unwrap();
    assert_eq!(cloud["count"], 2);
    assert_eq!(ctx["deletion"]["eligible"], false);
}
