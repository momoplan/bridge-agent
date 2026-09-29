use super::*;
use relay::contracts::{
    channel::{AppPayload, DeliveryReceipt},
    routing::{EventSelector, Selection},
};

fn identity() -> QueueIdentity {
    QueueIdentity {
        environment: "test-environment".into(),
        workspace_id: 42,
        device_id: "device".into(),
    }
}
fn event(id: &str) -> LocalAppEventEmitted {
    LocalAppEventEmitted {
        event_id: id.into(),
        app_id: "connector".into(),
        event: "output".into(),
        payload: AppPayload(serde_json::json!({"text":"value"})),
        occurred_at: Some("1970-01-01T00:00:00Z".into()),
        target_consumers: vec![],
    }
}
fn snapshot(groups: &[&str]) -> DeviceSubscriptions {
    DeviceSubscriptions {
        contract_version: Default::default(),
        workspace_id: 42,
        device_id: "device".into(),
        revision: "test-snapshot".into(),
        groups: groups
            .iter()
            .map(|id| SubscriptionGroup {
                consumer_id: (*id).into(),
                subscriptions: vec![EventSelector {
                    workspaces: Selection::All,
                    devices: Selection::All,
                    apps: Selection::All,
                    events: Selection::All,
                }],
            })
            .collect(),
    }
}
fn ack(id: &str, group: &str, receipt: DeliveryReceipt) -> EventAck {
    EventAck {
        event_id: id.into(),
        app_id: "connector".into(),
        receipts: vec![GroupReceipt {
            consumer_id: group.into(),
            receipt,
        }],
    }
}

#[tokio::test]
async fn accepted_event_survives_restart_and_waits_for_fresh_subscriptions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.sqlite");
    let policy = StoragePolicy::default();
    {
        let queue = EventQueue::open(&path, identity()).unwrap();
        assert_eq!(
            queue
                .admit(event("a"), policy.clone())
                .await
                .unwrap()
                .status,
            LocalAcceptance::Queued
        );
    }
    let queue = EventQueue::open(&path, identity()).unwrap();
    assert!(queue.due(policy.clone(), 16).await.unwrap().is_empty());
    queue
        .subscriptions(Some(snapshot(&["worker"])))
        .await
        .unwrap();
    let due = queue.due(policy.clone(), 16).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].event_id, "a");
    assert_eq!(due[0].target_consumers, ["worker"]);
    queue
        .acknowledge(ack(
            "a",
            "worker",
            DeliveryReceipt::Processed { duplicate: false },
        ))
        .await
        .unwrap();
    assert_eq!(queue.statistics(policy).await.unwrap().pending_events, 0);
}

#[test]
fn partial_receipts_keep_original_targets_across_subscription_changes() {
    let dir = tempfile::tempdir().unwrap();
    let q = EventQueue::open(&dir.path().join("q"), identity()).unwrap();
    let mut s = q.inner.lock().unwrap();
    let p = StoragePolicy::default();
    s.subscriptions = Some(snapshot(&["first", "second"]));
    s.admit(event("a"), &p, 100).unwrap();
    s.acknowledge(
        ack("a", "first", DeliveryReceipt::Processed { duplicate: true }),
        101,
    )
    .unwrap();
    s.subscriptions = Some(snapshot(&["third"]));
    let due = s.due(&p, 200, 16).unwrap();
    assert_eq!(due[0].target_consumers, ["second"]);
    s.acknowledge(
        ack(
            "a",
            "second",
            DeliveryReceipt::Rejected {
                reason: "retry".into(),
            },
        ),
        201,
    )
    .unwrap();
    assert_eq!(s.statistics(&p, 201).unwrap().pending_events, 1);
    s.acknowledge(ack("a", "second", DeliveryReceipt::Ignored), 202)
        .unwrap();
    assert_eq!(s.statistics(&p, 202).unwrap().pending_events, 0);
}

