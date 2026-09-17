use super::symbols::digest;
use super::{Edge, Import, LanguageAdapter, RepoFile, SourceRange, Symbol};
use std::collections::BTreeMap;
use tree_sitter::Node;

const MAX_SOURCE_BYTES: usize = 1_000_000;
const MAX_RECORDS: usize = 20_000;

fn parameters(node: Node<'_>, source: &str) -> Vec<String> {
    let function = node.child_by_field_name("value").unwrap_or(node);
    let mut stack: Vec<_> = function
        .child_by_field_name("parameters")
        .or_else(|| function.child_by_field_name("parameter"))
        .into_iter()
        .collect();
    let mut names = Vec::new();
    while let Some(parameter) = stack.pop() {
        if matches!(parameter.kind(), "type_annotation" | "predefined_type") {
            continue;
        }
        if parameter.kind() == "identifier" {
            names.push(text(parameter, source).into());
        }
        let mut cursor = parameter.walk();
        stack.extend(parameter.named_children(&mut cursor));
    }
    names.sort();
    names.dedup();
    names.truncate(256);
    names
}

pub fn parse_source(path: &str, source: &str) -> Result<RepoFile, String> {
    let language = LanguageAdapter::for_path(path).ok_or("unsupported_language")?;
    if source.len() > MAX_SOURCE_BYTES {
        return Err("result_limit_exceeded: source byte limit".into());
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language.grammar(path))
        .map_err(|e| e.to_string())?;
    parser.set_timeout_micros(250_000);
    let mut file = RepoFile {
        path: path.into(),
        language,
        content_hash: digest(source.as_bytes()),
        size: source.len(),
        module_identity: path.into(),
        imports: Vec::new(),
        exports: Vec::new(),
        export_bindings: BTreeMap::new(),
        symbols: Vec::new(),
        edges: Vec::new(),
        parse_status: "ok".into(),
        unresolved: Vec::new(),
    };
    let Some(tree) = parser.parse(source, None) else {
        file.parse_status = "parse_failed: parser budget exhausted".into();
        return Ok(file);
    };
    if tree.root_node().has_error() {
        file.parse_status = "parse_failed: incomplete or malformed syntax".into();
    }
    let mut stack = vec![(tree.root_node(), None::<usize>, false)];
    let mut identities = BTreeMap::<String, usize>::new();
    let mut visited = 0usize;
    while let Some((node, parent, exported)) = stack.pop() {
        visited += 1;
        if visited > 200_000
            || file.symbols.len() + file.edges.len() + file.imports.len() >= MAX_RECORDS
        {
            file.parse_status = "parse_failed: structural record budget exhausted".into();
            break;
        }
        let exported = exported || node.kind() == "export_statement";
        extract_links(node, parent, source, &mut file);
        let mut next_parent = parent;
        if let Some(kind) = symbol_kind(node) {
            let declared_name = node
                .child_by_field_name("name")
                .map(|n| text(n, source).to_string())
                .or_else(|| (exported && parent.is_none()).then(|| "default".to_string()));
            if let Some(name) = declared_name {
                if !name.is_empty() && name.len() <= 256 {
                    let qualified = parent
                        .map(|i| format!("{}.{}", file.symbols[i].qualified_name, name))
                        .unwrap_or_else(|| name.clone());
                    let identity = format!("{path}:{kind}:{qualified}");
                    let ordinal = identities.entry(identity.clone()).or_default();
                    let id = digest(format!("{identity}:{}", *ordinal).as_bytes());
                    *ordinal += 1;
                    let parent_id = parent.map(|i| file.symbols[i].id.clone());
                    if let Some(owner) = &parent_id {
                        file.edges.push(Edge {
                            source: owner.clone(),
                            target: id.clone(),
                            kind: "contains".into(),
                            line: node.start_position().row + 1,
                        });
                    }
                    let signature_end = node
                        .child_by_field_name("body")
                        .map(|body| body.start_byte())
                        .unwrap_or(node.end_byte());
                    let signature = source
                        .get(node.start_byte()..signature_end)
                        .unwrap_or("")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(240)
                        .collect();
                    let is_exported = exported && parent.is_none();
                    if is_exported {
                        file.exports.push(name.clone());
                        file.export_bindings
                            .entry(name.clone())
                            .or_insert_with(|| name.clone());
                        file.edges.push(Edge {
                            source: path.into(),
                            target: id.clone(),
                            kind: "exports".into(),
                            line: node.start_position().row + 1,
                        });
                    }
                    next_parent = Some(file.symbols.len());
                    file.symbols.push(Symbol {
                        id,
                        name,
                        kind: kind.into(),
                        file: path.into(),
                        qualified_name: qualified,
                        range: range(node),
                        exported: is_exported,
                        parent: parent_id,
                        signature,
                        parameters: parameters(node, source),
                    });
                }
            }
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            // Export status belongs to the declaration, not its nested locals.
            stack.push((child, next_parent, exported && next_parent.is_none()));
        }
    }
    // Export clauses can appear before or after declarations.
    for symbol in &mut file.symbols {
        if symbol.parent.is_none() && file.exports.contains(&symbol.name) {
            symbol.exported = true;
        }
    }
    file.exports.sort();
    file.exports.dedup();
    file.unresolved.sort();
    file.unresolved.dedup();
    file.unresolved.truncate(32);
    Ok(file)
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn range(node: Node<'_>) -> SourceRange {
    SourceRange {
        start_line: node.start_position().row + 1,
        start_column: node.start_position().column,
        end_line: node.end_position().row + 1,
        end_column: node.end_position().column,
    }
}

fn symbol_kind(node: Node<'_>) -> Option<&'static str> {
    Some(match node.kind() {
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            "function"
        }
        "function_expression" | "arrow_function"
            if node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement") =>
        {
            "function"
        }
        "class"
            if node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement") =>
        {
            "class"
        }
        "class_declaration" | "abstract_class_declaration" => "class",
        "method_definition" | "method_signature" | "abstract_method_signature" => "method",
        "interface_declaration" => "interface",
        "type_alias_declaration" => "type_alias",
        "enum_declaration" => "enum",
        "internal_module" | "module" => "namespace",
        "public_field_definition" | "property_signature" => "property",
        "variable_declarator" => match node.child_by_field_name("value").map(|n| n.kind()) {
            Some("arrow_function" | "function_expression" | "generator_function") => "function",
            Some("class") => "class",
            _ => "variable",
        },
        _ => return None,
    })
}

fn push_edge(file: &mut RepoFile, parent: Option<usize>, target: &str, kind: &str, node: Node<'_>) {
    if target.is_empty() || target.len() > 512 {
        return;
    }
    file.edges.push(Edge {
        source: parent
            .map(|i| file.symbols[i].id.clone())
            .unwrap_or_else(|| file.path.clone()),
        target: target.into(),
        kind: kind.into(),
        line: node.start_position().row + 1,
    });
}

fn literal(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let raw = text(node, source);
    let body = raw.get(1..raw.len().checked_sub(1)?)?;
    // Escapes require JS decoding; do not invent a resolved module name.
    (!body.contains('\\') && body.len() <= 512).then(|| body.to_string())
}

fn import_bindings(node: Node<'_>, source: &str) -> BTreeMap<String, String> {
    let mut bindings = BTreeMap::new();
    let mut stack = vec![node];
    while let Some(item) = stack.pop() {
        match item.kind() {
            "import_specifier" | "export_specifier" => {
                if let Some(name) = item.child_by_field_name("name") {
                    let alias = item.child_by_field_name("alias").unwrap_or(name);
                    bindings.insert(text(alias, source).into(), text(name, source).into());
                }
                continue;
            }
            "namespace_import" => {
                let mut cursor = item.walk();
                if let Some(name) = item
                    .named_children(&mut cursor)
                    .find(|n| n.kind() == "identifier")
                {
                    bindings.insert(text(name, source).into(), "*".into());
                }
                continue;
            }
            "identifier" if item.parent().is_some_and(|p| p.kind() == "import_clause") => {
                bindings.insert(text(item, source).into(), "default".into());
            }
            _ => {}
        }
        let mut cursor = item.walk();
        stack.extend(item.named_children(&mut cursor));
    }
    bindings
}

fn extract_links(node: Node<'_>, parent: Option<usize>, source: &str, file: &mut RepoFile) {
    match node.kind() {
        "assignment_expression" => {
            if let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                let target = text(left, source);
                if (target.starts_with("exports.")
                    || target.starts_with("module.exports.")
                    || target == "module.exports")
                    && right.kind() == "identifier"
                {
                    let name = text(right, source);
                    file.exports.push(name.into());
                    push_edge(file, None, name, "exports", node);
                }
            }
        }
        "import_statement" | "export_statement" => {
            let reexport = node.kind() == "export_statement";
            if let Some(specifier) = node
                .child_by_field_name("source")
                .and_then(|n| literal(n, source))
            {
                file.imports.push(Import {
                    specifier: specifier.clone(),
                    bindings: import_bindings(node, source),
                    reexport,
                    dynamic: false,
                    line: node.start_position().row + 1,
                });
                push_edge(
                    file,
                    None,
                    &specifier,
                    if reexport { "reexports" } else { "imports" },
                    node,
                );
            } else if reexport {
                let bindings = import_bindings(node, source);
                file.exports.extend(bindings.values().cloned());
                file.export_bindings.extend(bindings);
                let mut cursor = node.walk();
                if node
                    .children(&mut cursor)
                    .any(|child| child.kind() == "default")
                {
                    let local = node
                        .child_by_field_name("value")
                        .or_else(|| node.child_by_field_name("declaration"))
                        .map(|value| value.child_by_field_name("name").unwrap_or(value))
                        .filter(|value| value.kind() == "identifier")
                        .map(|value| text(value, source))
                        .unwrap_or("default");
                    file.export_bindings.insert("default".into(), local.into());
                    file.exports.push(local.into());
                }
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function") {
                let target = text(function, source);
                if target == "require" || function.kind() == "import" {
                    let argument = node
                        .child_by_field_name("arguments")
                        .and_then(|n| n.named_child(0));
                    if let Some(specifier) = argument.and_then(|n| literal(n, source)) {
                        file.imports.push(Import {
                            specifier: specifier.clone(),
                            bindings: BTreeMap::new(),
                            reexport: false,
                            dynamic: target != "require",
                            line: node.start_position().row + 1,
                        });
                        push_edge(file, parent, &specifier, "imports", node);
                    } else {
                        file.unresolved.push(
                            "dynamic import/require: relationship not statically resolvable".into(),
                        );
                    }
                } else if function.kind() == "identifier"
                    || (function.kind() == "member_expression" && target.starts_with("this."))
                {
                    push_edge(file, parent, target, "calls", node);
                } else {
                    file.unresolved.push(
                        "member/computed call: relationship not statically resolvable".into(),
                    );
                }
            }
        }
        "extends_clause" | "extends_type_clause" | "implements_clause" => {
            let kind = if node.kind() == "implements_clause" {
                "implements"
            } else {
                "extends"
            };
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                push_edge(file, parent, text(child, source), kind, child);
            }
        }
        "type_identifier"
            if node.parent().is_some_and(|p| {
                p.kind() != "type_alias_declaration" && p.kind() != "interface_declaration"
            }) =>
        {
            push_edge(file, parent, text(node, source), "type_uses", node);
        }
        _ => {}
    }
}
