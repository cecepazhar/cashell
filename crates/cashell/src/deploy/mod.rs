//! Standalone Git deployment engine, build queue, and webhook receiver (Tahap 4).

pub mod executor;
pub mod git;
pub mod queue;
pub mod webhook;
pub mod zero_downtime;

pub use executor::{deploy_app, detect_compose, DeployOutcome};
pub use git::{sync_app_repository, DeployGitResult};
pub use queue::{BuildJob, BuildQueue};
pub use webhook::{extract_commit_from_payload, verify_webhook_signature};
pub use zero_downtime::ZeroDowntimeDeployer;
