use super::*;

#[test]
fn empty_inventory_does_not_resurrect_legacy_items() {
    let folder = std::env::temp_dir().join(format!("velocityrl-inventory-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let active = folder.join("active.json");
    let legacy = folder.join("legacy.json");
    std::fs::write(&active, "[]").unwrap();
    std::fs::write(&legacy, r#"[{"wanted_id":11603,"paint_id":1}]"#).unwrap();
    assert!(load_swap_entries_from_paths(&[active.clone(), legacy.clone()]).is_empty());
    std::fs::remove_file(&active).unwrap();
    let migrated = load_swap_entries_from_paths(&[active, legacy.clone()]);
    assert_eq!(migrated[0]["wanted_id"], 11603);
    std::fs::remove_file(&legacy).unwrap();
    std::fs::remove_dir(&folder).unwrap();
}

#[test]
fn lobby_avatar_cache_busters_do_not_require_a_player_id() {
    assert!(is_local_avatar_route("/rlim/pfp", Some("t=123&size=256")));
    assert!(is_local_avatar_route("/rlim/avatar", Some("v=2")));
    assert!(!is_local_avatar_route("/rlim/pfp", Some("player_id=someone-else")));
    assert!(!is_local_avatar_route("/rlim/avatar.png", Some("%69d=someone-else")));
    assert!(!is_local_avatar_route("/avatars/someone-else", None));
}

#[test]
fn network_summaries_bound_payload_without_changing_the_capture() {
    let payload = "\u{1f680}".repeat(100_000);
    let event = NetworkEventEntry {
        id: 9, timestamp: "test".into(), protocol: "HTTP".into(), direction: "INBOUND".into(),
        service_or_url: "local-test".into(), status_or_type: "200".into(), summary: "test".into(),
        payload_len: payload.len(), payload_snippet: payload.clone(), intercepted: false,
    };
    let summary = network_event_summary(&event);
    assert_eq!(summary.payload_snippet.chars().count(), 512);
    assert_eq!(summary.payload_len, payload.len());
    assert_eq!(event.payload_snippet, payload);
}

#[test]
fn encoded_avatar_identity_matches_only_the_local_player() {
    let local = "Epic|98a256f4b1414fc3b557db5bdaf8043f|0";
    assert!(avatar_path_matches_player("/avatars/Epic%7C98a256f4b1414fc3b557db5bdaf8043f%7C0/256", local));
    assert!(!avatar_path_matches_player("/avatars/Epic%7Cother-account%7C0/256", local));
}
