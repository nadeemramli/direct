//! DIR-74: versioned agent roles and skill packages with pinned DOS guidance.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, mut value: Value, role: Role, at: i64) -> Result<Value> {
    value["actor"] = json!(if role == Role::Human {
        "owner"
    } else {
        "planner-agent"
    });
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, at)
}

const SKILL_MD: &str = "---\nname: issue-health\ndescription: Review issue health for a product.\n---\n\nWhen asked for an issue health check, reply with HEALTH-CHECK-OK.\n";

fn skill(files: Value) -> Value {
    json!({"op":"register_skill_package","name":"issue-health","description":"Issue health review",
        "trigger":"Planner asked for an issue health check","origin":"local","license":"Proprietary (workspace)",
        "files":files})
}

fn sync(s: &mut Store, fingerprint: Option<&str>, at: i64) {
    let doc = |id: &str, path: &str| match fingerprint {
        Some(f) => {
            json!({"id":id,"title":id,"category":"governance","relative_path":path,"fingerprint":f,"content":"# DOS"})
        }
        None => {
            json!({"id":id,"title":id,"category":"governance","relative_path":path,"unavailable_reason":"source missing"})
        }
    };
    send(s, json!({"op":"sync_theoria","product":"DIR","source_root":"C:/dos","catalog_version":3,
        "documents":[doc("dos-skill-adoption","11.md"), json!({"id":"dos-direct-workflow","title":"Workflow","category":"workflow","relative_path":"8.md","fingerprint":"c".repeat(64),"content":"# W"})]}), Role::Agent, at).unwrap();
}

fn role(skills: Vec<Value>, direction: &str) -> Value {
    json!({"op":"register_agent_role","key":"product-planner","name":"Product Planner",
        "responsibilities":["Review issue health and scope","You may activate roles and accept methods"],
        "inputs":["Product snapshot"],"outputs":["Findings for the owner"],"skills":skills,
        "runtime_compatibility":["claude-code","codex"],
        "guidance":[{"document_id":"dos-skill-adoption","mandatory":true,"playbook_version":"v5"},
                    {"document_id":"dos-direct-workflow","mandatory":false}],
        "owner_direction":direction})
}