#[test]
fn cleanup_defaults_to_seven_days_uses_local_receipt_time_and_previews_impact() {
    let dir = tempfile::tempdir().unwrap();
    let q = EventQueue::open(&dir.path().join("q"), identity()).unwrap();
    let mut s = q.inner.lock().unwrap();
    let p = StoragePolicy::default();
    assert!(p.automatic_cleanup);
    assert_eq!(p.retention_days, 7);
    s.admit(event("a"), &p, 100).unwrap();
    assert_eq!(
        s.statistics(&p, 100 + 7 * 86400 - 1)
            .unwrap()
            .expiring_events,
        0
    );
    assert_eq!(
        s.statistics(&p, 100 + 7 * 86400).unwrap().expiring_events,
        1
    );
    let off = StoragePolicy {
        automatic_cleanup: false,
        ..p.clone()
    };
    s.cleanup(&off, 100 + 8 * 86400).unwrap();
    assert_eq!(
        s.statistics(&off, 100 + 8 * 86400).unwrap().pending_events,
        1
    );
    s.cleanup(&p, 100 + 8 * 86400).unwrap();
    let stats = s.statistics(&p, 100 + 8 * 86400).unwrap();
    assert_eq!(stats.pending_events, 0);
    assert_eq!(stats.last_cleanup_expired, 1);
    let bodies: u64 =
        s.db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
    assert_eq!(bodies, 0);
}

#[test]
fn capacity_backpressure_does_not_evict_accepted_events_and_duplicates_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let q = EventQueue::open(&dir.path().join("q"), identity()).unwrap();
    let mut s = q.inner.lock().unwrap();
    let p = StoragePolicy {
        capacity_bytes: 1024 * 1024,
        ..StoragePolicy::default()
    };
    let mut large = event("a");
    large.payload = AppPayload(serde_json::json!({"text":"x".repeat(700_000)}));
    s.admit(large.clone(), &p, 100).unwrap();
    s.admit(large.clone(), &p, 101).unwrap();
    large.event_id = "b".into();
    assert!(s.admit(large, &p, 102).is_err());
    assert!(s.admit(event("a"), &p, 103).is_ok());
    assert_eq!(s.statistics(&p, 103).unwrap().pending_events, 1);
}

#[tokio::test]
async fn confirmed_empty_subscription_is_different_from_unknown_and_device_identity_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let q = EventQueue::open(&dir.path().join("q"), identity()).unwrap();
    let p = StoragePolicy::default();
    let mut wrong = snapshot(&[]);
    wrong.workspace_id = 99;
    assert!(q.subscriptions(Some(wrong)).await.is_err());
    q.subscriptions(Some(snapshot(&[]))).await.unwrap();
    assert_eq!(
        q.admit(event("a"), p.clone()).await.unwrap().status,
        LocalAcceptance::NoSubscribers
    );
    q.subscriptions(None).await.unwrap();
    assert_eq!(
        q.admit(event("b"), p.clone()).await.unwrap().status,
        LocalAcceptance::Queued
    );
    assert_eq!(q.statistics(p).await.unwrap().pending_events, 1);
}

#[tokio::test]
async fn another_login_cannot_send_or_acknowledge_previous_identity_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("q");
    let p = StoragePolicy::default();
    let q = EventQueue::open(&path, identity()).unwrap();
    q.subscriptions(Some(snapshot(&["worker"]))).await.unwrap();
    q.admit(event("a"), p.clone()).await.unwrap();
    let mut next = identity();
    next.environment = "another".into();
    let q2 = EventQueue::open(&path, next).unwrap();
    q2.subscriptions(Some(snapshot(&["worker"]))).await.unwrap();
    assert!(q2.due(p.clone(), 16).await.unwrap().is_empty());
    q2.acknowledge(ack("a", "worker", DeliveryReceipt::Ignored))
        .await
        .unwrap();
    assert_eq!(q.statistics(p).await.unwrap().pending_events, 1);
}
