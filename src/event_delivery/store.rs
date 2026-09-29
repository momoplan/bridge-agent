use super::*;
use relay::contracts::channel::DeliveryReceipt;
use rusqlite::{params, OptionalExtension, Transaction};

impl Store {
    pub(super) fn admit(
        &mut self,
        mut event: LocalAppEventEmitted,
        policy: &StoragePolicy,
        now: i64,
    ) -> Result<LocalEventAccepted> {
        policy.validate()?;
        if !relay::routing::valid_identity(&event.event_id)
            || !relay::routing::valid_identity(&event.app_id)
            || !relay::routing::valid_identity(&event.event)
        {
            bail!("invalid event identity");
        }
        event.target_consumers.clear();
        self.cleanup(policy, now)?;
        let body = serde_json::to_string(&event)?;
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT body FROM events WHERE scope=?1 AND app_id=?2 AND event_id=?3",
                params![self.scope, event.app_id, event.event_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            // The first durable record owns this event identity. A source replay
            // may carry a later observation timestamp; never replace the pending body.
            return Ok(accepted(&event, LocalAcceptance::Queued));
        }
        let groups = self
            .subscriptions
            .as_ref()
            .filter(|_| self.subscription_received.elapsed().as_secs() < 30)
            .map(|s| targets(s, &event));
        if groups.as_ref().is_some_and(Vec::is_empty) {
            return Ok(accepted(&event, LocalAcceptance::NoSubscribers));
        }
        let bytes = body.len() as u64
            + 512
            + groups
                .as_ref()
                .map_or(0, |g| g.iter().map(|id| id.len() as u64 + 128).sum());
        let tx = self.db.transaction()?;
        // Capacity covers all identities in this database, including a previous login.
        let used: u64 =
            tx.query_row("SELECT COALESCE(SUM(bytes),0) FROM event_sizes", [], |r| {
                r.get(0)
            })?;
        if used.saturating_add(bytes) > policy.capacity_bytes {
            bail!("event storage capacity reached; retry after space becomes available");
        }
        tx.execute("INSERT INTO events(scope,app_id,event_id,body,received_at,routed,next_attempt) VALUES(?1,?2,?3,?4,?5,?6,?5)",
            params![self.scope,event.app_id,event.event_id,body,now,groups.is_some()])?;
        if let Some(groups) = groups {
            insert_groups(&tx, &self.scope, &event, &groups)?;
        }
        tx.commit()?;
        Ok(accepted(&event, LocalAcceptance::Queued))
    }

