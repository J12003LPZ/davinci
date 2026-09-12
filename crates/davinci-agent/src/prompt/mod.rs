//! Versioned prompt engineering subsystem.

pub mod astra;
pub mod bundle;
pub mod capabilities;
pub mod coding;
pub mod collaboration;
pub mod composer;
pub mod core;
pub mod manifest;
pub mod model_policy;
pub mod provider;
pub mod runtime_state;
pub mod session;
pub mod tool_strategy;
pub mod turn;
pub mod verification;
pub mod version;

pub use bundle::{
    legacy_bundle, preview_bundle, stable_bundle, validate_ab_comparison, PromptBundle,
    PromptCandidateDescriptor,
};
pub use capabilities::{
    capability_module, detect_native_capabilities, evaluate_completion, incomplete_evidence_reason,
    route_capabilities, CapabilityDecision, CapabilityEvidence, CapabilityEvidenceKind,
    CapabilityGateOutcome, CapabilityRouterInput, CapabilityRunState, DebuggingState,
    FrontendDesignState, NativeBehaviorCapability, ReproducerEvidence, ReviewState,
    CAPABILITY_POLICY_MAX_TOKENS, DEBUGGING_REPRODUCER_REASON_CODE, FRONTEND_SNAPSHOT_REASON_CODE,
    MAX_CAPABILITY_COMPLETION_REMINDERS,
};
pub use coding::{
    coding_change_quality_module, coding_exploration_module, coding_scope_discipline_module,
};
pub use collaboration::collaboration_user_intent_module;
pub use composer::{
    compose_default_prompt, compose_legacy_default, compose_modules, compose_profile_prompt,
    compose_with_mutations, ComposedPrompt, PromptCacheClass, PromptContext, PromptModule,
    PromptMutation,
};
pub use core::{core_autonomy_module, core_identity_module};
pub use manifest::{PromptManifest, PromptModuleIdentity};
pub use model_policy::{
    apply_model_policy, prompt_model_policy, PromptModelPolicy, GPT6_ASTRA_POLICY_VERSION,
};
pub use provider::{
    prompt_model_family, provider_adapter, PromptModelFamily, PROVIDER_ADAPTER_MAX_TOKENS,
};
pub use runtime_state::{runtime_state_module, runtime_state_text, RuntimePromptState};
pub use session::{
    resolve_resume_prompt_session, PromptSessionRecord, PromptSessionState, PromptSource,
};
pub use tool_strategy::tool_strategy_module;
pub use turn::{compose_turn_prompt, PreparedTurnPrompt};
pub use verification::verification_completion_module;
pub use version::{
    PromptProfile, LEGACY_PROMPT_VERSION, PREVIEW_PROMPT_VERSION, STABLE_PROMPT_VERSION,
};
