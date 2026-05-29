/// Assemble the executor system prompt from its three inputs, in the order the
/// architecture pins: the embedded executor contract, the project `STANDARDS.md`,
/// then the architect's (pre-injected) phase doc. The local model reads none of
/// these as files — they are composed in-process from strings the caller holds.
pub fn assemble_system_prompt(executor_contract: &str, standards: &str, phase_doc: &str) -> String {
    let mut out = String::new();
    out.push_str("# Executor contract\n\n");
    out.push_str(executor_contract.trim_end());
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
        let prompt = assemble_system_prompt("CONTRACT_BODY", "STANDARDS_BODY", "PHASE_BODY");

        let contract = prompt.find("CONTRACT_BODY").expect("contract present");
        let standards = prompt.find("STANDARDS_BODY").expect("standards present");
        let phase = prompt.find("PHASE_BODY").expect("phase present");

        assert!(
            contract < standards && standards < phase,
            "expected contract < standards < phase, got {contract}/{standards}/{phase}"
        );
    }
}
