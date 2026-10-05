//! DIR-78: durable Product Manager routines on a controlled clock.
use direct_core::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const MODEL: &str = "claude-opus-5-5";
const TOKEN: &str = "routine-run-token-0123456789";
/// 2026-10-05T00:30:00Z = 08:30 in Kuala Lumpur (UTC+8).
const T0: i64 = 1_791_160_200;
const DAY: i64 = 86_400;

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

fn trigger(kind: &str, time: &str, weekday: Option<&str>, tz: &str) -> RoutineTrigger {
    RoutineTrigger {
        kind: kind.into(),
        time: time.into(),
        weekday: weekday.map(str::to_string),
        timezone: tz.into(),
    }
}

#[test]
fn due_times_follow_the_local_clock_and_daylight_saving() {
    let kl = trigger("daily", "09:00", None, "Asia/Kuala_Lumpur");
    assert_eq!(
        next_due(T0, &kl).unwrap(),
        T0 + 1_800,
        "09:00 KL is 01:00Z the same day"
    );
    assert_eq!(
        next_due(T0 + 1_800, &kl).unwrap(),
        T0 + 1_800 + DAY,
        "strictly after"
    );
    // 2026-10-05 is a Monday; weekly Wednesday 09:00 KL is two days later.
    let weekly = trigger("weekly", "09:00", Some("wed"), "Asia/Kuala_Lumpur");
    assert_eq!(next_due(T0, &weekly).unwrap(), T0 + 1_800 + 2 * DAY);
    // New York leaves daylight saving on 2026-11-01: 09:00 local moves from 13:00Z to 14:00Z.
    let ny = trigger("daily", "09:00", None, "America/New_York");
    let oct31 = 1_793_448_000; // 2026-10-31T12:00:00Z
    assert_eq!(
        next_due(oct31, &ny).unwrap(),
        1_793_451_600,
        "Oct 31 09:00 EDT = 13:00Z"
    );
    assert_eq!(
        next_due(1_793_451_600, &ny).unwrap(),
        1_793_541_600,
        "Nov 1 09:00 EST = 14:00Z"
    );
}

struct World {
    member: Value,
    role: Value,
    keys: Vec<String>,
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
    let member = send(s, json!({"op":"record_member_capability","id":m["id"],"expected_version":1,"harness_version":"2.1.289","verified_models":[MODEL],"evidence":"e"}), Role::Agent, 7).unwrap();
    let keys = (0..4)
        .map(|n| send(s, json!({"op":"create_issue","product":"DIR","title":format!("Backlog {n}"),"planning_scope":"inbox"}), Role::Agent, 10 + n).unwrap()["key"].as_str().unwrap().to_string())
        .collect();
    World { member, role, keys }
}

fn config(w: &World, cost: Option<f64>) -> Value {
    json!({"product":"DIR","states":["backlog"],"member_id":w.member["id"],"role_id":w.role["id"],"requested_model":MODEL,
        "policy":"inspect_only","objective":"Daily issue health","trigger":{"kind":"daily","time":"09:00","timezone":"Asia/Kuala_Lumpur"},
        "limits":{"max_issues":3,"max_minutes":10,"max_cost_usd":cost}})
}

fn runs(s: &mut Store) -> Vec<Value> {
    send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap()["agent_runs"]
        .as_array()
        .unwrap()
        .clone()
}

fn snapshot_list(s: &mut Store, field: &str) -> Vec<Value> {
    send(s, json!({"op":"snapshot"}), Role::Agent, 1).unwrap()[field]
        .as_array()
        .unwrap()
        .clone()
}

