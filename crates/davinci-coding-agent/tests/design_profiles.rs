use davinci_coding_agent::design::{
    context::compile_design_context, skills::select_profile, types::DesignKind,
};

#[test]
fn profiles_route_scope_and_preserve_complete_sections() {
    let landing = select_profile(DesignKind::Landing, &[]).unwrap();
    let product = select_profile(DesignKind::Product, &[]).unwrap();
    let document = select_profile(DesignKind::Document, &[]).unwrap();
    assert_eq!(landing.dials, [7, 4, 4]);
    assert_eq!(product.dials, [4, 2, 7]);
    assert_eq!(document.dials, [5, 1, 4]);
    assert_eq!(
        landing
            .sections
            .iter()
            .map(|s| s.number)
            .collect::<Vec<_>>(),
        [0, 1, 11, 13, 14]
    );
    assert!(product.sections.is_empty());
    assert!(document.sections.is_empty());
    let context =
        compile_design_context(&landing, "Keep the user's purple brand", None, 12_000).unwrap();
    assert!(context.text.contains("purple"));
    assert!(context.text.contains("## 14."));
    assert!(context.estimated_tokens <= 12_000);
    assert!(compile_design_context(&landing, "brief", None, 10).is_err());
}

#[test]
fn changed_profiles_and_invalid_overrides_are_rejected() {
    let mut profile =
        select_profile(DesignKind::Landing, &["Preserve purple brand".into()]).unwrap();
    profile.sections[0].text.push('x');
    assert!(compile_design_context(&profile, "brief", None, 12_000).is_err());
    let mut profile = select_profile(DesignKind::Product, &[]).unwrap();
    profile.dials[0] = 11;
    assert!(compile_design_context(&profile, "brief", None, 12_000).is_err());
}
