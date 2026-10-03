//! Fixed offline monorepo used by before/after test-impact evaluations.
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const CHANGED: &str = "packages/auth/src/token.mjs";
pub const IMPACTED: [&str; 3] = [
    "packages/auth/test/token.test.mjs",
    "packages/auth/test/session.test.mjs",
    "packages/web/test/login.test.mjs",
];

pub fn write(root: &Path, path: &str, body: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

pub fn populate(root: &Path) -> Vec<String> {
    write(
        root,
        "package.json",
        r#"{"private":true,"workspaces":["packages/*"],"scripts":{"test":"node --test"}}"#,
    );
    for name in ["auth", "web", "other"] {
        write(
            root,
            &format!("packages/{name}/package.json"),
            &json!({"name":format!("@fixture/{name}"),"type":"module",
                "scripts":{"test":"node --test test/*.test.mjs"}})
            .to_string(),
        );
    }
    write(
        root,
        CHANGED,
        // A synthetic prefix, not a credential. Separate the JS literal from
        // surrounding Rust quotes so generic assignment scanners can parse it.
        concat!(
            "export function tokenFor(user) { return ",
            "'token:'",
            " + user; }\n"
        ),
    );
    write(
        root,
        "packages/auth/src/session.mjs",
        "import { tokenFor } from './token.mjs';\nexport function sessionFor(user) { return { token: tokenFor(user) }; }\n",
    );
    write(
        root,
        "packages/web/src/login.mjs",
        "import { sessionFor } from '../../auth/src/session.mjs';\nexport function login(user) { return sessionFor(user).token.startsWith('token:'); }\n",
    );
    for (path, import, assertion) in [
        (
            IMPACTED[0],
            "import { tokenFor } from '../src/token.mjs';",
            "assert.equal(tokenFor('alice'), 'token:alice');",
        ),
        (
            IMPACTED[1],
            "import { sessionFor } from '../src/session.mjs';",
            "assert.equal(sessionFor('alice').token, 'token:alice');",
        ),
        (
            IMPACTED[2],
            "import { login } from '../src/login.mjs';",
            "assert.equal(login('alice'), true);",
        ),
    ] {
        write_test(root, path, import, assertion);
    }
    let mut tests: Vec<String> = IMPACTED.iter().map(|p| (*p).to_string()).collect();
    for n in 0..20 {
        write(
            root,
            &format!("packages/other/src/unrelated{n}.mjs"),
            &format!("export function unrelated{n}() {{ return {n}; }}\n"),
        );
        let path = format!("packages/other/test/unrelated{n}.test.mjs");
        write_test(
            root,
            &path,
            &format!("import {{ unrelated{n} }} from '../src/unrelated{n}.mjs';"),
            &format!("assert.equal(unrelated{n}(), {n});"),
        );
        tests.push(path);
    }
    tests.sort();
    tests
}

fn write_test(root: &Path, path: &str, import: &str, assertion: &str) {
    write(
        root,
        path,
        &format!(
            "import test from 'node:test';\nimport assert from 'node:assert/strict';\n{import}\ntest('{path}', () => {{ {assertion} }});\n"
        ),
    );
}

pub fn plant_failure(root: &Path) {
    write(
        root,
        CHANGED,
        "export function tokenFor(user) { return 'wrong:' + user; }\n",
    );
}

/// Only executes generated, finite fixtures; no project scripts or dependencies.
/// Each run gets a process group and a timeout; output goes to temporary files.
pub fn run_tests(root: &Path, tests: &[String]) -> Value {
    assert!(
        !tests.is_empty(),
        "an empty selection must never count as verification"
    );
    let stdout = tempfile::tempfile().unwrap();
    let stderr = tempfile::tempfile().unwrap();
    let mut command = Command::new("node");
    command
        .args(["--test", "--test-reporter=tap"])
        .args(tests)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let start = Instant::now();
    let mut child = command
        .spawn()
        .expect("explicit evaluation requires installed Node.js");
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            davinci_agent::jobs::kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            panic!("Node fixture exceeded its 30-second deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let output = bounded_text(stdout);
    let errors = bounded_text(stderr);
    let count = |name: &str| {
        output
            .lines()
            .find_map(|line| line.strip_prefix(&format!("# {name} ")))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or_else(|| panic!("missing TAP {name} count: {output}\n{errors}"))
    };
    let failures: BTreeSet<String> = output
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("not ok "))
        .filter_map(|line| line.split_once(" - ").map(|(_, name)| name.to_string()))
        .collect();
    assert_eq!(
        count("tests"),
        tests.len(),
        "each fixture path must execute one test"
    );
    assert_eq!(failures.len(), count("fail"));
    assert_eq!(status.success(), failures.is_empty());
    json!({
        "program":"node","argv":["--test","--test-reporter=tap"],
        "paths":tests,"test_count":count("tests"),"passed":count("pass"),
        "failed":count("fail"),"failure_paths":failures,"exit_code":status.code(),
        "runtime_ms":elapsed_ms,"stdout_bytes":output.len(),"stderr":errors,
    })
}

fn bounded_text(mut file: fs::File) -> String {
    use std::io::{Read, Seek, SeekFrom};
    assert!(
        file.metadata().unwrap().len() <= 256 * 1024,
        "unexpected fixture output"
    );
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}
