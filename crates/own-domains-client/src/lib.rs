#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::{Client, ClientError, EventStream, EventStreamCancellation};

#[cfg(unix)]
mod lifecycle;
#[cfg(unix)]
pub use lifecycle::{LifecycleError, connect_or_launch, stop_and_wait};
