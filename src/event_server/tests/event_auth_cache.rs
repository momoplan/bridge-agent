#[test]
fn event_auth_cache_reuses_a_matching_stamp_and_misses_when_it_changes() {
    use std::collections::HashSet;

    let cache = super::EventAuthCache::new();
    let config_stamp = super::FileStamp {
        modified: None,
        len: 10,
    };
    let token_stamp = super::FileStamp {
        modified: None,
        len: 4,
    };
    let record_stamp = super::FileStamp {
        modified: None,
        len: 8,
    };
    let mut events = HashSet::new();
    events.insert("messageReceived".to_string());
    cache.insert_for_test(
        config_stamp.clone(),
        crate::event_delivery::StoragePolicy::default(),
        "com.example.app",
        super::CachedApp {
            token_stamp: token_stamp.clone(),
            record_stamp: record_stamp.clone(),
            token: "secret".to_string(),
            events,
        },
    );

    assert!(matches!(
        cache.lookup(
            &config_stamp,
            "com.example.app",
            "messageReceived",
            "secret",
            Some(&token_stamp),
            Some(&record_stamp),
        ),
        super::AuthLookup::Authorized(_)
    ));
    match cache.lookup(
        &config_stamp,
        "com.example.app",
        "messageReceived",
        "other",
        Some(&token_stamp),
        Some(&record_stamp),
    ) {
        super::AuthLookup::Rejected(message) => {
            assert!(message.contains("invalid connector event credential"))
        }
        _ => panic!("mismatched token should be rejected from the cache"),
    }
    match cache.lookup(
        &config_stamp,
        "com.example.app",
        "otherEvent",
        "secret",
        Some(&token_stamp),
        Some(&record_stamp),
    ) {
        super::AuthLookup::Rejected(message) => assert!(message.contains("not declared")),
        _ => panic!("undeclared event should be rejected from the cache"),
    }

    let changed_config = super::FileStamp {
        modified: None,
        len: 11,
    };
    assert!(matches!(
        cache.lookup(
            &changed_config,
            "com.example.app",
            "messageReceived",
            "secret",
            Some(&token_stamp),
            Some(&record_stamp),
        ),
        super::AuthLookup::Miss
    ));
    let changed_token = super::FileStamp {
        modified: None,
        len: 5,
    };
    assert!(matches!(
        cache.lookup(
            &config_stamp,
            "com.example.app",
            "messageReceived",
            "secret",
            Some(&changed_token),
            Some(&record_stamp),
        ),
        super::AuthLookup::Miss
    ));

    cache.invalidate();
    assert!(matches!(
        cache.lookup(
            &config_stamp,
            "com.example.app",
            "messageReceived",
            "secret",
            Some(&token_stamp),
            Some(&record_stamp),
        ),
        super::AuthLookup::Miss
    ));
}
