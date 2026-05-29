// Security: path-scope confinement, bash command classification, and secret
// redaction.

pub mod bash_classify;
pub mod redact;
pub mod scope;

pub use bash_classify::{Severity, classify};
pub use redact::{Redactor, SecretKind};
pub use scope::{Scope, ScopeError};