/// Drive a launched run to an end state with the given proposals.
fn complete(
    s: &mut Store,
    run_id: &str,
    proposals: &[Value],
    cost: Option<f64>,
    succeed: bool,
    at: i64,
) {
    let r = runs(s).into_iter().find(|r| r["id"] == run_id).unwrap();
    let r = send(s, json!({"op":"start_agent_run","id":run_id,"expected_version":r["version"],"launcher":"pid 1 on test","harness_version":"2.1.289","credential_sha256":sha(TOKEN)}), Role::Agent, at).unwrap();
    send(s, json!({"op":"record_run_session","id":run_id,"expected_version":r["version"],"session_id":run_id,"actual_model":MODEL}), Role::Agent, at).unwrap();
    for (index, p) in proposals.iter().enumerate() {
        send(s, json!({"op":"run_action","run_id":run_id,"credential":TOKEN,"index":index,"proposal":p}), Role::Agent, at).unwrap();
    }
    let r = send(
        s,
        json!({"op":"record_run_cost","id":run_id,"cost_usd":cost}),
        Role::Agent,
        at,
    )
    .unwrap();
    send(s, json!({"op":"finish_agent_run","id":run_id,"expected_version":r["version"],"succeeded":succeed,"summary":"done"}), Role::Agent, at).unwrap();
}

fn finding(key: &str) -> Value {
    json!({"op":"finding","issue_key":key,"kind":"missing_criteria","route":"bounded_brief","summary":"No criteria","evidence":["empty acceptance"],"recommendation":"Add criteria"})
}

#[test]
fn routines_schedule_one_occurrence_serialize_overlaps_and_coalesce_downtime() {
    let dir = TempDir::new().unwrap();
    let mut s = Store::open(&dir.path().join("db")).unwrap();
    let w = world(&mut s);

    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_routine","name":"Daily health","config":config(&w, None)}),
            Role::Agent,
            T0
        )
        .unwrap_err()
        .code,
        "forbidden"
    );
    let mut external = config(&w, None);
    external["notify"] = json!("slack");
    assert!(send(
        &mut s,
        json!({"op":"create_routine","name":"x","config":external}),
        Role::Human,
        T0
    )
    .unwrap_err()
    .message
    .contains("separate authorization"));
    let mut bad_tz = config(&w, None);
    bad_tz["trigger"]["timezone"] = json!("Mars/Olympus");
    assert_eq!(
        send(
            &mut s,
            json!({"op":"create_routine","name":"x","config":bad_tz}),
            Role::Human,
            T0
        )
        .unwrap_err()
        .code,
        "invalid"
    );

    let preview = send(
        &mut s,
        json!({"op":"preview_routine","config":config(&w, None)}),
        Role::Human,
        T0,
    )
    .unwrap();
    assert_eq!(
        preview["keys"].as_array().unwrap().len(),
        3,
        "max issues applies"
    );
    assert_eq!(preview["next_due_at"], T0 + 1_800);
    assert!(preview["dispatch_block"].is_null());

    let r = send(
        &mut s,
        json!({"op":"create_routine","name":"Daily health","config":config(&w, None)}),
        Role::Human,
        T0,
    )
    .unwrap();
    assert_eq!(r["status"], "paused", "created paused");
    assert!(
        s.tick_routines(T0 + DAY).unwrap().is_empty(),
        "paused routines never launch"
    );
    let r = send(
        &mut s,
        json!({"op":"set_routine_status","id":r["id"],"expected_version":1,"status":"active"}),
        Role::Human,
        T0,
    )
    .unwrap();
    assert_eq!(r["next_due_at"], T0 + 1_800);

    assert!(
        s.tick_routines(T0 + 1_799).unwrap().is_empty(),
        "not yet due"
    );
    let launched = s.tick_routines(T0 + 1_800).unwrap();
    assert_eq!(launched.len(), 1, "exactly one occurrence");
    assert!(
        s.tick_routines(T0 + 1_830).unwrap().is_empty(),
        "no duplicate on the next tick"
    );
    let run = runs(&mut s)
        .into_iter()
        .find(|x| x["id"] == launched[0].as_str())
        .unwrap();
    assert_eq!(run["queue"]["keys"].as_array().unwrap().len(), 3);
    assert_eq!(run["max_seconds"], 600);
    assert_eq!(
        run["created_by"].as_str().unwrap(),
        format!("routine:{}", r["id"].as_str().unwrap())
    );

    // The next day's slot arrives while that review is still running: deferred, not doubled.
    assert!(s.tick_routines(T0 + 1_800 + DAY).unwrap().is_empty());
    let occ = snapshot_list(&mut s, "routine_occurrences");
    assert_eq!(occ.iter().filter(|o| o["state"] == "deferred").count(), 1);
    // A quiet run (nothing new) creates no notice; the deferred one launches after it ends.
    complete(
        &mut s,
        &launched[0],
        &[],
        Some(0.05),
        true,
        T0 + 1_800 + DAY + 10,
    );
    let next = s.tick_routines(T0 + 1_800 + DAY + 30).unwrap();
    assert_eq!(next.len(), 1);
    assert!(
        snapshot_list(&mut s, "routine_notices").is_empty(),
        "quiet runs stay quiet"
    );
    // A new finding is noticed once; the same finding again is retained and quiet.
    complete(
        &mut s,
        &next[0],
        &[finding(&w.keys[3])],
        None,
        true,
        T0 + 1_800 + DAY + 40,
    );
    let notices = snapshot_list(&mut s, "routine_notices");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["kind"], "changed_finding");
    let unknown_cost = runs(&mut s)
        .into_iter()
        .find(|x| x["id"] == next[0].as_str())
        .unwrap();
    assert!(
        unknown_cost["cost_usd"].is_null(),
        "unknown cost stays unknown"
    );

    // Five days of downtime: one coalesced latest-state check, not a storm.
    let later = T0 + 1_800 + 6 * DAY + 3_600;
    let storm = s.tick_routines(later).unwrap();
    assert_eq!(storm.len(), 1);
    let coalesced = snapshot_list(&mut s, "routine_occurrences")
        .into_iter()
        .find(|o| o["run_id"] == storm[0].as_str())
        .unwrap();
    assert_eq!(
        coalesced["coalesced"], 4,
        "five missed slots folded into one"
    );
    complete(
        &mut s,
        &storm[0],
        &[finding(&w.keys[3])],
        None,
        true,
        later + 10,
    );
    assert_eq!(
        snapshot_list(&mut s, "routine_notices").len(),
        1,
        "retained finding adds no notice"
    );
    let routine = snapshot_list(&mut s, "routines")[0].clone();
    assert!(
        routine["next_due_at"].as_i64().unwrap() > later,
        "next due is after now"
    );
}