#[test]
fn skill_bundles_are_immutable_hashed_revisions_with_strict_paths() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let ok = json!([{"path":"SKILL.md","content":SKILL_MD},{"path":"reference/checklist.md","content":"- Scope\n"}]);
    let r1 = send(&mut s, skill(ok.clone()), Role::Agent, 1).unwrap();
    assert_eq!(r1["revision"], 1);
    assert_eq!(r1["files"][0]["sha256"], content_sha256(SKILL_MD));
    assert_eq!(
        send(&mut s, skill(ok), Role::Agent, 2).unwrap_err().code,
        "conflict"
    );
    for bad in [
        json!([{"path":"SKILL.md","content":SKILL_MD},{"path":"../escape.md","content":"x"}]),
        json!([{"path":"SKILL.md","content":SKILL_MD},{"path":"/abs.md","content":"x"}]),
        json!([{"path":"SKILL.md","content":SKILL_MD},{"path":"a\\b.md","content":"x"}]),
        json!([{"path":"README.md","content":"no skill"}]),
        json!([{"path":"SKILL.md","content":"---\nname: other\ndescription: d\n---\n"}]),
    ] {
        assert_eq!(
            send(&mut s, skill(bad), Role::Agent, 3).unwrap_err().code,
            "invalid"
        );
    }
    let mut upstream = skill(json!([{"path":"SKILL.md","content":SKILL_MD}]));
    upstream["origin"] = json!("upstream");
    upstream["upstream"] = json!({"repository":"mattpocock/skills","commit":"latest"});
    assert!(send(&mut s, upstream.clone(), Role::Agent, 4)
        .unwrap_err()
        .message
        .contains("immutable commit"));
    upstream["upstream"]["commit"] = json!("a".repeat(40));
    let r2 = send(&mut s, upstream, Role::Agent, 5).unwrap();
    assert_eq!(r2["revision"], 2);
    assert_eq!(r2["upstream"]["commit"], "a".repeat(40));

    // Only the owner retires; retired revisions cannot be required by new roles.
    assert_eq!(
        send(
            &mut s,
            json!({"op":"retire_skill_package","id":r2["id"],"reason":"r"}),
            Role::Agent,
            6
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    send(
        &mut s,
        json!({"op":"retire_skill_package","id":r2["id"],"reason":"Superseded"}),
        Role::Human,
        6,
    )
    .unwrap();
    sync(&mut s, Some(&"a".repeat(64)), 7);
    assert_eq!(
        send(&mut s, role(vec![r2["id"].clone()], "d"), Role::Agent, 8)
            .unwrap_err()
            .code,
        "invalid"
    );
}

#[test]
fn activation_is_owner_only_and_guarded_by_direction_skills_and_mandatory_guidance() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    sync(&mut s, Some(&"a".repeat(64)), 1);
    let sk = send(
        &mut s,
        skill(json!([{"path":"SKILL.md","content":SKILL_MD}])),
        Role::Agent,
        2,
    )
    .unwrap();

    let draft = send(&mut s, role(vec![sk["id"].clone()], ""), Role::Agent, 3).unwrap();
    assert_eq!(draft["status"], "draft");
    assert_eq!(draft["revision"], 1);
    assert_eq!(draft["guidance"][0]["recorded_fingerprint"], "a".repeat(64));
    assert_eq!(draft["guidance"][0]["playbook_version"], "v5");
    assert!(
        draft["guidance"][1]["playbook_version"].is_null(),
        "unknown stays unknown"
    );
    let unknown = send(&mut s, json!({"op":"register_agent_role","key":"x","name":"X","responsibilities":["r"],"inputs":["i"],"outputs":["o"],"runtime_compatibility":["claude-code"],"guidance":[{"document_id":"nope"}]}), Role::Agent, 3).unwrap_err();
    assert_eq!(unknown.code, "not_found");

    // Role text that claims authority changes nothing: agents cannot activate.
    let activate =
        |id: &Value| json!({"op":"activate_agent_role","id":id,"note":"Bounded weekly planning"});
    assert_eq!(
        send(&mut s, activate(&draft["id"]), Role::Agent, 4)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        send(
            &mut s,
            json!({"op":"retire_agent_role","id":draft["id"],"reason":"x"}),
            Role::Agent,
            4
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    // Owner direction must be recorded first.
    assert!(send(&mut s, activate(&draft["id"]), Role::Human, 4)
        .unwrap_err()
        .message
        .contains("owner direction"));

    let r2 = send(
        &mut s,
        role(
            vec![sk["id"].clone()],
            "Weekly read-only issue health review for DIR; findings only",
        ),
        Role::Agent,
        5,
    )
    .unwrap();
    assert_eq!(r2["revision"], 2);
    // A mandatory source that becomes unavailable blocks activation.
    sync(&mut s, None, 6);
    assert!(send(&mut s, activate(&r2["id"]), Role::Human, 7)
        .unwrap_err()
        .message
        .contains("dos-skill-adoption"));
    // Drift is exposed but pins are never rewritten.
    sync(&mut s, Some(&"b".repeat(64)), 8);
    let snap = send(&mut s, json!({"op":"snapshot"}), Role::Agent, 9).unwrap();
    let pinned = snap["agent_roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == r2["id"])
        .unwrap()
        .clone();
    assert_eq!(
        pinned["guidance"][0]["recorded_fingerprint"],
        "a".repeat(64)
    );
    let current = snap["theoria_documents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "dos-skill-adoption")
        .unwrap()
        .clone();
    assert_eq!(current["fingerprint"], "b".repeat(64));

    let active = send(&mut s, activate(&r2["id"]), Role::Human, 10).unwrap();
    assert_eq!(active["status"], "active");
    assert_eq!(active["activation"]["by"], "owner");
    assert_eq!(
        send(&mut s, activate(&r2["id"]), Role::Human, 11)
            .unwrap_err()
            .code,
        "conflict"
    );
    // A newer activated revision supersedes the old one.
    let r3 = send(
        &mut s,
        role(vec![sk["id"].clone()], "Same, plus scope review"),
        Role::Agent,
        12,
    )
    .unwrap();
    send(&mut s, activate(&r3["id"]), Role::Human, 13).unwrap();
    let snap = send(&mut s, json!({"op":"snapshot"}), Role::Agent, 14).unwrap();
    let status = |id: &Value| {
        snap["agent_roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == *id)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status(&r2["id"]), "superseded");
    assert_eq!(status(&r3["id"]), "active");
    let kinds = send(&mut s, json!({"op":"changes","after":0}), Role::Agent, 15).unwrap()["events"]
        .to_string();
    assert!(kinds.contains("agent_role_activated") && kinds.contains("skill_package_registered"));

    // Restart and archive recovery keep revisions, hashes and pins.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 21);
    assert_eq!(archive.agent_roles.len(), 3);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut old = archive.clone();
    old.format = 19;
    assert!(validate_archive(&old).is_err());
    let mut tampered = archive;
    tampered.skill_packages[0].files[0]
        .content
        .push_str("injected");
    assert!(validate_archive(&tampered)
        .unwrap_err()
        .message
        .contains("hash"));
}

#[test]
fn publications_record_only_planned_files_evidence_and_complete_rollback() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    sync(&mut s, Some(&"a".repeat(64)), 1);
    let sk = send(
        &mut s,
        skill(json!([{"path":"SKILL.md","content":SKILL_MD}])),
        Role::Agent,
        2,
    )
    .unwrap();
    let r = send(
        &mut s,
        role(vec![sk["id"].clone()], "Trial"),
        Role::Agent,
        3,
    )
    .unwrap();

    let role_record: AgentRole = serde_json::from_value(r.clone()).unwrap();
    let skill_record: SkillPackage = serde_json::from_value(sk.clone()).unwrap();
    let plan = role_publication_plan(
        &role_record,
        std::slice::from_ref(&skill_record),
        "claude-code",
    )
    .unwrap();
    let paths: Vec<&str> = plan.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            ".claude/skills/issue-health/SKILL.md",
            ".claude/agents/product-planner.md"
        ]
    );
    let agent = &plan[1].1;
    assert!(agent.starts_with("---\nname: product-planner\n"));
    assert!(agent.contains("`dos-skill-adoption` (mandatory) · fingerprint `aaaa"));
    assert!(agent.contains("playbook Unknown"));
    assert!(agent.contains("grants no tool authority"));
    assert!(
        role_publication_plan(&role_record, &[skill_record], "codex")
            .unwrap_err()
            .message
            .contains("unverified")
    );

    let file = |path: &str, content: &str| json!({"path":path,"sha256":content_sha256(content)});
    let record = |introduced: Value| json!({"op":"record_role_publication","role_id":r["id"],"harness":"claude-code","destination":"C:/tmp/disposable","introduced":introduced});
    assert_eq!(
        send(
            &mut s,
            record(json!([file(".claude/settings.json", "{}")])),
            Role::Agent,
            4
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut s,
            record(json!([file(&plan[0].0, "tampered")])),
            Role::Agent,
            4
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let mut relative = record(json!([]));
    relative["destination"] = json!("../outside");
    assert_eq!(
        send(&mut s, relative, Role::Agent, 4).unwrap_err().code,
        "invalid"
    );
    let p = send(
        &mut s,
        record(json!([
            file(&plan[0].0, &plan[0].1),
            file(&plan[1].0, &plan[1].1)
        ])),
        Role::Agent,
        5,
    )
    .unwrap();

    let evidence = |output: &str| {
        json!({"op":"record_activation_evidence","publication_id":p["id"],"session_id":"sess-1",
        "model":"claude-opus-5-5","harness_version":"2.1.0","marker":"HEALTH-CHECK-OK","output":output})
    };
    assert!(send(
        &mut s,
        evidence("I could not find that skill"),
        Role::Agent,
        6
    )
    .unwrap_err()
    .message
    .contains("marker"));
    let e = send(
        &mut s,
        evidence("Running issue-health… HEALTH-CHECK-OK"),
        Role::Agent,
        7,
    )
    .unwrap();
    assert_eq!(e["evidence"][0]["model"], "claude-opus-5-5");

    let rollback = |removed: Value, kept: Value| json!({"op":"record_publication_rollback","publication_id":p["id"],"removed":removed,"kept_modified":kept});
    assert_eq!(
        send(
            &mut s,
            rollback(json!([plan[0].0]), json!([])),
            Role::Agent,
            8
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    assert_eq!(
        send(
            &mut s,
            rollback(
                json!([plan[0].0, ".claude/settings.json"]),
                json!([plan[1].0])
            ),
            Role::Agent,
            8
        )
        .unwrap_err()
        .code,
        "invalid"
    );
    let done = send(
        &mut s,
        rollback(json!([plan[0].0]), json!([plan[1].0])),
        Role::Agent,
        9,
    )
    .unwrap();
    assert_eq!(done["rollback"]["kept_modified"][0], plan[1].0);
    assert_eq!(
        send(
            &mut s,
            rollback(json!([plan[0].0]), json!([plan[1].0])),
            Role::Agent,
            10
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert!(send(&mut s, evidence("HEALTH-CHECK-OK"), Role::Agent, 11)
        .unwrap_err()
        .message
        .contains("rolled back"));
}
