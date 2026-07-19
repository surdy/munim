//! munim-core — pure domain logic shared by the desktop shell.
//!
//! No Tauri / UI dependencies, so it builds and unit-tests in isolation
//! (`cargo test -p munim-core`). See BUILD_SPEC.md §4 (collector), §4.5 (pricing),
//! §5.2b (settings).

pub mod cache;
pub mod collector;
pub mod pricing;
pub mod settings;

pub use cache::collect_and_persist;
pub use collector::{
    collect, Caches, CollectOutput, CollectResult, Provider, ScanStats, SessionRecord, Summary,
};
pub use pricing::Pricing;
pub use settings::Settings;
