use direct_core::*;
use serde_json::{json, Value};

fn send(store: &mut Store, mut command: Value) -> Result<Value> {
    command["actor"] = json!("intake-test");
    command["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    store.execute_at(serde_json::from_value(command).unwrap(), Role::Agent, 100)
}
fn fixture() -> Value {
    json!({"text":"Location: sidebar. Group projects into Teroka. Keep issue status unchanged.","images":[{
        "data_url":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6ioAAAAASUVORK5CYII=",
        "caption":"Current sidebar"}]})
}
#[test]
fn original_context_survives_edits_reopen_and_archive_without_polling_image_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store.db");
    let mut store = Store::open(&path).unwrap();
    let source = fixture();
    let created = send(&mut store,json!({"op":"create_issue","product":"DIR","title":"Sidebar groups","body":"Shaped brief","intake":source})).unwrap();
    assert_eq!(created["intake"], source);
    assert_eq!(created["status"], "backlog");
    send(&mut store,json!({"op":"update_issue","key":"DIR-1","expected_version":1,"title":"Sidebar groups","body":"Edited brief","acceptance":"Persist groups","owner":"tester","priority":"medium"})).unwrap();
    assert_eq!(
        send(&mut store, json!({"op":"context","key":"DIR-1"})).unwrap()["issue"]["intake"],
        source
    );
    assert!(
        send(&mut store, json!({"op":"snapshot"})).unwrap()["issues"][0]
            .get("intake")
            .is_none()
    );
    let archive = store.export().unwrap();
    assert_eq!(archive.format, 24);
    drop(store);
    let mut reopened = Store::open(&path).unwrap();
    assert_eq!(
        send(&mut reopened, json!({"op":"context","key":"DIR-1"})).unwrap()["issue"]["intake"],
        source
    );
    let mut restored = Store::open(&temp.path().join("restore.db")).unwrap();
    restored.restore(archive.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.export().unwrap()).unwrap(),
        serde_json::to_value(&archive).unwrap()
    );
    let mut mislabeled = archive;
    mislabeled.format = 13;
    let mut refused = Store::open(&temp.path().join("refused.db")).unwrap();
    assert!(refused.restore(mislabeled).is_err());
    let cleared = send(&mut restored,json!({"op":"update_issue","key":"DIR-1","expected_version":2,"title":"Sidebar groups","body":"Edited brief","acceptance":"Persist groups","owner":"tester","priority":"medium","intake":{}})).unwrap();
    assert_eq!(cleared["intake"], json!({"text":"","images":[]}));
}

#[test]
fn refuses_unsafe_oversized_or_mistyped_images_and_excess_context() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("store.db")).unwrap();
    let mut bad = fixture();
    bad["images"][0]["data_url"] = json!("https://example.com/tracking.png");
    assert!(send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Bad URL","intake":bad})
    )
    .is_err());
    let mut bad = fixture();
    bad["images"][0]["data_url"] = json!("data:image/png;base64,aGVsbG8=");
    assert!(send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Bad bytes","intake":bad})
    )
    .is_err());
    let mut bad = fixture();
    bad["text"] = json!("x".repeat(40_001));
    assert!(send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Too long","intake":bad})
    )
    .is_err());
    let mut bad = fixture();
    bad["images"] = json!(vec![bad["images"][0].clone(); 4]);
    assert!(send(
        &mut store,
        json!({"op":"create_issue","product":"DIR","title":"Too many","intake":bad})
    )
    .is_err());
    assert!(
        send(&mut store, json!({"op":"snapshot"})).unwrap()["issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
