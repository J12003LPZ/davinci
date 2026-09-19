use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceBand {
    High,
    Medium,
    Abstain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoulBand {
    StrongNo,
    PossibleNo,
    Abstain,
    Uncertain,
    ActionableYes,
    StrongYes,
}

pub fn confidence_band(confidence: f32) -> ConfidenceBand {
    if confidence >= 0.85 {
        ConfidenceBand::High
    } else if confidence >= 0.60 {
        ConfidenceBand::Medium
    } else {
        ConfidenceBand::Abstain
    }
}

pub fn noul_band(value: f32) -> NoulBand {
    if value >= 0.90 {
        NoulBand::StrongYes
    } else if value >= 0.85 {
        NoulBand::ActionableYes
    } else if value >= 0.70 {
        NoulBand::Uncertain
    } else if value >= 0.30 {
        NoulBand::Abstain
    } else if value >= 0.10 {
        NoulBand::PossibleNo
    } else {
        NoulBand::StrongNo
    }
}

pub fn distribution_margin(distribution: &BTreeMap<String, f32>) -> Option<f32> {
    let mut values: Vec<f32> = distribution.values().copied().collect();
    if values.len() < 2 {
        return None;
    }
    values.sort_by(|left, right| right.total_cmp(left));
    Some(values[0] - values[1])
}