#[test]
fn pause_cancels_pending_member_disable_blocks_limits_hold_and_restore_requires_revalidation() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let w = world(&mut s);
    let r = send(
        &mut s,
        json!({"op":"create_routine","name":"Daily health","config":config(&w, Some(0.10))}),
        Role::Human,
        T0,
    )
    .unwrap();
    let r = send(
        &mut s,
        json!({"op":"set_routine_status","id":r["id"],"expected_version":1,"status":"active"}),
        Role::Human,
        T0,
    )
    .unwrap();
    let launched = s.tick_routines(T0 + 1_800).unwrap();
    // The run is in flight when the owner pauses: cancellation is pending until the runner acknowledges.
    let run = runs(&mut s)
        .into_iter()
        .find(|x| x["id"] == launched[0].as_str())
        .unwrap();
    let run = send(&mut s, json!({"op":"start_agent_run","id":run["id"],"expected_version":run["version"],"launcher":"pid 1 on test","harness_version":"2.1.289","credential_sha256":sha(TOKEN)}), Role::Agent, T0 + 1_810).unwrap();
    let r = send(&mut s, json!({"op":"set_routine_status","id":r["id"],"expected_version":r["version"],"status":"paused"}), Role::Human, T0 + 1_820).unwrap();
    let pending = runs(&mut s)
        .into_iter()
        .find(|x| x["id"] == run["id"])
        .unwrap();
    assert_eq!(pending["state"], "cancel_pending");
    assert!(
        s.tick_routines(T0 + 1_800 + 3 * DAY).unwrap().is_empty(),
        "paused: no launch"
    );
    send(&mut s, json!({"op":"acknowledge_run_cancel","id":pending["id"],"expected_version":pending["version"]}), Role::Agent, T0 + 1_830).unwrap();
    // Resume never backfills.
    let resumed_at = T0 + 1_800 + 3 * DAY + 100;
    let r = send(&mut s, json!({"op":"set_routine_status","id":r["id"],"expected_version":r["version"],"status":"active"}), Role::Human, resumed_at).unwrap();
    assert_eq!(r["next_due_at"], T0 + 1_800 + 4 * DAY);
    assert!(s.tick_routines(resumed_at + 10).unwrap().is_empty());

    // Over the cost limit: the routine holds and tells the owner.
    let costly = s.tick_routines(T0 + 1_800 + 4 * DAY).unwrap();
    complete(
        &mut s,
        &costly[0],
        &[],
        Some(0.42),
        true,
        T0 + 1_800 + 4 * DAY + 10,
    );
    let held = snapshot_list(&mut s, "routines")[0].clone();
    assert!(held["held_reason"]
        .as_str()
        .unwrap()
        .contains("over the $0.10 limit"));
    assert!(
        s.tick_routines(T0 + 1_800 + 5 * DAY).unwrap().is_empty(),
        "held: no launch"
    );
    assert!(snapshot_list(&mut s, "routine_notices")
        .iter()
        .any(|n| n["message"].as_str().unwrap().contains("cost limit")));

    // Reactivating clears the hold; a disabled member then blocks with a notice.
    let r = send(&mut s, json!({"op":"set_routine_status","id":r["id"],"expected_version":held["version"],"status":"active"}), Role::Human, T0 + 1_800 + 5 * DAY + 1).unwrap();
    let m = snapshot_list(&mut s, "agent_members")[0].clone();
    send(&mut s, json!({"op":"update_agent_member","id":m["id"],"expected_version":m["version"],"name":"Claude","enabled":false,"connection_ref":"local","product_ids":m["product_ids"]}), Role::Human, T0 + 1_800 + 5 * DAY + 2).unwrap();
    assert!(s.tick_routines(T0 + 1_800 + 6 * DAY).unwrap().is_empty());
    let blocked = snapshot_list(&mut s, "routine_occurrences")
        .into_iter()
        .filter(|o| o["state"] == "blocked")
        .count();
    assert_eq!(blocked, 1);
    assert!(snapshot_list(&mut s, "routine_notices")
        .iter()
        .any(|n| n["kind"] == "blocked" && n["message"].as_str().unwrap().contains("disabled")));
    // A failed run is a distinct notice with a recovery action.
    let m = snapshot_list(&mut s, "agent_members")[0].clone();
    send(&mut s, json!({"op":"update_agent_member","id":m["id"],"expected_version":m["version"],"name":"Claude","enabled":true,"connection_ref":"local","product_ids":m["product_ids"]}), Role::Human, T0 + 1_800 + 6 * DAY + 3).unwrap();
    send(
        &mut s,
        json!({"op":"run_routine_now","id":r["id"]}),
        Role::Human,
        T0 + 1_800 + 6 * DAY + 4,
    )
    .unwrap();
    let manual = s.tick_routines(T0 + 1_800 + 6 * DAY + 5).unwrap();
    complete(
        &mut s,
        &manual[0],
        &[],
        None,
        false,
        T0 + 1_800 + 6 * DAY + 6,
    );
    let failure = snapshot_list(&mut s, "routine_notices")
        .into_iter()
        .find(|n| n["kind"] == "failure" && n["message"].as_str().unwrap().contains("Failed"))
        .unwrap();
    assert!(failure["action"]
        .as_str()
        .unwrap()
        .contains("run the routine again"));
    send(
        &mut s,
        json!({"op":"acknowledge_routine_notice","id":failure["id"]}),
        Role::Human,
        T0 + 1_800 + 6 * DAY + 7,
    )
    .unwrap();

    // Restore keeps history and configuration, but the schedule stays paused
    // until reactivated, and reactivation needs a fresh capability check.
    drop(s);
    let s = Store::open(&path).unwrap();
    let archive = s.export().unwrap();
    assert_eq!(archive.format, 24);
    assert_eq!(archive.routines.len(), 1);
    validate_archive(&archive).unwrap();
    let mut restored = Store::open(&dir.path().join("restored")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    assert!(
        restored.tick_routines(now() + 30 * DAY).unwrap().is_empty(),
        "restored schedules stay paused"
    );
    let r = snapshot_list(&mut restored, "routines")[0].clone();
    let refused = send(&mut restored, json!({"op":"set_routine_status","id":r["id"],"expected_version":r["version"],"status":"active"}), Role::Human, now() + 60).unwrap_err();
    assert!(refused.message.contains("predates the last restore"));
    let mut old = archive;
    old.format = 23;
    assert!(validate_archive(&old).is_err());
}
