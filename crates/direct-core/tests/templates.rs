//! DIR-22: owner-managed workspace intake templates with immutable revisions.
use direct_core::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn send(s: &mut Store, actor: &str, mut value: Value, role: Role) -> Result<Value> {
    value["actor"] = json!(actor);
    if value.get("request_id").is_none() {
        value["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    }
    s.execute_at(serde_json::from_value(value).unwrap(), role, 500)
}
fn owner(s: &mut Store, value: Value) -> Result<Value> {
    send(s, "owner", value, Role::Human)
}
fn agent(s: &mut Store, value: Value) -> Result<Value> {
    send(s, "template-agent", value, Role::Agent)
}
fn read(s: &mut Store, value: Value) -> Value {
    owner(s, value).unwrap()
}
fn code(result: Result<Value>) -> &'static str {
    result.unwrap_err().code
}

struct Workspace {
    _dir: TempDir,
    path: std::path::PathBuf,
    store: Store,
    alp: String,
    bet: String,
    label_triage: String,
    label_alpha: String,
}

fn workspace() -> Workspace {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("direct.db");
    let mut store = Store::open(&path).unwrap();
    let alp = owner(
        &mut store,
        json!({"op":"create_product","key":"ALP","name":"Synthetic Alpha"}),
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bet = owner(
        &mut store,
        json!({"op":"create_product","key":"BET","name":"Synthetic Beta"}),
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let label_triage = owner(&mut store, json!({"op":"create_label","name":"Triage"})).unwrap()
        ["id"]
        .as_str()
        .unwrap()
        .to_string();
    let label_alpha = owner(
        &mut store,
        json!({"op":"create_label","name":"Alpha only","products":[{"product_id":alp,"default_for_new_issues":false}]}),
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    Workspace {
        _dir: dir,
        path,
        store,
        alp,
        bet,
        label_triage,
        label_alpha,
    }
}

fn delivery_template(w: &mut Workspace) -> Value {
    owner(
        &mut w.store,
        json!({
            "op":"create_template",
            "target":"issue",
            "name":"Delivery",
            "shape":"delivery",
            "description":"Shared delivery intake",
            "content":{
                "intent":"Who gains what outcome?",
                "execution_mode":"agent",
                "boundaries":"Out of scope:",
                "verification":"Observable result at the real entrypoint",
                "checklist":["Acceptance per criterion","Failure path"],
                "suggested_priority":"high",
                "suggested_planning_scope":"project",
                "suggested_labels":[w.label_triage]
            },
            "supplements":[{"product_id":w.alp,"note":"Alpha ships behind its preview","checklist":["Preview link"],"suggested_labels":[w.label_alpha]}],
            "note":"First shared base"
        }),
    )
    .unwrap()
}

#[test]
fn owner_creates_and_revises_immutable_revisions_and_old_records_keep_v1() {
    let mut w = workspace();
    let created = delivery_template(&mut w);
    let id = created["template"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["template"]["current_revision"], 1);
    assert_eq!(created["template"]["status"], "active");
    assert_eq!(created["revision"]["revision"], 1);
    assert_eq!(created["revision"]["created_by"], "owner");
    let v1 = created["revision"].clone();

    // Shared use across two products; ALP gets its explicit supplement, BET the base only.
    let alp_issue = agent(
        &mut w.store,
        json!({"op":"create_issue","product":"ALP","title":"Alpha brief","body":"Intent","priority":"high","planning_scope":"project",
               "template":{"template_id":id,"revision":1,"labels":[w.label_triage,w.label_alpha]}}),
    )
    .unwrap();
    let used = &alp_issue["template"];
    assert_eq!(used["template_id"], json!(id));
    assert_eq!(used["revision"], 1);
    assert_eq!(used["supplement_product_id"], json!(w.alp));
    assert_eq!(used["execution_mode"], "agent");
    assert_eq!(used["overrides"], json!([]));
    assert_eq!(used["applied_by"], "template-agent");
    assert_eq!(alp_issue["labels"], json!([w.label_triage, w.label_alpha]));

    // Explicit overrides are recorded, never silently absorbed.
    let bet_issue = owner(
        &mut w.store,
        json!({"op":"create_issue","product":"BET","title":"Beta brief","priority":"low","planning_scope":"inbox",
               "template":{"template_id":id,"revision":1,"execution_mode":"prototype"}}),
    )
    .unwrap();
    let used = &bet_issue["template"];
    assert_eq!(used["supplement_product_id"], Value::Null);
    assert_eq!(used["execution_mode"], "prototype");
    assert_eq!(
        used["overrides"],
        json!(["priority", "planning_scope", "execution_mode", "labels"])
    );
    assert_eq!(bet_issue["labels"], json!([]));

    // Revision 2 is a new immutable row; revision 1 is byte-for-byte unchanged.
    let revised = owner(
        &mut w.store,
        json!({"op":"revise_template","id":id,"expected_version":1,"name":"Delivery brief","shape":"delivery",
               "content":{"intent":"Outcome and user","execution_mode":"paired","verification":"Owner entrypoint smoke"},
               "note":"Tighter prompts"}),
    )
    .unwrap();
    assert_eq!(revised["template"]["current_revision"], 2);
    assert_eq!(revised["template"]["name"], "Delivery brief");
    assert_eq!(revised["template"]["version"], 2);
    let listing = read(&mut w.store, json!({"op":"templates"}));
    let revisions = listing["template_revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0], v1);
    assert_eq!(revisions[1]["revision"], 2);

    // Existing records keep v1 and show as outdated; new work uses v2.
    let key = alp_issue["key"].as_str().unwrap();
    let context = read(&mut w.store, json!({"op":"context","key":key}));
    assert_eq!(context["issue"]["template"]["revision"], 1);
    assert_eq!(context["template"]["revision"], v1);
    assert_eq!(context["template"]["outdated"], true);
    assert_eq!(context["template"]["current_revision"], 2);
    // Editing the brief does not move it to the later revision.
    let updated = agent(
        &mut w.store,
        json!({"op":"update_issue","key":key,"expected_version":1,"title":"Alpha brief","body":"Edited","acceptance":"","owner":"","priority":"high"}),
    )
    .unwrap();
    assert_eq!(updated["template"]["revision"], 1);
    let stale = agent(
        &mut w.store,
        json!({"op":"create_issue","product":"ALP","title":"Stale form","template":{"template_id":id,"revision":1}}),
    );
    assert_eq!(code(stale), "conflict");
    let fresh = agent(
        &mut w.store,
        json!({"op":"create_issue","product":"ALP","title":"Fresh form","template":{"template_id":id,"revision":2}}),
    )
    .unwrap();
    assert_eq!(fresh["template"]["revision"], 2);
    // v2 has no supplement, so no supplement is claimed for ALP.
    assert_eq!(fresh["template"]["supplement_product_id"], Value::Null);
    let context = read(&mut w.store, json!({"op":"context","key":fresh["key"]}));
    assert_eq!(context["template"]["outdated"], false);

    // A revise with a stale version is refused.
    let conflict = owner(
        &mut w.store,
        json!({"op":"revise_template","id":id,"expected_version":1,"name":"X","shape":"bug","content":{"intent":"x"}}),
    );
    assert_eq!(code(conflict), "conflict");
}

#[test]
fn project_templates_apply_across_products_and_targets_are_not_interchangeable() {
    let mut w = workspace();
    let project_template = owner(
        &mut w.store,
        json!({"op":"create_template","target":"project","name":"Release train","shape":"release",
               "content":{"intent":"Release outcome","verification":"Production evidence","suggested_priority":"urgent","suggested_labels":[w.label_triage]},
               "supplements":[{"product_id":w.bet,"checklist":["Beta changelog"]}]}),
    )
    .unwrap();
    let id = project_template["template"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let alp = owner(
        &mut w.store,
        json!({"op":"create_project","product":"ALP","name":"Alpha release","priority":"urgent",
               "template":{"template_id":id,"revision":1,"labels":[w.label_triage]}}),
    )
    .unwrap();
    assert_eq!(alp["template"]["revision"], 1);
    assert_eq!(alp["template"]["supplement_product_id"], Value::Null);
    assert_eq!(alp["template"]["overrides"], json!([]));
    assert_eq!(alp["labels"], json!([w.label_triage]));
    assert_eq!(alp["status"], "planned");
    let bet = owner(
        &mut w.store,
        json!({"op":"create_project","product":"BET","name":"Beta release","priority":"medium",
               "template":{"template_id":id,"revision":1}}),
    )
    .unwrap();
    assert_eq!(bet["template"]["supplement_product_id"], json!(w.bet));
    assert_eq!(bet["template"]["overrides"], json!(["priority", "labels"]));

    // A project template cannot shape an issue and vice versa.
    assert_eq!(
        code(agent(
            &mut w.store,
            json!({"op":"create_issue","product":"ALP","title":"Wrong","template":{"template_id":id,"revision":1}}),
        )),
        "invalid"
    );
    let issue_template = delivery_template(&mut w);
    assert_eq!(
        code(owner(
            &mut w.store,
            json!({"op":"create_project","product":"ALP","name":"Wrong","template":{"template_id":issue_template["template"]["id"],"revision":1}}),
        )),
        "invalid"
    );
    // Project context exposes the project's provenance.
    let issue = owner(
        &mut w.store,
        json!({"op":"create_issue","product":"BET","title":"Inside","planning_scope":"project","project_id":bet["id"]}),
    )
    .unwrap();
    let context = read(&mut w.store, json!({"op":"context","key":issue["key"]}));
    assert_eq!(context["template"], Value::Null);
    assert_eq!(
        context["project_template"]["revision"]["name"],
        "Release train"
    );
}

#[test]
fn retired_templates_cannot_be_chosen_but_remain_readable() {
    let mut w = workspace();
    let created = delivery_template(&mut w);
    let id = created["template"]["id"].as_str().unwrap().to_string();
    let before = owner(
        &mut w.store,
        json!({"op":"create_issue","product":"BET","title":"Before retirement","template":{"template_id":id,"revision":1}}),
    )
    .unwrap();
    assert_eq!(
        code(owner(
            &mut w.store,
            json!({"op":"retire_template","id":id,"expected_version":1,"reason":"  "}),
        )),
        "invalid"
    );
    let retired = owner(
        &mut w.store,
        json!({"op":"retire_template","id":id,"expected_version":1,"reason":"Superseded by bug intake"}),
    )
    .unwrap();
    assert_eq!(retired["template"]["status"], "retired");
    assert_eq!(retired["template"]["retired_by"], "owner");
    assert_eq!(
        code(owner(
            &mut w.store,
            json!({"op":"retire_template","id":id,"expected_version":2,"reason":"again"}),
        )),
        "conflict"
    );
    assert_eq!(
        code(agent(
            &mut w.store,
            json!({"op":"create_issue","product":"BET","title":"After","template":{"template_id":id,"revision":1}}),
        )),
        "invalid"
    );
    assert_eq!(
        code(owner(
            &mut w.store,
            json!({"op":"revise_template","id":id,"expected_version":2,"name":"Back","shape":"delivery","content":{"intent":"x"}}),
        )),
        "invalid"
    );
    let snapshot = read(&mut w.store, json!({"op":"snapshot"}));
    assert_eq!(snapshot["templates"][0]["status"], "retired");
    assert_eq!(snapshot["template_revisions"].as_array().unwrap().len(), 1);
    let context = read(&mut w.store, json!({"op":"context","key":before["key"]}));
    assert_eq!(context["template"]["retired"], true);
    assert_eq!(context["template"]["revision"]["name"], "Delivery");
    assert_eq!(context["issue"]["template"]["revision"], 1);
}

#[test]
fn agents_cannot_mutate_definitions_and_templates_grant_nothing() {
    let mut w = workspace();
    let body = json!({"op":"create_template","target":"issue","name":"Agent","shape":"bug","content":{"intent":"x"}});
    assert_eq!(code(agent(&mut w.store, body)), "forbidden");
    let created = delivery_template(&mut w);
    let id = created["template"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        code(agent(
            &mut w.store,
            json!({"op":"revise_template","id":id,"expected_version":1,"name":"A","shape":"bug","content":{"intent":"x"}}),
        )),
        "forbidden"
    );
    assert_eq!(
        code(agent(
            &mut w.store,
            json!({"op":"retire_template","id":id,"expected_version":1,"reason":"agent"}),
        )),
        "forbidden"
    );
    // Agents can read and apply, and nothing becomes Ready, claimed or verified.
    let listing = agent(&mut w.store, json!({"op":"templates"})).unwrap();
    assert_eq!(listing["templates"][0]["version"], 1);
    let issue = agent(
        &mut w.store,
        json!({"op":"create_issue","product":"ALP","title":"Applied","body":"Intent","acceptance":"Checks","owner":"synthetic-owner",
               "template":{"template_id":id,"revision":1}}),
    )
    .unwrap();
    assert_eq!(issue["status"], "backlog");
    assert_eq!(issue["claim"], Value::Null);
    assert_eq!(issue["verification_key"], Value::Null);
    assert_eq!(issue["current_run"], Value::Null);
    let key = issue["key"].as_str().unwrap();
    assert_eq!(
        code(agent(
            &mut w.store,
            json!({"op":"ready","key":key,"expected_version":1}),
        )),
        "forbidden"
    );
    // A template checklist is not verification: readiness still needs a real brief.
    let thin = agent(
        &mut w.store,
        json!({"op":"create_issue","product":"ALP","title":"Thin","template":{"template_id":id,"revision":1}}),
    )
    .unwrap();
    assert_eq!(
        code(owner(
            &mut w.store,
            json!({"op":"ready","key":thin["key"],"expected_version":1}),
        )),
        "invalid"
    );
    let snapshot = read(&mut w.store, json!({"op":"snapshot"}));
    assert!(snapshot["review_ready_runs"].as_array().unwrap().is_empty());
    assert_eq!(
        read(&mut w.store, json!({"op":"export"}))["verifications"],
        json!([])
    );
    // An empty workspace is never seeded with templates.
    let empty = TempDir::new().unwrap();
    let mut fresh = Store::open(&empty.path().join("direct.db")).unwrap();
    let listing = read(&mut fresh, json!({"op":"templates"}));
    assert_eq!(listing["templates"], json!([]));
    assert_eq!(listing["template_revisions"], json!([]));
}

#[test]
fn invalid_products_supplements_versions_and_selections_are_rejected() {
    let mut w = workspace();
    let base = |supplements: Value, content: Value| json!({"op":"create_template","target":"issue","name":"Bug","shape":"bug","content":content,"supplements":supplements});
    let ok_content = json!({"intent":"What broke?","checklist":["Repro"]});
    let cases = [
        base(
            json!([{"product_id":"nope","note":"x"}]),
            ok_content.clone(),
        ),
        base(
            json!([{"product_id":w.alp,"note":"x"},{"product_id":w.alp,"note":"y"}]),
            ok_content.clone(),
        ),
        base(json!([{"product_id":w.alp}]), ok_content.clone()),
        base(
            json!([{"product_id":w.alp,"checklist":["repro"]}]),
            ok_content.clone(),
        ),
        base(
            json!([{"product_id":w.alp,"checklist":["a","b","c","d","e","f"]}]),
            ok_content.clone(),
        ),
        base(
            json!([{"product_id":w.bet,"suggested_labels":[w.label_alpha]}]),
            ok_content.clone(),
        ),
        base(json!([]), json!({"intent":""})),
        base(json!([]), json!({"intent":"x".repeat(2001)})),
        base(
            json!([]),
            json!({"intent":"x","suggested_priority":"someday"}),
        ),
        base(
            json!([]),
            json!({"intent":"x","suggested_labels":["missing"]}),
        ),
        base(
            json!([]),
            json!({"intent":"x","checklist":(0..13).map(|n| n.to_string()).collect::<Vec<_>>()}),
        ),
        json!({"op":"create_template","target":"project","name":"P","shape":"delivery","content":{"intent":"x","suggested_planning_scope":"inbox"}}),
    ];
    for case in cases {
        assert_eq!(code(owner(&mut w.store, case.clone())), "invalid", "{case}");
    }
    // A supplement cannot replace a base field: unknown fields are refused at parse time.
    let replace: std::result::Result<Request, _> = serde_json::from_value(json!({
        "actor":"owner","request_id":"r","op":"create_template","target":"issue","name":"Bug","shape":"bug",
        "content":{"intent":"x"},"supplements":[{"product_id":w.alp,"intent":"Replaced"}]
    }));
    assert!(replace.is_err());
    let unknown_shape: std::result::Result<Request, _> = serde_json::from_value(json!({
        "actor":"owner","request_id":"r","op":"create_template","target":"issue","name":"Bug","shape":"epic","content":{"intent":"x"}
    }));
    assert!(unknown_shape.is_err());
    assert_eq!(
        read(&mut w.store, json!({"op":"templates"}))["templates"],
        json!([])
    );

    let created = delivery_template(&mut w);
    let id = created["template"]["id"].as_str().unwrap().to_string();
    let selections = [
        (json!({"template_id":"missing","revision":1}), "not_found"),
        (json!({"template_id":id,"revision":7}), "conflict"),
        (json!({"template_id":id,"revision":0}), "conflict"),
        (
            json!({"template_id":id,"revision":1,"labels":["not-suggested"]}),
            "invalid",
        ),
        // ALP's supplement label is not a suggestion for BET.
        (
            json!({"template_id":id,"revision":1,"labels":[w.label_alpha]}),
            "invalid",
        ),
    ];
    for (selection, expected) in selections {
        assert_eq!(
            code(agent(
                &mut w.store,
                json!({"op":"create_issue","product":"BET","title":"Bad","template":selection}),
            )),
            expected
        );
    }
    assert_eq!(
        code(owner(
            &mut w.store,
            json!({"op":"revise_template","id":"missing","expected_version":1,"name":"x","shape":"bug","content":{"intent":"x"}}),
        )),
        "not_found"
    );
    // No refused selection wrote an issue.
    let snapshot = read(&mut w.store, json!({"op":"snapshot"}));
    assert_eq!(snapshot["issues"], json!([]));
}

#[test]
fn templates_persist_across_reopen_and_roundtrip_through_archives() {
    let mut w = workspace();
    let created = delivery_template(&mut w);
    let id = created["template"]["id"].as_str().unwrap().to_string();
    let issue = owner(
        &mut w.store,
        json!({"op":"create_issue","product":"ALP","title":"Kept","priority":"high","planning_scope":"project",
               "template":{"template_id":id,"revision":1,"labels":[w.label_triage,w.label_alpha]}}),
    )
    .unwrap();
    owner(
        &mut w.store,
        json!({"op":"revise_template","id":id,"expected_version":1,"name":"Delivery","shape":"delivery","content":{"intent":"v2"}}),
    )
    .unwrap();
    owner(
        &mut w.store,
        json!({"op":"retire_template","id":id,"expected_version":2,"reason":"Done"}),
    )
    .unwrap();
    let before = serde_json::to_value(w.store.export().unwrap()).unwrap();
    assert_eq!(before["format"], 13);
    assert_eq!(before["templates"].as_array().unwrap().len(), 1);
    assert_eq!(before["template_revisions"].as_array().unwrap().len(), 2);

    // Restart.
    drop(w.store);
    let reopened = Store::open(&w.path).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.export().unwrap()).unwrap(),
        before
    );

    // Export → restore → export is identical, including immutable provenance.
    let dir = TempDir::new().unwrap();
    let mut restored = Store::open(&dir.path().join("restored.db")).unwrap();
    restored.restore(reopened.export().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        before
    );
    let context = read(&mut restored, json!({"op":"context","key":issue["key"]}));
    assert_eq!(context["template"]["revision"]["revision"], 1);
    assert_eq!(context["template"]["outdated"], true);
    assert_eq!(context["template"]["retired"], true);

    // Invalid archives are refused.
    let archive = reopened.export().unwrap();
    let mut older = archive.clone();
    older.format = 12;
    assert_eq!(validate_archive(&older).unwrap_err().code, "invalid");
    let mut gap = archive.clone();
    gap.template_revisions.remove(0);
    assert!(validate_archive(&gap).is_err());
    let mut dangling = archive.clone();
    dangling.issues[0].template.as_mut().unwrap().revision = 9;
    assert!(validate_archive(&dangling).is_err());
    let mut supplement = archive.clone();
    supplement.issues[0]
        .template
        .as_mut()
        .unwrap()
        .supplement_product_id = None;
    assert!(validate_archive(&supplement).is_err());
    let mut wrong_target = archive.clone();
    wrong_target.templates[0].target = TemplateTarget::Project;
    assert!(validate_archive(&wrong_target).is_err());
    let mut half_retired = archive.clone();
    half_retired.templates[0].retired_reason = None;
    assert!(validate_archive(&half_retired).is_err());
    let mut renamed = archive.clone();
    renamed.templates[0].name = "Edited head".into();
    assert!(validate_archive(&renamed).is_err());
    let mut bad_override = archive;
    bad_override.issues[0]
        .template
        .as_mut()
        .unwrap()
        .overrides
        .push("status".into());
    assert!(validate_archive(&bad_override).is_err());
}

#[test]
fn format_12_archive_restores_and_legacy_requests_replay_with_unchanged_hashes() {
    // Generated by the pre-template build (main 832530e) with synthetic data only.
    let archive: Archive =
        serde_json::from_str(include_str!("fixtures/format12-synthetic.json")).unwrap();
    let source = serde_json::to_value(&archive).unwrap();
    assert_eq!(archive.format, 12);
    let replays: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/format12-synthetic-requests.json")).unwrap();
    let dir = TempDir::new().unwrap();
    let mut store = Store::open(&dir.path().join("direct.db")).unwrap();
    store.restore(archive.clone()).unwrap();
    let restored = serde_json::to_value(store.export().unwrap()).unwrap();
    // Only the format and the new empty collections differ.
    let mut expected = source.clone();
    expected["format"] = json!(13);
    expected["templates"] = json!([]);
    expected["template_revisions"] = json!([]);
    assert_eq!(restored, expected);
    // Untemplated records serialize exactly as before (no `template` key).
    assert!(restored["issues"][0].get("template").is_none());
    assert!(restored["projects"][0].get("template").is_none());

    for replay in &replays {
        let role = if replay["role"] == "human" {
            Role::Human
        } else {
            Role::Agent
        };
        let request: Request = serde_json::from_value(replay["request"].clone()).unwrap();
        let response = store.execute_at(request, role, 9_999).unwrap();
        assert_eq!(response, replay["response"], "exact legacy replay");
    }
    // The same ID with a changed payload is still refused.
    let mut changed = replays[3]["request"].clone();
    changed["title"] = json!("Different");
    assert_eq!(
        store
            .execute_at(serde_json::from_value(changed).unwrap(), Role::Agent, 9_999)
            .unwrap_err()
            .code,
        "conflict"
    );
    // A template in the same request ID is a different command.
    let mut templated = replays[3]["request"].clone();
    templated["template"] = json!({"template_id":"x","revision":1});
    assert_eq!(
        store
            .execute_at(
                serde_json::from_value(templated).unwrap(),
                Role::Agent,
                9_999
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    // Nothing was added by the replays.
    assert_eq!(
        serde_json::to_value(store.export().unwrap()).unwrap(),
        expected
    );
}

#[test]
fn clearing_a_suggested_execution_mode_is_explicit_and_recorded() {
    let mut w = workspace();
    let created = delivery_template(&mut w);
    let id = created["template"]["id"].as_str().unwrap().to_string();
    let create = |mode: Option<Value>| {
        let mut selection = json!({"template_id":id,"revision":1,"labels":[w.label_triage]});
        if let Some(mode) = mode {
            selection["execution_mode"] = mode;
        }
        json!({"op":"create_issue","product":"BET","title":"Mode","priority":"high","planning_scope":"project","template":selection})
    };
    // Absent: the suggestion applies and is not an override.
    let accepted = owner(&mut w.store, create(None)).unwrap();
    assert_eq!(accepted["template"]["execution_mode"], "agent");
    assert_eq!(accepted["template"]["overrides"], json!([]));
    // Explicitly choosing the suggested mode is not an override either.
    let same = owner(&mut w.store, create(Some(json!("agent")))).unwrap();
    assert_eq!(same["template"]["overrides"], json!([]));
    // Explicit null clears the suggestion and is recorded as the creator's choice.
    let cleared = owner(&mut w.store, create(Some(Value::Null))).unwrap();
    assert_eq!(cleared["template"]["execution_mode"], Value::Null);
    assert_eq!(cleared["template"]["overrides"], json!(["execution_mode"]));
    // The explicit clear survives reopen and context.
    drop(w.store);
    let mut reopened = Store::open(&w.path).unwrap();
    let context = read(&mut reopened, json!({"op":"context","key":cleared["key"]}));
    assert_eq!(context["issue"]["template"]["execution_mode"], Value::Null);
    assert_eq!(
        context["template"]["use"]["overrides"],
        json!(["execution_mode"])
    );
    // Absent and null are different commands for request-ID idempotency.
    let mut first = create(None);
    first["request_id"] = json!("mode-request");
    owner(&mut reopened, first).unwrap();
    let mut second = create(Some(Value::Null));
    second["request_id"] = json!("mode-request");
    assert_eq!(code(owner(&mut reopened, second)), "conflict");
}
