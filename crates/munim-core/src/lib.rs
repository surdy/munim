//! munim-core — pure domain logic shared by the desktop shell.
//!
//! No Tauri / UI dependencies, so it builds and unit-tests in isolation
//! (`cargo test -p munim-core`). See BUILD_SPEC.md §4 (collector), §4.5 (pricing),
//! §5.2b (settings).

pub mod collector;
pub mod pricing;
pub mod settings;

pub use collector::{collect, CollectOutput, Provider, SessionRecord, Summary};
pub use pricing::Pricing;
pub use settings::Settings;
