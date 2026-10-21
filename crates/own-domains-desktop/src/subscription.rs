use crate::bridge::{BridgeError, DesktopBridge};
use own_domains_client::EventStreamCancellation;
use own_domains_protocol::EventEnvelope;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

type EventSink = Arc<dyn Fn(u64, Result<EventEnvelope, BridgeError>) + Send + Sync>;
struct Worker {
    id: u64,
    cancel: EventStreamCancellation,
    cancelled: Arc<AtomicBool>,
    join: JoinHandle<()>,
}
#[derive(Default)]
pub struct Subscriptions {
    next_id: u64,
    worker: Option<Worker>,
}
impl Subscriptions {
    pub fn replace(
        &mut self,
        bridge: &DesktopBridge,
        after: Option<u64>,
        sink: EventSink,
    ) -> Result<u64, BridgeError> {
        self.clear()?;
        let mut stream = bridge.client.subscribe(after)?;
        let cancel = stream.cancellation_handle()?;
        self.next_id += 1;
        let id = self.next_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = cancelled.clone();
        let join = thread::spawn(move || {
            loop {
                let event = stream.recv().map_err(BridgeError::from);
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let failed = event.is_err();
                sink(id, event);
                if failed {
                    break;
                }
            }
        });
        self.worker = Some(Worker {
            id,
            cancel,
            cancelled,
            join,
        });
        Ok(id)
    }
    pub fn remove(&mut self, id: u64) -> Result<(), BridgeError> {
        if self.worker.as_ref().is_some_and(|worker| worker.id == id) {
            self.clear()?;
        }
        Ok(())
    }
    pub fn clear(&mut self) -> Result<(), BridgeError> {
        if let Some(worker) = self.worker.take() {
            worker.cancelled.store(true, Ordering::Release);
            let result = worker.cancel.cancel();
            let _ = worker.join.join();
            result?;
        }
        Ok(())
    }
}
impl Drop for Subscriptions {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}
