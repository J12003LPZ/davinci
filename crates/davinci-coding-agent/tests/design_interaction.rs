use davinci_coding_agent::design::{interaction::*, records::*, types::*};

#[test]
fn prototype_actions_are_bounded_and_never_accept_script_or_external_navigation() {
    let request = InteractionRequest {
        render: RenderRequest {
            artifact_id: ArtifactId::new(),
            revision: RevisionId(1),
            artboard_id: ArtboardId::new(),
            viewport: Viewport::defaults()[0].clone(),
            theme: Theme::Light,
            fixture: "default".into(),
            reduced_motion: true,
        },
        actions: vec![
            PrototypeAction::Click {
                selector: PrototypeSelector::Role {
                    role: "button".into(),
                    name: "Continue".into(),
                },
            },
            PrototypeAction::ExpectText {
                text: "Success".into(),
            },
        ],
        operation_id: OperationId::new(),
    };
    request.validate().unwrap();
    let mut too_many = request.clone();
    too_many.actions = vec![request.actions[0].clone(); 17];
    assert!(too_many.validate().is_err());
    let mut too_large = request;
    too_large.actions = vec![PrototypeAction::Type {
        selector: PrototypeSelector::Label {
            value: "Name".into(),
        },
        text: "x".repeat(4097),
    }];
    assert!(too_large.validate().is_err());
    assert!(serde_json::from_str::<PrototypeAction>(
        r#"{"action":"evaluate","script":"fetch('/api')"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<PrototypeAction>(
        r#"{"action":"navigate","url":"http://localhost"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<PrototypeAction>(
        r#"{"action":"click","selector":{"kind":"css","value":"body"}}"#
    )
    .is_err());
}
