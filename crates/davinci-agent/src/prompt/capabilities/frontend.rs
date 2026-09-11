//! Frontend design capability module and policy.

use super::NativeBehaviorCapability;
use crate::prompt::composer::{PromptCacheClass, PromptModule};

pub const FRONTEND_DESIGN_POLICY: &str = "\
<frontend_design_policy>
When crafting frontend user interfaces and visual experiences:
1. Ground visual choices in the product's identity, target audience, and intended emotional tone.
2. Inspect existing design systems, typography, color palettes, spacing conventions, and CSS tokens before introducing new styles.
3. Establish a clear visual hierarchy with intentional contrast, harmonious proportions, and deliberate typography scales.
4. Avoid generic placeholder UI patterns, monotonous gray-on-gray palettes, or artificial clutter. Strive for polished, distinctive interfaces with character.
5. Ensure responsive design across viewport sizes, robust accessibility (proper ARIA semantics, keyboard navigation, visible focus indicators, color contrast), and fluid transitions.
6. When browser or screenshot tools are available, visually inspect rendered output to verify spacing, contrast, and alignment before completing the task.
7. Do not activate visual redesigns for simple bug fixes, component renames, or behavioral logic changes.
</frontend_design_policy>";

pub fn frontend_design_module() -> PromptModule {
    PromptModule {
        id: NativeBehaviorCapability::FrontendDesign.id().to_string(),
        version: 1,
        cache_class: PromptCacheClass::Dynamic,
        body: FRONTEND_DESIGN_POLICY.to_string(),
    }
}
