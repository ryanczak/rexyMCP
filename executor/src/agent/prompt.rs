use crate::config::CommandConfig;

use super::contract;

/// Assemble the executor system prompt from its three inputs, in the order the
/// architecture pins: the embedded executor contract, the project `STANDARDS.md`,
/// then the architect's (pre-injected) phase doc. The local model reads none of
/// these as files — they are composed in-process from strings the caller holds.
pub fn assemble_system_prompt(
    commands: &CommandConfig,
    standards: &str,
    phase_doc: &str,
) -> String {
    let contract_body = contract::assemble_executor_contract(commands);
    let mut out = String::new();
    out.push_str("# Executor contract\n\n");
    out.push_str(contract_body.trim_end());
    out.push_str("\n\n# Engineering standards\n\n");
    out.push_str(standards.trim_end());
    out.push_str("\n\n# Phase\n\n");
    out.push_str(phase_doc.trim_end());
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_system_prompt_in_contract_standards_phase_order() {
        let commands = CommandConfig {
            format: Some("cargo fmt".to_string()),
            build: Some("cargo build".to_string()),
            lint: Some("cargo clippy".to_string()),
            test: Some("cargo test".to_string()),
        };
        let prompt = assemble_system_prompt(&commands, "STANDARDS_BODY", "PHASE_BODY");

        assert!(prompt.contains("cargo fmt"));
        assert!(prompt.contains("STANDARDS_BODY"));
        assert!(prompt.contains("PHASE_BODY"));

        let contract = prompt.find("cargo fmt").expect("contract present");
        let standards = prompt.find("STANDARDS_BODY").expect("standards present");
        let phase = prompt.find("PHASE_BODY").expect("phase present");

        assert!(
            contract < standards && standards < phase,
            "expected contract < standards < phase, got {contract}/{standards}/{phase}"
        );
    }

    #[test]
    fn system_prompt_includes_substituted_contract() {
        let commands = CommandConfig {
            format: Some("npm fmt".to_string()),
            build: Some("npm run build".to_string()),
            lint: Some("npm run lint".to_string()),
            test: Some("npm test".to_string()),
        };
        let prompt = assemble_system_prompt(&commands, "MY_STANDARDS", "MY_PHASE");

        assert!(prompt.contains("Executor Contract"));
        assert!(prompt.contains("npm fmt"));
        assert!(prompt.contains("npm run build"));
        assert!(prompt.contains("npm run lint"));
        assert!(prompt.contains("npm test"));
        assert!(prompt.contains("MY_STANDARDS"));
        assert!(prompt.contains("MY_PHASE"));
    }

    #[test]
    fn system_prompt_order_is_contract_then_standards_then_phase_doc() {
        let commands = CommandConfig::default();
        let prompt =
            assemble_system_prompt(&commands, "UNIQUE_STANDARDS_MARKER", "UNIQUE_PHASE_MARKER");

        let contract_pos = prompt
            .find("Executor Contract")
            .expect("contract section present");
        let standards_pos = prompt
            .find("UNIQUE_STANDARDS_MARKER")
            .expect("standards section present");
        let phase_pos = prompt
            .find("UNIQUE_PHASE_MARKER")
            .expect("phase section present");

        assert!(
            contract_pos < standards_pos && standards_pos < phase_pos,
            "expected contract < standards < phase, got {contract_pos}/{standards_pos}/{phase_pos}"
        );
    }
}
