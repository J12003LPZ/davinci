use super::ContextRoot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldReason {
    Manual,
    PhaseBoundary,
    DeltaDepth,
    DeltaTokens,
    WindowPressure,
}

impl FoldReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PhaseBoundary => "phase_boundary",
            Self::DeltaDepth => "delta_depth",
            Self::DeltaTokens => "delta_tokens",
            Self::WindowPressure => "window_pressure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextFoldDecision {
    pub should_fold: bool,
    pub reason: Option<FoldReason>,
}

#[derive(Debug, Clone, Copy)]
pub struct ContextFoldPolicy {
    pub max_delta_pages: usize,
    pub max_delta_tokens: u64,
    pub window_pressure_percent: u8,
}

impl ContextFoldPolicy {
    pub fn decide(
        &self,
        root: &ContextRoot,
        delta_tokens: u64,
        compiled_tokens: u64,
        context_window: u64,
        manual: bool,
        phase_boundary: bool,
    ) -> ContextFoldDecision {
        let reason = if manual {
            Some(FoldReason::Manual)
        } else if phase_boundary {
            Some(FoldReason::PhaseBoundary)
        } else if context_window > 0
            && compiled_tokens.saturating_mul(100)
                >= context_window.saturating_mul(self.window_pressure_percent as u64)
        {
            Some(FoldReason::WindowPressure)
        } else if root.deltas.len() >= self.max_delta_pages {
            Some(FoldReason::DeltaDepth)
        } else if delta_tokens >= self.max_delta_tokens {
            Some(FoldReason::DeltaTokens)
        } else {
            None
        };
        ContextFoldDecision {
            should_fold: reason.is_some(),
            reason,
        }
    }
}
