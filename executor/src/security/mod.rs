// Security: path-scope confinement and bash command classification.

pub mod bash_classify;
pub mod scope;

pub use bash_classify::{Severity, classify};
pub use scope::{Scope, ScopeError};