    pub(super) fn due(
        &mut self,
        policy: &StoragePolicy,
        now: i64,
        limit: usize,
    ) -> Result<Vec<LocalAppEventEmitted>> {
        self.cleanup(policy, now)?;
        // No fresh subscription authority: retain locally and wait, including across restart.
        let Some(snapshot) = self
            .subscriptions
            .as_ref()
            .filter(|_| self.subscription_received.elapsed().as_secs() < 30)
        else {
            return Ok(vec![]);
        };
        let tx = self.db.transaction()?;
        let rows = {
            let mut stmt = tx.prepare("SELECT body,routed,attempts FROM events WHERE scope=?1 AND next_attempt<=?2 ORDER BY next_attempt LIMIT ?3")?;
            let rows = stmt
                .query_map(params![self.scope, now, limit as u64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, u32>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut result = Vec::new();
        for (body, routed, attempts) in rows {
            let mut event: LocalAppEventEmitted = serde_json::from_str(&body)?;
            if !routed {
                let groups = targets(snapshot, &event);
                if groups.is_empty() {
                    finish(
                        &tx,
                        &self.scope,
                        &event.app_id,
                        &event.event_id,
                        "no_subscribers",
                        now,
                    )?;
                    continue;
                }
                insert_groups(&tx, &self.scope, &event, &groups)?;
            }
            event.target_consumers = pending(&tx, &self.scope, &event.app_id, &event.event_id)?;
            let delay = 2_i64.pow(attempts.min(6)).min(60);
            tx.execute("UPDATE events SET routed=1,next_attempt=?4,attempts=MIN(attempts+1,64) WHERE scope=?1 AND app_id=?2 AND event_id=?3",
                params![self.scope,event.app_id,event.event_id,now+15+delay])?;
            result.push(event);
        }
        tx.commit()?;
        Ok(result)
    }

    pub(super) fn acknowledge(&mut self, ack: EventAck, now: i64) -> Result<()> {
        let tx = self.db.transaction()?;
        for group in ack.receipts {
            // A business rejection does not prove processing; retain until retry or expiry.
            if matches!(
                group.receipt,
                DeliveryReceipt::Processed { .. } | DeliveryReceipt::Ignored
            ) {
                tx.execute("DELETE FROM deliveries WHERE scope=?1 AND app_id=?2 AND event_id=?3 AND consumer_id=?4",
                    params![self.scope,ack.app_id,ack.event_id,group.consumer_id])?;
            }
        }
        let routed = tx
            .query_row(
                "SELECT routed FROM events WHERE scope=?1 AND app_id=?2 AND event_id=?3",
                params![self.scope, ack.app_id, ack.event_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?;
        if routed == Some(true) && pending(&tx, &self.scope, &ack.app_id, &ack.event_id)?.is_empty()
        {
            finish(
                &tx,
                &self.scope,
                &ack.app_id,
                &ack.event_id,
                "confirmed",
                now,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn cleanup(&mut self, policy: &StoragePolicy, now: i64) -> Result<()> {
        policy.validate()?;
        let tx = self.db.transaction()?;
        let expired = if let Some(cutoff) = policy.cutoff(now) {
            tx.execute("INSERT INTO diagnostics(app_id,event_id,outcome,finished_at) SELECT app_id,event_id,'expired',?1 FROM events WHERE received_at<=?2",params![now,cutoff])?;
            tx.execute("DELETE FROM deliveries WHERE EXISTS (SELECT 1 FROM events e WHERE e.scope=deliveries.scope AND e.app_id=deliveries.app_id AND e.event_id=deliveries.event_id AND e.received_at<=?1)",[cutoff])?;
            tx.execute("DELETE FROM events WHERE received_at<=?1", [cutoff])?
        } else {
            0
        };
        tx.execute("DELETE FROM diagnostics WHERE finished_at<=?1 OR id NOT IN (SELECT id FROM diagnostics ORDER BY id DESC LIMIT 10000)",
            [now-i64::from(policy.diagnostic_retention_days)*86400])?;
        let maintenance = now.saturating_sub(self.last_maintenance) >= 60;
        if expired > 0 || maintenance {
            tx.execute("INSERT INTO cleanup(id,at,expired) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET at=excluded.at,expired=excluded.expired",params![now,expired])?;
        }
        tx.commit()?;
        if maintenance {
            self.db.execute_batch(
                "PRAGMA incremental_vacuum(256); PRAGMA wal_checkpoint(TRUNCATE);",
            )?;
            self.last_maintenance = now;
        }
        Ok(())
    }

    pub(super) fn statistics(&self, policy: &StoragePolicy, now: i64) -> Result<StorageStats> {
        policy.validate()?;
        let (pending_events, logical_bytes, oldest_received_at) = self.db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(bytes),0),MIN(received_at) FROM event_sizes",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let (expiring_events, expiring_bytes) = self.db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(bytes),0) FROM event_sizes WHERE received_at<=?1",
            [policy.cutoff(now)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let last = self
            .db
            .query_row("SELECT at,expired FROM cleanup WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let physical_bytes = [
            self.path.clone(),
            PathBuf::from(format!("{}-wal", self.path.display())),
            PathBuf::from(format!("{}-shm", self.path.display())),
        ]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
        Ok(StorageStats {
            pending_events,
            logical_bytes,
            physical_bytes,
            oldest_received_at,
            expiring_events,
            expiring_bytes,
            last_cleanup_at: last.map(|v| v.0),
            last_cleanup_expired: last.map_or(0, |v| v.1),
        })
    }
}
fn accepted(event: &LocalAppEventEmitted, status: LocalAcceptance) -> LocalEventAccepted {
    LocalEventAccepted {
        contract_version: Default::default(),
        event_id: event.event_id.clone(),
        app_id: event.app_id.clone(),
        status,
    }
}
fn insert_groups(
    tx: &Transaction<'_>,
    scope: &str,
    event: &LocalAppEventEmitted,
    groups: &[String],
) -> Result<()> {
    for group in groups {
        tx.execute(
            "INSERT INTO deliveries(scope,app_id,event_id,consumer_id) VALUES(?1,?2,?3,?4)",
            params![scope, event.app_id, event.event_id, group],
        )?;
    }
    Ok(())
}
fn pending(tx: &Transaction<'_>, scope: &str, app: &str, event: &str) -> Result<Vec<String>> {
    let mut stmt=tx.prepare("SELECT consumer_id FROM deliveries WHERE scope=?1 AND app_id=?2 AND event_id=?3 ORDER BY consumer_id")?;
    let rows = stmt
        .query_map(params![scope, app, event], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
fn finish(
    tx: &Transaction<'_>,
    scope: &str,
    app: &str,
    event: &str,
    outcome: &str,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO diagnostics(app_id,event_id,outcome,finished_at) VALUES(?1,?2,?3,?4)",
        params![app, event, outcome, now],
    )?;
    tx.execute(
        "DELETE FROM deliveries WHERE scope=?1 AND app_id=?2 AND event_id=?3",
        params![scope, app, event],
    )?;
    tx.execute(
        "DELETE FROM events WHERE scope=?1 AND app_id=?2 AND event_id=?3",
        params![scope, app, event],
    )?;
    Ok(())
}
