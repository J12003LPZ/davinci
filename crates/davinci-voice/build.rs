fn main() {
    #[cfg(feature = "native")]
    {
        println!("cargo:rerun-if-changed=native");
        let target = std::env::var("TARGET").unwrap_or_default();
        println!("cargo:rerun-if-env-changed=DAVINCI_VOICE_NATIVE_CPU");
        let native_cpu = match std::env::var("DAVINCI_VOICE_NATIVE_CPU").as_deref() {
            Ok("1") => {
                assert_eq!(
                    std::env::var("HOST").unwrap(),
                    target,
                    "DAVINCI_VOICE_NATIVE_CPU requires a build for the host machine"
                );
                "ON"
            }
            Ok("0") | Err(std::env::VarError::NotPresent) => "OFF",
            _ => panic!("DAVINCI_VOICE_NATIVE_CPU must be 0 or 1"),
        };
        // The cmake helper otherwise inherits opt-level=0 from cargo test even
        // with a Release configuration, making CPU inference impractical.
        let optimization = if target.contains("msvc") {
            "/O2"
        } else {
            "-O3"
        };
        let out = cmake::Config::new("native")
            .profile("Release")
            .define("DAVINCI_VOICE_NATIVE_CPU", native_cpu)
            .cflag(optimization)
            .cxxflag(optimization)
            .build();
        println!("cargo:rustc-link-search=native={}/lib", out.display());
        for lib in [
            "davinci_voice_shim",
            "whisper",
            "ggml",
            "ggml-base",
            "ggml-cpu",
        ] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        if target.contains("apple") {
            println!("cargo:rustc-link-lib=c++");
        } else if !target.contains("msvc") {
            println!("cargo:rustc-link-lib=stdc++");
        }
    }
}
