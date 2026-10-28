//! Authenticated control-host composition, dispatch and process supervision.
//! Durable admission and execution policy remain in the backend during extraction.
pub mod app;
pub mod host;
pub mod orchestration;
pub mod supervisor;
