use davinci_agent::process_manager::BrowserLeaseIdentity;
use davinci_agent::runtime::operations::{
    BrowserOperationAdapter, BrowserOperationDisposition, CallerType, ExecutionOwner,
    ExecutionOwnerId, JournalId, JournalIdentity, OperationContext, OperationJournal,
    RootNamespaceId, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_agent::ToolResult;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    runtime: RuntimeHandle,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let root_namespace_id = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let journal = Arc::new(
            OperationJournal::open(
                &temp_root.join("operations"),
                identity.clone(),
                root_namespace_id.clone(),
            )
            .unwrap(),
        );
        let context = OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id,
            session_id: "browser-operation-test".into(),
            runtime_run_id: run_id,
            parent_operation_id: None,
            agent_id,
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace,
            caller: CallerType::HostControl,
            wire_tool_call_id: None,
        };
        let operations = ToolOperationRuntime::new(
            journal,
            context,
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            &temp_root,
        )
        .unwrap();
        let runtime = RuntimeHandle::new(run_id, agent_id, RuntimeBus::new())
            .with_operation_runtime(operations);
        Self {
            _temp: temp,
            runtime,
        }
    }

    fn adapter(&self) -> BrowserOperationAdapter {
        BrowserOperationAdapter::from_runtime(&self.runtime).unwrap()
    }

    fn lease(&self) -> BrowserLeaseIdentity {
        BrowserLeaseIdentity {
            process_id: 42,
            owner: Uuid::new_v4(),
            parent_owner: None,
            session: Uuid::new_v4(),
            lifetime: Uuid::new_v4(),
            port: 3000,
            ipv6: false,
            pid: 4242,
            pid_birth: Some(7),
        }
    }
}

#[test]
fn browser_operation_latches_once_and_replays_sanitized_result() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let lease = fixture.lease();
    let args = json!({
        "browser_id": Uuid::new_v4().to_string(),
        "selector": {"kind": "role", "role": "textbox", "name": "Password"},
        "text": "super-secret-password",
        "authorization": "Bearer credential-value"
    });
    let operation = adapter
        .start(
            "call-browser-1",
            "browser_type",
            &args,
            args["browser_id"].as_str().map(str::to_owned),
            None,
            lease.clone(),
            0,
        )
        .unwrap();
    assert_eq!(operation.disposition(), BrowserOperationDisposition::New);
    operation.begin(false, || Ok(()), || ()).unwrap();

    let result = ToolResult {
        content: json!({"status":"observed","text":"super-secret-password"}).to_string(),
        is_error: false,
        details: Some(json!({
            "result": {
                "text": "super-secret-password",
                "network": {"headers": {"authorization": "Bearer credential-value"}}
            }
        })),
    };
    operation.complete(&result).unwrap();

    let replay = adapter
        .start(
            "call-browser-1",
            "browser_type",
            &args,
            args["browser_id"].as_str().map(str::to_owned),
            None,
            lease,
            0,
        )
        .unwrap();
    assert_eq!(
        replay.disposition(),
        BrowserOperationDisposition::ExistingResult
    );
    let replayed = replay.replay_result().unwrap();
    let encoded = serde_json::to_string(&replayed).unwrap();
    assert!(!encoded.contains("super-secret-password"));
    assert!(!encoded.contains("credential-value"));
    assert!(
        replayed.details.unwrap()["replayed_from_operation_journal"]
            .as_bool()
            .unwrap_or(false)
            || encoded.contains("replayed_from_operation_journal")
    );
}

#[test]
fn browser_operation_refuses_duplicate_in_flight_call() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let lease = fixture.lease();
    let args = json!({"browser_id": Uuid::new_v4().to_string()});
    let first = adapter
        .start(
            "call-browser-in-flight",
            "browser_snapshot",
            &args,
            args["browser_id"].as_str().map(str::to_owned),
            None,
            lease.clone(),
            0,
        )
        .unwrap();
    let second = adapter
        .start(
            "call-browser-in-flight",
            "browser_snapshot",
            &args,
            args["browser_id"].as_str().map(str::to_owned),
            None,
            lease,
            0,
        )
        .unwrap();
    assert_eq!(
        second.disposition(),
        BrowserOperationDisposition::ExistingInFlight
    );
    assert!(second.begin(false, || Ok(()), || ()).is_err());
    first.begin(false, || Ok(()), || ()).unwrap();
    first
        .complete(&ToolResult {
            content: "{}".into(),
            is_error: false,
            details: Some(json!({"status":"observed"})),
        })
        .unwrap();
}

#[test]
fn browser_action_variants_share_the_same_durable_boundary() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let lease = fixture.lease();
    let browser_id = Uuid::new_v4().to_string();
    for (index, (action, args)) in [
        (
            "browser_open",
            json!({"process_id":42,"port":3000,"path":"/"}),
        ),
        (
            "browser_click",
            json!({"browser_id":browser_id.clone(),"selector":{"kind":"role","role":"button","name":"Save"}}),
        ),
        (
            "browser_type",
            json!({"browser_id":browser_id.clone(),"selector":{"kind":"label","value":"Name"},"text":"private"}),
        ),
        (
            "browser_select",
            json!({"browser_id":browser_id.clone(),"selector":{"kind":"test_id","value":"country"},"value":"US"}),
        ),
        ("browser_close", json!({"browser_id":browser_id.clone()})),
    ]
    .into_iter()
    .enumerate()
    {
        let operation = adapter
            .start(
                &format!("call-browser-action-{index}"),
                action,
                &args,
                args["browser_id"].as_str().map(str::to_owned),
                None,
                lease.clone(),
                0,
            )
            .unwrap();
        assert_eq!(operation.disposition(), BrowserOperationDisposition::New);
        operation.begin(false, || Ok(()), || ()).unwrap();
        operation
            .complete(&ToolResult {
                content: json!({"status":"observed","action":action}).to_string(),
                is_error: false,
                details: Some(json!({"action":action})),
            })
            .unwrap();
    }
}
