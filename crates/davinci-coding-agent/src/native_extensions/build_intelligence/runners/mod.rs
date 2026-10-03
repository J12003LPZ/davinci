pub mod frameworks;
pub mod nx;
pub mod tsconfig;
pub mod turborepo;

#[allow(unused_imports)]
pub use frameworks::{detect_framework, detect_package_manager, FrameworkKind, PackageManager};
#[allow(unused_imports)]
pub use nx::{extract_nx_targets, parse_nx_json, parse_project_json, NxConfig, NxProjectConfig};
#[allow(unused_imports)]
pub use tsconfig::{parse_tsconfig, Tsconfig};
#[allow(unused_imports)]
pub use turborepo::{extract_turbo_targets, parse_turbo_json, TurboConfig};
