#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Client, ClientError, EventStream, EventStreamCancellation};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Client, ClientError, EventStream, EventStreamCancellation};

mod lifecycle;
pub use lifecycle::{LifecycleError, connect_or_launch, stop_and_wait};

pub use own_domains_protocol::Endpoint;
