use crate::catalog::Model;
use crate::types::{Usage, UsageCost};

/// Cost is USD per million tokens, matching TypeScript model catalogs.
pub fn usage_cost(model: &Model, usage: &Usage) -> UsageCost {
    let million = 1_000_000.0;
    let input = (usage.input as f64) * model.cost.input / million;
    let output = (usage.output as f64) * model.cost.output / million;
    let cache_read = (usage.cache_read as f64) * model.cost.cache_read / million;
    let cache_write = (usage.cache_write as f64) * model.cost.cache_write / million;
    UsageCost {
        input,
        output,
        cache_read,
        cache_write,
        total: input + output + cache_read + cache_write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::get_builtin_model;

    #[test]
    fn anthropic_cost_matches_catalog_units() {
        let model = get_builtin_model("anthropic", "claude-sonnet-4-5")
            .or_else(|| {
                crate::catalog::list_models(Some("anthropic"))
                    .into_iter()
                    .next()
            })
            .expect("anthropic catalog");
        let cost = usage_cost(
            &model,
            &Usage {
                input: 1_000_000,
                output: 0,
                ..Usage::default()
            },
        );
        assert!((cost.input - model.cost.input).abs() < 1e-9);
    }
}
