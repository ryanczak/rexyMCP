use crate::config::CommandConfig;

/// The embedded executor-contract template, baked in at compile time.
/// Lives at executor/templates/executor_contract.md; see M6 phase-02.
const TEMPLATE: &str = include_str!("../../templates/executor_contract.md");

/// Marker used when a CommandConfig field is `None`. The contract template
/// references all four commands; substituting an empty string would produce
/// confusing output like `run `` (the configured format-check command)`.
/// This sentinel is unambiguous and tells the model the situation when it
/// reads the assembled prompt.
pub const UNCONFIGURED: &str = "(not configured)";

/// Substitute the four `{…_COMMAND}` placeholders in the embedded contract
/// template with values from `commands`. Unset commands render as the
/// `UNCONFIGURED` sentinel.
///
/// Returns the substituted contract body. Pure; no I/O.
pub fn assemble_executor_contract(commands: &CommandConfig) -> String {
    let mut s = TEMPLATE.to_string();
    s = s.replace(
        "{FORMAT_COMMAND}",
        commands.format.as_deref().unwrap_or(UNCONFIGURED),
    );
    s = s.replace(
        "{BUILD_COMMAND}",
        commands.build.as_deref().unwrap_or(UNCONFIGURED),
    );
    s = s.replace(
        "{LINT_COMMAND}",
        commands.lint.as_deref().unwrap_or(UNCONFIGURED),
    );
    s = s.replace(
        "{TEST_COMMAND}",
        commands.test.as_deref().unwrap_or(UNCONFIGURED),
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_all_four_commands_when_set() {
        let commands = CommandConfig {
            format: Some("cargo fmt".to_string()),
            build: Some("cargo build".to_string()),
            lint: Some("cargo clippy".to_string()),
            test: Some("cargo test".to_string()),
        };
        let output = assemble_executor_contract(&commands);

        assert!(output.contains("cargo fmt"));
        assert!(output.contains("cargo build"));
        assert!(output.contains("cargo clippy"));
        assert!(output.contains("cargo test"));

        assert!(!output.contains("{FORMAT_COMMAND}"));
        assert!(!output.contains("{BUILD_COMMAND}"));
        assert!(!output.contains("{LINT_COMMAND}"));
        assert!(!output.contains("{TEST_COMMAND}"));
    }

    #[test]
    fn unset_command_renders_as_unconfigured_sentinel() {
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: None,
        };
        let output = assemble_executor_contract(&commands);

        let unconfigured_count = output.matches(UNCONFIGURED).count();
        assert_eq!(
            unconfigured_count, 3,
            "expected 3 UNCONFIGURED sentinels for 3 None fields, got {}",
            unconfigured_count
        );
        assert!(output.contains("cargo build"));
        assert!(!output.contains("{FORMAT_COMMAND}"));
        assert!(!output.contains("{LINT_COMMAND}"));
        assert!(!output.contains("{TEST_COMMAND}"));
    }

    #[test]
    fn output_starts_with_contract_preamble() {
        let commands = CommandConfig::default();
        let output = assemble_executor_contract(&commands);

        assert!(
            output.starts_with("# Executor Contract"),
            "expected output to start with '# Executor Contract', got: {}",
            &output[..output
                .char_indices()
                .take(100)
                .map(|(i, _)| i)
                .last()
                .map(|i| i + 1)
                .unwrap_or(0)
                .min(output.len())]
        );
    }

    #[test]
    fn placeholder_set_is_exactly_the_four_authorized() {
        // Verify the four authorized placeholders exist in the template
        assert!(TEMPLATE.contains("{FORMAT_COMMAND}"));
        assert!(TEMPLATE.contains("{BUILD_COMMAND}"));
        assert!(TEMPLATE.contains("{LINT_COMMAND}"));
        assert!(TEMPLATE.contains("{TEST_COMMAND}"));

        // Verify no other {...} placeholders exist by removing the four authorized
        // and checking nothing curly-brace-wrapped remains
        let remaining = TEMPLATE
            .replace("{FORMAT_COMMAND}", "")
            .replace("{BUILD_COMMAND}", "")
            .replace("{LINT_COMMAND}", "")
            .replace("{TEST_COMMAND}", "");

        let has_other_placeholder = remaining.split('{').skip(1).any(|after| {
            after.contains('}')
                && after
                    .split('}')
                    .next()
                    .map(|inner| inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(false)
        });

        assert!(
            !has_other_placeholder,
            "template contains placeholders other than the four authorized ones"
        );
    }
}
