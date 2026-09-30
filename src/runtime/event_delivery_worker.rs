type EventAttempts = FuturesUnordered<BoxFuture<'static, std::result::Result<EventAck, ()>>>;

/// Attempts own bounded waits. Durable rows survive timeout, disconnect and restart.
async fn run_event_delivery(
    queue: EventQueue,
    sender: mpsc::Sender<LocalAppEventSubmission>,
    config_path: PathBuf,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut attempts = EventAttempts::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            Some(result) = attempts.next(), if !attempts.is_empty() => {
                persist_event_receipt(&queue, result).await;
            }
            _ = tick.tick() => {
                if let Err(error) = schedule_event_attempts(&queue, &sender, &config_path, &mut attempts).await {
                    tracing::error!(%error,"event delivery scheduling failed");
                }
            }
        }
    }
}

async fn schedule_event_attempts(
    queue: &EventQueue,
    sender: &mpsc::Sender<LocalAppEventSubmission>,
    config_path: &Path,
    attempts: &mut EventAttempts,
) -> Result<()> {
    let policy = crate::event_delivery::read_policy(config_path).await?;
    for event in queue.due(policy, 16_usize.saturating_sub(attempts.len())).await? {
        attempts.push(attempt_event(sender.clone(), event).boxed());
    }
    Ok(())
}

async fn attempt_event(
    sender: mpsc::Sender<LocalAppEventSubmission>,
    event: LocalAppEventEmitted,
) -> std::result::Result<EventAck, ()> {
    let (tx, rx) = oneshot::channel();
    timeout(Duration::from_secs(15), async move {
        sender.send(LocalAppEventSubmission {event, response: tx}).await.map_err(|_| ())?;
        rx.await.map_err(|_| ())?.map_err(|_| ())
    }).await.map_err(|_| ())?
}

async fn persist_event_receipt(queue: &EventQueue, result: std::result::Result<EventAck, ()>) {
    if let Ok(ack) = result {
        if let Err(error) = queue.acknowledge(ack).await {
            tracing::error!(%error,"cannot persist event receipt");
        }
    }
}
