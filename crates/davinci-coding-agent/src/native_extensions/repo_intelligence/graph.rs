use super::{RepoIndex, Symbol};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Dependency {
    pub from: String,
    pub to: Option<String>,
    pub specifier: String,
    pub external: bool,
    pub reexport: bool,
}

pub(super) fn resolve(index: &RepoIndex, from: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with('.') {
        return super::modules::candidates(&index.aliases, from, specifier)
            .into_iter()
            .find_map(|base| resolve_base(index, base));
    }
    let parent = Path::new(from).parent().unwrap_or(Path::new(""));
    resolve_base(index, super::modules::normalize(&parent.join(specifier))?)
}

fn resolve_base(index: &RepoIndex, base: String) -> Option<String> {
    path_candidates(base)
        .into_iter()
        .find(|path| index.files.contains_key(path))
}

fn path_candidates(base: String) -> Vec<String> {
    let mut candidates = vec![base.clone()];
    if let Some(stem) = base
        .strip_suffix(".js")
        .or_else(|| base.strip_suffix(".jsx"))
    {
        candidates.extend([format!("{stem}.ts"), format!("{stem}.tsx")]);
    }
    if let Some(stem) = base.strip_suffix(".mjs") {
        candidates.push(format!("{stem}.mts"));
    }
    if let Some(stem) = base.strip_suffix(".cjs") {
        candidates.push(format!("{stem}.cts"));
    }
    for extension in ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"] {
        candidates.push(format!("{base}.{extension}"));
        candidates.push(format!("{base}/index.{extension}"));
    }
    candidates
}

/// Candidate paths are hypotheses for missing imports, never resolved edges.
pub(super) fn missing_candidates(index: &RepoIndex, from: &str, specifier: &str) -> Vec<String> {
    let bases = if specifier.starts_with('.') {
        let parent = Path::new(from).parent().unwrap_or(Path::new(""));
        super::modules::normalize(&parent.join(specifier))
            .into_iter()
            .collect()
    } else {
        super::modules::candidates(&index.aliases, from, specifier)
    };
    bases.into_iter().flat_map(path_candidates).collect()
}

pub(super) fn dependencies(index: &RepoIndex) -> Vec<Dependency> {
    index
        .files
        .values()
        .flat_map(|file| {
            file.imports.iter().map(|import| Dependency {
                from: file.path.clone(),
                to: resolve(index, &file.path, &import.specifier),
                specifier: import.specifier.clone(),
                external: !import.specifier.starts_with('.')
                    && super::modules::candidates(&index.aliases, &file.path, &import.specifier)
                        .is_empty(),
                reexport: import.reexport,
            })
        })
        .collect()
}

pub(super) fn distances(
    edges: &[Dependency],
    origin: &str,
    max_depth: usize,
) -> BTreeMap<String, usize> {
    let mut distances = BTreeMap::new();
    let mut seen = BTreeSet::from([origin.to_string()]);
    let mut queue = VecDeque::from([(origin.to_string(), 0)]);
    while let Some((file, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for edge in edges {
            let Some(target) = &edge.to else {
                continue;
            };
            let next = if edge.from == file {
                target
            } else if *target == file {
                &edge.from
            } else {
                continue;
            };
            if seen.insert(next.clone()) {
                distances.insert(next.clone(), depth + 1);
                queue.push_back((next.clone(), depth + 1));
            }
        }
    }
    distances
}

/// Resolve only unambiguous lexical declarations/import bindings. No spelling-only
/// cross-repository call/reference guesses and no dynamic property resolution.
pub(super) fn edge_target<'a>(
    index: &'a RepoIndex,
    file: &str,
    source: &str,
    target: &str,
) -> Option<&'a Symbol> {
    let record = index.files.get(file)?;
    if let Some(symbol) = record.symbols.iter().find(|s| s.id == target) {
        return Some(symbol);
    }
    let owner = record.symbols.iter().find(|s| s.id == source);
    if owner.is_some_and(|s| s.parameters.iter().any(|name| name == target)) {
        return None;
    }
    let candidates: Vec<_> = record
        .symbols
        .iter()
        .filter(|s| {
            s.name == target
                && (s.parent.is_none()
                    || s.parent.as_deref() == Some(source)
                    || owner.is_some_and(|o| s.parent == o.parent))
        })
        .collect();
    if candidates.len() == 1 {
        return candidates.first().copied();
    }
    for import in record.imports.iter().filter(|import| !import.reexport) {
        if let Some(exported) = import.bindings.get(target) {
            let path = resolve(index, file, &import.specifier)?;
            return exported_symbol(index, &path, exported, &mut BTreeSet::new());
        }
    }
    None
}

fn exported_symbol<'a>(
    index: &'a RepoIndex,
    path: &str,
    name: &str,
    seen: &mut BTreeSet<(String, String)>,
) -> Option<&'a Symbol> {
    if seen.len() >= 32 || !seen.insert((path.into(), name.into())) {
        return None;
    }
    let file = index.files.get(path)?;
    let local = file
        .export_bindings
        .get(name)
        .map(String::as_str)
        .unwrap_or(name);
    let mut matches = file
        .symbols
        .iter()
        .filter(|s| s.exported && s.name == local);
    if let Some(symbol) = matches.next() {
        return matches.next().is_none().then_some(symbol);
    }
    let mut matches = file
        .imports
        .iter()
        .filter(|import| import.reexport && import.bindings.contains_key(name));
    let import = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    exported_symbol(
        index,
        &resolve(index, path, &import.specifier)?,
        &import.bindings[name],
        seen,
    )
}
