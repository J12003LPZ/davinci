use davinci_ai::{calculate_usage, load_builtin_models};

fn sol() -> davinci_ai::Model {
    load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai" && model.id == "gpt-5.6-sol")
        .expect("gpt-5.6-sol")
}

#[test]
fn pricing_tier_uses_raw_input_total_before_cache_bucket_split() {
    let model = sol();

    let at_threshold = calculate_usage(&model, 100, 10, 271_900, 0);
    let above_threshold = calculate_usage(&model, 100, 10, 271_901, 0);

    assert!((at_threshold.cost.input - 0.0004).abs() < 1e-12);
    assert!((at_threshold.cost.cache_read - (271_900.0 / 1_000_000.0 * 0.4)).abs() < 1e-12);

    assert!((above_threshold.cost.input - 0.0008).abs() < 1e-12);
    assert!((above_threshold.cost.cache_read - (271_901.0 / 1_000_000.0 * 0.8)).abs() < 1e-12);
    assert!((above_threshold.cost.output - (10.0 / 1_000_000.0 * 30.0)).abs() < 1e-12);
}

#[test]
fn cache_write_is_a_disjoint_price_bucket() {
    let model = sol();
    let usage = calculate_usage(&model, 100, 0, 400, 500);

    assert_eq!(usage.total_tokens, 1_000);
    assert!((usage.cost.input - (100.0 / 1_000_000.0 * 4.0)).abs() < 1e-12);
    assert!((usage.cost.cache_read - (400.0 / 1_000_000.0 * 0.4)).abs() < 1e-12);
    assert!((usage.cost.cache_write - (500.0 / 1_000_000.0 * 5.0)).abs() < 1e-12);
}
