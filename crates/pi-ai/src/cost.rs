use crate::types::{ModelCost, ModelCostRates, Usage, UsageCost};

pub fn calculate_cost(model_cost: &ModelCost, usage: &Usage) -> UsageCost {
    let mut rates = ModelCostRates {
        input: model_cost.input,
        output: model_cost.output,
        cache_read: model_cost.cache_read,
        cache_write: model_cost.cache_write,
    };

    for tier in &model_cost.tiers {
        if usage.input >= tier.input_tokens_above {
            rates.input = tier.input;
            rates.output = tier.output;
            rates.cache_read = tier.cache_read;
            rates.cache_write = tier.cache_write;
        }
    }

    let input_cost = (usage.input as f64 * rates.input) / 1_000_000.0;
    let output_cost = (usage.output as f64 * rates.output) / 1_000_000.0;
    let cache_read_cost = (usage.cache_read as f64 * rates.cache_read) / 1_000_000.0;
    let cache_write_cost = (usage.cache_write as f64 * rates.cache_write) / 1_000_000.0;
    let total = input_cost + output_cost + cache_read_cost + cache_write_cost;

    UsageCost {
        input: input_cost,
        output: output_cost,
        cache_read: cache_read_cost,
        cache_write: cache_write_cost,
        total,
    }
}
