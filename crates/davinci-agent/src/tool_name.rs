//! Tool-name ingress shared by direct, batch, and recovered dispatch.

pub(crate) fn canonical_tool_name(name: &str, registered: impl Fn(&str) -> bool) -> &str {
    if registered(name) {
        return name;
    }
    match name.strip_prefix("functions.") {
        Some(base) if !base.is_empty() && !base.starts_with("functions.") && registered(base) => {
            base
        }
        _ => name,
    }
}

impl crate::Agent {
    pub(crate) fn canonical_tool_name<'a>(&self, name: &'a str) -> &'a str {
        canonical_tool_name(name, |candidate| {
            self.tool_registry
                .iter()
                .chain(&self.tools)
                .any(|n| n == candidate)
                || self
                    .runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.capability_registry.get(candidate).is_some())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_alias_table() {
        let registered = ["find", "apply_patch", "write", "batch", "mcp.server.find"];
        for (name, expected) in [
            ("functions.find", "find"),
            ("functions.apply_patch", "apply_patch"),
            ("find", "find"),
            ("functions.functions.find", "functions.functions.find"),
            ("other.find", "other.find"),
            ("functions.", "functions."),
            ("functions.missing", "functions.missing"),
            ("mcp.server.find", "mcp.server.find"),
        ] {
            assert_eq!(
                canonical_tool_name(name, |n| registered.contains(&n)),
                expected
            );
        }
    }

    #[test]
    fn tool_name_exact_registration_wins_even_when_disabled() {
        let mut agent = crate::Agent::new("names");
        agent.tool_registry.push("functions.find".into());
        agent.tools.retain(|name| name != "functions.find");
        assert_eq!(
            agent.canonical_tool_name("functions.find"),
            "functions.find"
        );
        assert_eq!(
            agent.canonical_tool_name("functions.functions.find"),
            "functions.functions.find"
        );
        assert_eq!(agent.canonical_tool_name("functions.write"), "write");
    }
}
