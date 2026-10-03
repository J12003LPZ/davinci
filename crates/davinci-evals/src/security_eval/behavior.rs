//! Compiled representative cases. Worker-visible bytes are these modules only.

pub mod auth_idor_s;
pub mod auth_idor_v;
pub mod dependency_s;
pub mod dependency_v;
pub mod filesystem_s;
pub mod filesystem_v;
pub mod prompt_s;
pub mod prompt_v;
pub mod subprocess_s;
pub mod subprocess_v;

#[cfg(test)]
mod tests {
    use super::{
        auth_idor_s, auth_idor_v, dependency_s, dependency_v, filesystem_s, filesystem_v, prompt_s,
        prompt_v, subprocess_s, subprocess_v,
    };

    #[test]
    fn security_eval_behavior_owner_check_is_the_access_boundary() {
        let documents = [auth_idor_v::Document {
            id: 7,
            owner_id: 2,
            body: "secret",
        }];
        let stranger = auth_idor_v::Actor { id: 1 };
        assert_eq!(
            auth_idor_v::get_document(&stranger, 7, &documents),
            Some("secret")
        );
        let documents = [auth_idor_s::Document {
            id: 7,
            owner_id: 2,
            body: "secret",
        }];
        let stranger = auth_idor_s::Actor { id: 1 };
        let owner = auth_idor_s::Actor { id: 2 };
        assert_eq!(auth_idor_s::get_document(&stranger, 7, &documents), None);
        assert_eq!(
            auth_idor_s::get_document(&owner, 7, &documents),
            Some("secret")
        );
        let _ = owner;
    }

    #[test]
    fn security_eval_behavior_path_is_checked_at_final_use() {
        let encoded = "%2e%2e/secret";
        assert!(filesystem_v::resolve_user_file("/data", encoded)
            .unwrap()
            .contains(".."));
        assert_eq!(
            filesystem_s::resolve_user_file("/data", encoded),
            Err("rejected")
        );
        assert_eq!(
            filesystem_s::resolve_user_file("/data", "notes.txt").unwrap(),
            "/data/notes.txt"
        );
    }

    #[test]
    fn security_eval_behavior_subprocess_is_argv_only() {
        let hostile = "ok; cat /etc/passwd";
        let vulnerable = subprocess_v::build_echo(hostile);
        assert_eq!(vulnerable[1], "-c");
        assert!(vulnerable[2].contains(hostile));
        let secure = subprocess_s::build_echo(hostile);
        assert_eq!(secure, ["echo", hostile]);
        assert!(!secure.iter().any(|part| part == "-c"));
    }

    #[test]
    fn security_eval_behavior_affected_dependency_feature_is_absent_when_secure() {
        assert!(dependency_v::render_template("{{x}}").starts_with("yaml:"));
        assert!(dependency_s::render_template("{{x}}").starts_with("json:"));
    }

    #[test]
    fn security_eval_behavior_tool_request_requires_approval() {
        assert!(prompt_v::allow_tool("write", false));
        assert!(!prompt_s::allow_tool("write", false));
        assert!(prompt_s::allow_tool("write", true));
        assert!(!prompt_s::allow_tool("web_fetch", true));
    }
}
