// Detection: lexically sniff a model response for each format's tell-tale marker
// and return the formats worth attempting, in priority order. The pipeline walks
// the result in order and hands each format to its extractor. Multiple formats
// may fire on one response; an empty result means no tool call was attempted.

use regex::Regex;
use std::sync::OnceLock;

use super::Format;

pub fn detect(response: &str) -> Vec<Format> {
    let mut out = Vec::with_capacity(6);

    if response.contains("<tool_call>") {
        out.push(Format::Hermes);
    }
    if response.contains("<function=") {
        out.push(Format::XmlVariant);
    }
    if response.contains("```json") {
        out.push(Format::FencedJson);
    }
    if response.contains("```yaml") || yaml_block_re().is_match(response) {
        out.push(Format::Yaml);
    }
    if has_balanced_braces(response) {
        out.push(Format::LooseJson);
    }
    // PlainText only fires if nothing else did — the regex is too noisy to
    // coexist with structured formats.
    if out.is_empty() && plain_text_re().is_match(response) {
        out.push(Format::PlainText);
    }

    out
}

fn yaml_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)(^|\n)name:\s+\S+\s*\n\s*arguments:").unwrap())
}

fn plain_text_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\w+\(\w+\s*=").unwrap())
}

fn has_balanced_braces(s: &str) -> bool {
    let opens = s.matches('{').count();
    let closes = s.matches('}').count();
    opens > 0 && opens == closes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hermes_when_tool_call_tag_present() {
        let out = detect("prefix <tool_call>{}</tool_call> suffix");
        assert_eq!(out.first(), Some(&Format::Hermes));
    }

    #[test]
    fn detects_xml_variant_when_function_tag_present() {
        let out = detect("here: <function=read_file>{}</function>");
        assert!(out.contains(&Format::XmlVariant));
    }

    #[test]
    fn detects_fenced_json_when_json_fence_present() {
        let out = detect("```json\n{}\n```");
        assert!(out.contains(&Format::FencedJson));
    }

    #[test]
    fn detects_yaml_when_yaml_block_pattern_matches() {
        let out = detect("name: read_file\narguments:\n  path: x\n");
        assert!(out.contains(&Format::Yaml));
    }

    #[test]
    fn detects_loose_json_when_balanced_braces_in_prose() {
        let out = detect("the json {\"name\": \"x\"} is here");
        assert!(out.contains(&Format::LooseJson));
    }

    #[test]
    fn detects_plain_text_when_only_call_pattern_matches() {
        let out = detect("call read_file(path=foo)");
        assert!(out.contains(&Format::PlainText));
    }

    #[test]
    fn returns_formats_in_fixed_priority_order() {
        let input = "<tool_call>{}</tool_call> and ```json\n{}\n``` and {\"x\": 1}";
        let out = detect(input);
        assert_eq!(
            out,
            vec![Format::Hermes, Format::FencedJson, Format::LooseJson]
        );
    }

    #[test]
    fn returns_empty_for_plain_prose() {
        let out = detect("just a regular response, no tool calls.");
        assert!(out.is_empty());
    }

    // Negative cases (required by the phase spec): pin what must NOT fire.

    #[test]
    fn plain_text_does_not_fire_when_structured_format_present() {
        // A hermes marker is present, so the noisy plain-text regex must not fire
        // even though "call foo(path=x)" would match it in isolation.
        let out = detect("<tool_call>{\"name\":\"x\"}</tool_call> call foo(path=x)");
        assert!(out.contains(&Format::Hermes));
        assert!(!out.contains(&Format::PlainText));
    }

    #[test]
    fn loose_json_does_not_fire_on_unbalanced_braces() {
        let out = detect("a { b c");
        assert!(!out.contains(&Format::LooseJson));
    }
}
