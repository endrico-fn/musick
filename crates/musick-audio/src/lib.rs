pub mod backend;
pub mod cava;
pub mod engine;
pub mod error;
pub mod mpris;
pub mod queue;
pub mod resolver;

pub use backend::{
    AudioBackend, AudioProperties, BackendEvent, MpvIpcBackend, normalize_af, overdrive_limiter_af,
    safe_unlink_socket,
};
pub use cava::{
    CavaControl, CavaFrame, CavaRunner, CavaRunnerHandle, build_cava_config, detect_cava_methods,
    feather_bars, parse_cava_frame, resample_bars, resolve_cava_source, smoothing_params,
};
pub use engine::{PlayerEngine, PlayerEngineDeps, PlayerEngineHandle};
pub use error::{AudioError, AudioResult};
pub use mpris::MprisService;
pub use queue::PlayQueue;
#[cfg(feature = "fast-resolver")]
pub use resolver::RustyPipeBackend;
pub use resolver::{
    HybridStreamResolver, RawStream, ResolvedStream, StreamBackend, YtDlpBackend,
    parse_stream_expiry,
};
