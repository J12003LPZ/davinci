#[test]
fn no_module_is_compiled_by_both_crate_roots() {
    let main = include_str!("../src/main.rs");
    let lib = include_str!("../src/lib.rs");
    let mods = |src: &str| -> std::collections::BTreeSet<String> {
        src.lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line
                    .strip_prefix("pub mod ")
                    .or_else(|| line.strip_prefix("mod "))?;
                rest.strip_suffix(';').map(str::to_string)
            })
            .collect()
    };
    let both: Vec<_> = mods(main).intersection(&mods(lib)).cloned().collect();
    assert!(both.is_empty(), "compiled twice: {both:?}");
}
