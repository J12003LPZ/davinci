use std::fs;
use std::path::Path;

pub const CONTEXT_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", "CONTEXT.md"];

pub fn load_context_files(cwd: &Path, disabled: bool) -> Vec<(String, String)> {
    if disabled {
        return Vec::new();
    }
    let mut files = Vec::new();
    for name in CONTEXT_FILES {
        let path = cwd.join(name);
        if let Ok(content) = fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                files.push((name.to_string(), content));
            }
        }
    }
    files
}

pub fn render_system_prompt(
    base: Option<&str>,
    append: &[String],
    context: &[(String, String)],
) -> String {
    let mut prompt = base
        .unwrap_or("You are a coding assistant with read, bash, edit, and write tools.")
        .to_string();
    for extra in append {
        if let Ok(from_file) = fs::read_to_string(extra) {
            prompt.push('\n');
            prompt.push_str(&from_file);
        } else {
            prompt.push('\n');
            prompt.push_str(extra);
        }
    }
    for (name, content) in context {
        prompt.push_str(&format!("\n\n# {name}\n{content}"));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_agents_md() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "use rustfmt").unwrap();
        let files = load_context_files(dir.path(), false);
        assert_eq!(files[0].0, "AGENTS.md");
        assert!(load_context_files(dir.path(), true).is_empty());
    }
}
