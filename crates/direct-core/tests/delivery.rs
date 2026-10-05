//! DIR-82: delivery ledger facts and their computed gap.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    if value.get("actor").is_none() {
        value["actor"] = json!(if role == Role::Human {
            "owner"
        } else {
            "coordinator"
        });
    }
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}
fn hash(c: char) -> String {
    c.to_string().repeat(64)
}

fn fact(s: &mut Store, key: &str, status: &str, detail: Value, at: i64) -> Result<Value> {
    send(
        s,
        json!({"op":"record_delivery_fact","key":key,"status":status,"detail":detail}),
        Role::Agent,
        at,
    )
}

fn gap(s: &mut Store, key: &str) -> Value {
    send(s, json!({"op":"context","key":key}), Role::Agent, 1).unwrap()["delivery"]["state"].clone()
}

fn issue(s: &mut Store) -> String {
    send(
        s,
        json!({"op":"create_issue","product":"DIR","title":"Ship it","planning_scope":"inbox"}),
        Role::Agent,
        1,
    )
    .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_full_chain_reaches_delivered_only_with_every_fact_and_merge_is_not_install() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let k = issue(&mut s);
    assert_eq!(gap(&mut s, &k)["gap"], "unknown");

    fact(&mut s, &k, "ok", json!({"kind":"worker","host":"PC","runtime":"claude-code","session_id":"sess-1","model":"claude-opus-5-5"}), 10).unwrap();
    fact(&mut s, &k, "ok", json!({"kind":"working_tree","checkout":"C:/wt","branch":"feature","head":sha('a'),"dirty_files":2}), 11).unwrap();
    assert_eq!(gap(&mut s, &k)["gap"], "unsaved");
    fact(&mut s, &k, "ok", json!({"kind":"working_tree","checkout":"C:/wt","branch":"feature","head":sha('a'),"dirty_files":0}), 12).unwrap();
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"commit","sha":sha('a'),"branch":"feature"}),
        13,
    )
    .unwrap();
    assert_eq!(gap(&mut s, &k)["gap"], "unpushed");
    // A PR or integration cannot be claimed from an unpushed commit.
    assert!(fact(&mut s, &k, "ok", json!({"kind":"pull_request","number":7,"url":"https://github.com/o/r/pull/7","target":"main","head_sha":sha('a')}), 14).unwrap_err().message.contains("pushed"));
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"push","sha":sha('a'),"remote":"origin","remote_ref":"refs/heads/feature"}),
        15,
    )
    .unwrap();
    assert_eq!(
        gap(&mut s, &k)["gap"],
        "unmerged",
        "a push is not integration"
    );
    fact(&mut s, &k, "ok", json!({"kind":"pull_request","number":7,"url":"https://github.com/o/r/pull/7","target":"main","head_sha":sha('a')}), 16).unwrap();
    fact(&mut s, &k, "ok", json!({"kind":"integration","method":"merge","sources":[sha('a')],"result":sha('b'),"target":"main"}), 17).unwrap();
    assert_eq!(gap(&mut s, &k)["gap"], "unverified");
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"check","commit":sha('b'),"name":"ci","outcome":"passed"}),
        18,
    )
    .unwrap();
    assert_eq!(gap(&mut s, &k)["gap"], "unbuilt", "a merge is not a build");
    let build = fact(&mut s, &k, "ok", json!({"kind":"build","commit":sha('b'),"dirty":"false","artifacts":[{"name":"direct.exe","sha256":hash('1')},{"name":"direct-desktop.exe","sha256":hash('2')}]}), 19).unwrap();
    assert_eq!(
        gap(&mut s, &k)["gap"],
        "uninstalled",
        "a build is not an install"
    );
    // Installing a file that matches no built artifact is rejected.
    assert!(fact(&mut s, &k, "ok", json!({"kind":"install","build_fact_id":build["id"],"path":"C:/d/direct.exe","sha256":hash('9')}), 20).unwrap_err().message.contains("matches none"));
    fact(&mut s, &k, "ok", json!({"kind":"install","build_fact_id":build["id"],"path":"C:/d/direct.exe","sha256":hash('1')}), 21).unwrap();
    assert_eq!(
        gap(&mut s, &k)["gap"],
        "unknown",
        "running not yet observed"
    );
    fact(&mut s, &k, "ok", json!({"kind":"running","path":"C:/d/direct.exe","sha256":hash('1'),"service_commit":sha('b'),"service_dirty":"false","bundle":"index-x.js","bundle_commit":sha('b')}), 22).unwrap();
    let state = gap(&mut s, &k);
    assert_eq!(state["gap"], "delivered");
    assert_eq!(state["last_known_good"]["commit"], sha('b'));

    // Later drift is exposed while the last known-good evidence is retained.
    fact(&mut s, &k, "ok", json!({"kind":"running","path":"C:/d/direct.exe","sha256":hash('7'),"service_commit":sha('b')}), 30).unwrap();
    let drift = gap(&mut s, &k);
    assert_eq!(drift["gap"], "drifted");
    assert_eq!(drift["last_known_good"]["sha256"], hash('1'));
    fact(&mut s, &k, "ok", json!({"kind":"running","path":"C:/d/direct.exe","sha256":hash('1'),"service_commit":sha('b'),"bundle_commit":sha('c')}), 31).unwrap();
    assert!(gap(&mut s, &k)["detail"]
        .as_str()
        .unwrap()
        .contains("UI bundle"));

    // Restart and archive recovery keep every fact.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 25);
    assert_eq!(archive.delivery_facts.len(), 13);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut old = archive.clone();
    old.format = 24;
    assert!(validate_archive(&old).is_err());
    let mut dangling = archive;
    for f in &mut dangling.delivery_facts {
        if let DeliveryDetail::Install { build_fact_id, .. } = &mut f.detail {
            *build_fact_id = "ghost".into();
        }
    }
    assert!(validate_archive(&dangling).is_err());
}

