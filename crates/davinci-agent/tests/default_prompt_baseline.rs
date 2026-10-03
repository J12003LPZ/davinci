use davinci_agent::default_system_prompt;
use sha2::{Digest, Sha256};

#[test]
fn default_prompt_v1_matches_committed_baseline() {
    let actual = default_system_prompt();
    let expected = include_str!("fixtures/default_system_prompt_v1.txt").replace("\r\n", "\n");

    assert_eq!(actual, expected);

    let hash = format!("{:x}", Sha256::digest(actual.as_bytes()));
    assert!(!hash.is_empty());
}
