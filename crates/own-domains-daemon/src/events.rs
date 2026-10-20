use own_domains_protocol::{Event, EventEnvelope, PROTOCOL_VERSION, StateSnapshot};
use std::{
    collections::VecDeque,
    sync::{Mutex, mpsc},
    time::Duration,
};

struct BrokerState {
    sequence: u64,
    history: VecDeque<EventEnvelope>,
    subscribers: Vec<mpsc::Sender<EventEnvelope>>,
}

pub struct EventBroker {
    capacity: usize,
    state: Mutex<BrokerState>,
}

pub struct Subscription {
    receiver: mpsc::Receiver<EventEnvelope>,
}

impl Subscription {
    pub fn recv(&self) -> Result<EventEnvelope, mpsc::RecvError> {
        self.receiver.recv()
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<EventEnvelope, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
}

impl EventBroker {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(BrokerState {
                sequence: 0,
                history: VecDeque::new(),
                subscribers: vec![],
            }),
        }
    }

    pub fn publish(&self, event: Event) -> EventEnvelope {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.sequence += 1;
        let envelope = EventEnvelope {
            protocol_version: PROTOCOL_VERSION,
            sequence: state.sequence,
            event,
        };
        state.history.push_back(envelope.clone());
        while state.history.len() > self.capacity {
            state.history.pop_front();
        }
        state
            .subscribers
            .retain(|subscriber| subscriber.send(envelope.clone()).is_ok());
        envelope
    }

    pub fn subscribe(
        &self,
        after_sequence: Option<u64>,
        snapshot: impl FnOnce() -> StateSnapshot,
    ) -> Subscription {
        let (sender, receiver) = mpsc::channel();
        let snapshot = snapshot();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(after) = after_sequence {
            let oldest = state
                .history
                .front()
                .map(|event| event.sequence)
                .unwrap_or(state.sequence + 1);
            if after + 1 < oldest {
                let envelope = EventEnvelope {
                    protocol_version: PROTOCOL_VERSION,
                    sequence: state.sequence,
                    event: Event::ResyncRequired { snapshot },
                };
                let _ = sender.send(envelope);
            } else {
                for event in state.history.iter().filter(|event| event.sequence > after) {
                    let _ = sender.send(event.clone());
                }
            }
        } else {
            let envelope = EventEnvelope {
                protocol_version: PROTOCOL_VERSION,
                sequence: state.sequence,
                event: Event::Snapshot(snapshot),
            };
            let _ = sender.send(envelope);
        }
        state.subscribers.push(sender);
        Subscription { receiver }
    }

    pub fn latest_sequence(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .sequence
    }
}

impl Default for EventBroker {
    fn default() -> Self {
        Self::with_capacity(256)
    }
}