#[test]
fn squash_results_need_their_own_checks_and_failures_or_dirty_builds_are_honest() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let k = issue(&mut s);
    for (i, c) in ['a', 'b'].iter().enumerate() {
        fact(
            &mut s,
            &k,
            "ok",
            json!({"kind":"commit","sha":sha(*c),"branch":"feature"}),
            10 + i as i64,
        )
        .unwrap();
        fact(&mut s, &k, "ok", json!({"kind":"push","sha":sha(*c),"remote":"origin","remote_ref":"refs/heads/feature"}), 12 + i as i64).unwrap();
    }
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"check","commit":sha('b'),"name":"ci","outcome":"passed"}),
        14,
    )
    .unwrap();
    // A squash must name its new result commit, not reuse a source.
    assert!(fact(&mut s, &k, "ok", json!({"kind":"integration","method":"squash","sources":[sha('a'),sha('b')],"result":sha('b'),"target":"main"}), 15).unwrap_err().message.contains("new commit"));
    assert!(fact(&mut s, &k, "ok", json!({"kind":"integration","method":"rebase","sources":[sha('f')],"result":sha('g'),"target":"main"}), 15).unwrap_err().message.contains("never pushed"));
    fact(&mut s, &k, "ok", json!({"kind":"integration","method":"squash","sources":[sha('a'),sha('b')],"result":sha('c'),"target":"main"}), 16).unwrap();
    let state = gap(&mut s, &k);
    assert_eq!(state["gap"], "unverified");
    assert!(
        state["detail"]
            .as_str()
            .unwrap()
            .contains("source, not on the squash result"),
        "pre-merge checks do not verify the squash commit"
    );
    // A check on an unknown commit is rejected.
    assert!(fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"check","commit":sha('e'),"name":"ci","outcome":"passed"}),
        17
    )
    .unwrap_err()
    .message
    .contains("recorded"));
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"check","commit":sha('c'),"name":"ci","outcome":"passed"}),
        18,
    )
    .unwrap();
    // A dirty build is shown honestly, never as a clean commit build.
    fact(&mut s, &k, "ok", json!({"kind":"build","commit":sha('c'),"dirty":"true","artifacts":[{"name":"direct.exe","sha256":hash('1')}]}), 19).unwrap();
    assert!(gap(&mut s, &k)["detail"]
        .as_str()
        .unwrap()
        .contains("dirty"));
    // A failed build is the current gap until a later build succeeds.
    fact(&mut s, &k, "failed", json!({"kind":"build","commit":sha('c'),"dirty":"false","artifacts":[{"name":"direct.exe","sha256":hash('2')}]}), 20).unwrap();
    assert_eq!(gap(&mut s, &k)["gap"], "failed");
    let failed = send(&mut s, json!({"op":"context","key":k}), Role::Agent, 1).unwrap()["delivery"]
        ["facts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(
        fact(
            &mut s,
            &k,
            "ok",
            json!({"kind":"install","build_fact_id":failed["id"],"path":"C:/x","sha256":hash('2')}),
            21
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let good = fact(&mut s, &k, "ok", json!({"kind":"build","commit":sha('c'),"dirty":"false","artifacts":[{"name":"direct.exe","sha256":hash('3')}]}), 22).unwrap();
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"install","build_fact_id":good["id"],"path":"C:/x","sha256":hash('3')}),
        23,
    )
    .unwrap();
    // A running observation from before the install is stale.
    fact(
        &mut s,
        &k,
        "ok",
        json!({"kind":"running","path":"C:/x","sha256":hash('3'),"service_commit":sha('c')}),
        22,
    )
    .unwrap();
    assert_eq!(gap(&mut s, &k)["gap"], "stale");

    // Recording facts grants no authority: agents still cannot review or mark Ready.
    assert_eq!(
        send(
            &mut s,
            json!({"op":"ready","key":k,"expected_version":1}),
            Role::Agent,
            30
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let ctx = send(&mut s, json!({"op":"context","key":k}), Role::Agent, 31).unwrap();
    assert_eq!(ctx["issue"]["status"], "backlog");
    assert_eq!(
        ctx["issue"]["version"], 1,
        "ledger facts never touch the issue"
    );
    // Unknown fields and kinds are rejected.
    let parse = |v: Value| serde_json::from_value::<Request>(v).is_err();
    assert!(parse(
        json!({"op":"record_delivery_fact","actor":"a","request_id":"r","key":k,"status":"ok","detail":{"kind":"deploy_to_prod","target":"x"}})
    ));
    assert!(parse(
        json!({"op":"record_delivery_fact","actor":"a","request_id":"r","key":k,"status":"ok","detail":{"kind":"commit","sha":sha('a'),"branch":"b","force":true}})
    ));
}
