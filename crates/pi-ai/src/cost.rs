use crate::types::{Model, Usage};

pub fn calculate_cost(model: &Model, usage: &mut Usage) -> f64 {
    let input_tokens = usage.input + usage.cache_read + usage.cache_write;
    let mut rates = model.cost.rates.clone();
    let mut matched_threshold: i64 = -1;

    if let Some(tiers) = &model.cost.tiers {
        for tier in tiers {
            if input_tokens > tier.input_tokens_above
                && (tier.input_tokens_above as i64) > matched_threshold
            {
                rates = tier.rates.clone();
                matched_threshold = tier.input_tokens_above as i64;
            }
        }
    }

    let long_write = usage.cache_write_1h.unwrap_or(0);
    let short_write = usage.cache_write.saturating_sub(long_write);

    usage.cost.input = (rates.input / 1_000_000.0) * (usage.input as f64);
    usage.cost.output = (rates.output / 1_000_000.0) * (usage.output as f64);
    usage.cost.cache_read = (rates.cache_read / 1_000_000.0) * (usage.cache_read as f64);
    usage.cost.cache_write = (rates.cache_write * (short_write as f64)
        + rates.input * 2.0 * (long_write as f64))
        / 1_000_000.0;
    usage.cost.total =
        usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;

    usage.cost.total
}

pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}
