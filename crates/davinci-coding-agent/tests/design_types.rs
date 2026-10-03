use davinci_coding_agent::design::types::*;
use std::collections::BTreeMap;

fn bundle() -> SourceBundle {
    SourceBundle {
        files: BTreeMap::from([("index.html".into(), source_ref(5))]),
        entry_points: vec!["index.html".into()],
    }
}
fn source_ref(size: u64) -> davinci_coding_agent::design::types::ArtifactRef {
    serde_json::from_value(serde_json::json!({
        "id":"artifact_aaaaaaaaaaaa", "sha256":"a".repeat(64), "media_type":"text/html",
        "size":size.to_string(), "relative_store_path":format!("{}.bin", "a".repeat(64)), "redaction":null
    })).unwrap()
}
#[test]
fn revision_keeps_all_u64_bits() {
    let rev = parse_revision_decimal("9007199254740993").unwrap();
    assert_eq!(serde_json::to_string(&rev).unwrap(), "\"9007199254740993\"");
    assert_eq!(
        serde_json::from_str::<RevisionId>("\"9007199254740993\"").unwrap(),
        rev
    );
    for invalid in ["", "-1", "+1", "01", "1.0", "18446744073709551616"] {
        assert!(parse_revision_decimal(invalid).is_err(), "{invalid}");
    }
    assert!(serde_json::from_str::<RevisionId>("1").is_err());
    let reference = source_ref(u64::MAX);
    let mut value = serde_json::to_value(&reference).unwrap();
    assert_eq!(value["size"], u64::MAX.to_string());
    value["unknown"] = true.into();
    assert!(serde_json::from_value::<ArtifactRef>(value).is_err());
}
#[test]
fn source_bundle_round_trips_and_rejects_unknown_fields() {
    let value = bundle();
    assert!(validate_bundle(&value, &DesignLimits::default()).is_ok());
    let encoded = serde_json::to_string(&value).unwrap();
    assert_eq!(
        serde_json::from_str::<SourceBundle>(&encoded).unwrap(),
        value
    );
    let mut json = serde_json::to_value(value).unwrap();
    json["extra"] = true.into();
    assert!(serde_json::from_value::<SourceBundle>(json).is_err());
    assert!(serde_json::from_str::<SourceBundle>(
        r#"{"files":{"index.html":{},"index.html":{}},"entry_points":[]}"#
    )
    .is_err());
}
#[test]
fn paths_are_portable_and_unambiguous() {
    for path in [
        "../evil", "/evil", "C:/evil", "C:evil", "a\\b", "a//b", "./a", "a/../b", "AUX.txt",
        "a/COM1", "x.", "x ", "a:b", "a\0b",
    ] {
        let mut b = bundle();
        b.files.insert(path.into(), source_ref(1));
        assert!(
            validate_bundle(&b, &DesignLimits::default()).is_err(),
            "{path:?}"
        );
    }
    let mut b = bundle();
    b.files.insert("INDEX.HTML".into(), source_ref(1));
    assert!(validate_bundle(&b, &DesignLimits::default()).is_err());
}
#[test]
fn source_limits_apply_before_storage() {
    let limits = DesignLimits::default();
    let mut b = bundle();
    b.files
        .insert("large.css".into(), source_ref(256 * 1024 + 1));
    assert!(validate_bundle(&b, &limits).is_err());
    b = bundle();
    for n in 0..64 {
        b.files.insert(format!("file{n}.css"), source_ref(1));
    }
    assert!(validate_bundle(&b, &limits).is_err());
    b = bundle();
    for n in 0..8 {
        b.files
            .insert(format!("file{n}.css"), source_ref(256 * 1024));
    }
    assert!(validate_bundle(&b, &limits).is_err());
}
#[test]
fn schema_ids_and_artboard_limits_are_checked() {
    assert!(serde_json::from_str::<SchemaVersion>("2").is_err());
    assert!(serde_json::from_str::<ArtifactId>("\"../foreign\"").is_err());
    let id = ArtboardId::new();
    let variants = vec![Variant {
        id: VariantId::new(),
        title: "A".into(),
        artboards: vec![
            Artboard {
                id,
                title: "One".into(),
                entry_point: "index.html".into(),
            },
            Artboard {
                id,
                title: "Two".into(),
                entry_point: "index.html".into(),
            },
        ],
    }];
    assert!(validate_variants(&variants, &bundle(), &DesignLimits::default()).is_err());
}
