pub mod backend;
pub mod error;
pub mod queue;
pub mod resolver;

pub use backend::{
    AudioBackend, AudioProperties, BackendEvent, MpvIpcBackend, normalize_af,
    overdrive_limiter_af, safe_unlink_socket,
};
pub use error::{AudioError, AudioResult};
pub use queue::PlayQueue;
pub use resolver::{
    HybridStreamResolver, RawStream, ResolvedStream, StreamBackend, YtDlpBackend,
    parse_stream_expiry,
};
#[cfg(feature = "fast-resolver")]
pub use resolver::RustyPipeBackend;
