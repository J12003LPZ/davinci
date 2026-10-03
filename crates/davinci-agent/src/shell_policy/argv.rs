//! Structured process arguments are inspected here, never executed as a string.
use super::{ShellCommandDecision, ShellPolicyProfile};

pub fn argv_subject(executable: &str, argv: &[String]) -> String {
    let file = executable.rsplit(['/', '\\']).next().unwrap_or(executable);
    let lower = file.to_ascii_lowercase();
    let program = [".exe", ".cmd", ".bat", ".com"]
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix))
        .unwrap_or(&lower);
    std::iter::once(program)
        .chain(argv.iter().map(String::as_str))
        .map(|part| {
            if simple(part) {
                part.to_owned()
            } else {
                serde_json::to_string(part).expect("string serialization")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn simple(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:@%+=,-\\".contains(&b))
}

pub fn evaluate_argv(
    profile: ShellPolicyProfile,
    executable: &str,
    argv: &[String],
) -> ShellCommandDecision {
    if profile == ShellPolicyProfile::Permissive {
        return ShellCommandDecision::Allowed;
    }
    // Restricted role profiles cannot prove arbitrary interpreter programs or
    // opaque arguments obey their effect ceiling. Ordinary host approval can.
    let subject = argv_subject(executable, argv);
    let report = super::analyze_command(&subject);
    if argv.iter().any(|part| !simple(part)) || report.has_nested_shell || report.has_substitution {
        return ShellCommandDecision::Denied {
            reason: "structured command cannot be proven within this role's process policy".into(),
        };
    }
    super::evaluate(profile, &subject)
}
