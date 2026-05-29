//! The verifier seam. The loop depends on this trait, not on
//! `governor::verifier`'s free functions, so tests can inject a deterministic
//! mock instead of spawning a real compiler (`cargo`/`tsc`/`ruff`).

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::governor::verifier::{self, Baseline, VerifierResult};

/// Post-edit verification + session-start baseline capture, behind a trait for
/// test injection.
#[async_trait]
pub trait FileVerifier: Send + Sync {
    async fn verify(&self, path: &Path) -> VerifierResult;
    async fn capture_baseline(&self, paths: &[PathBuf]) -> Baseline;
}

/// The production verifier — delegates to `governor::verifier`, which shells out
/// to the per-language checker.
pub struct RealVerifier;

#[async_trait]
impl FileVerifier for RealVerifier {
    async fn verify(&self, path: &Path) -> VerifierResult {
        verifier::verify(path).await
    }

    async fn capture_baseline(&self, paths: &[PathBuf]) -> Baseline {
        verifier::capture_baseline(paths).await
    }
}
