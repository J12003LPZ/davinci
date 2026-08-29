//! LaTeX renderer matching `vendor/pi/packages/tui/src/latex.ts`.

use std::collections::{HashMap, HashSet};

use crate::render::visible_width;

const LAYOUT_MARKER_START: char = '\u{f0000}';
const LAYOUT_MARKER_END: char = '\u{f0001}';
const PROTECTED_SPACE: char = '\u{f0002}';
const NAMED_OPERATOR_START: char = '\u{f0004}';
const NAMED_OPERATOR_END: char = '\u{f0005}';
const NEGATIVE_SPACE: char = '\u{0000}';

#[derive(Clone)]
enum LayoutNode {
    Fraction {
        numerator: String,
        denominator: String,
    },
    Operator {
        operator: String,
        lower: Option<String>,
        upper: Option<String>,
    },
    Matrix {
        lines: Vec<String>,
        _baseline: usize,
    },
}

pub fn latex_symbols() -> HashMap<&'static str, &'static str> {
    [
        ("alpha", "\u{3b1}"),
        ("beta", "\u{3b2}"),
        ("gamma", "\u{3b3}"),
        ("delta", "\u{3b4}"),
        ("epsilon", "\u{3f5}"),
        ("varepsilon", "\u{3b5}"),
        ("zeta", "\u{3b6}"),
        ("eta", "\u{3b7}"),
        ("theta", "\u{3b8}"),
        ("vartheta", "\u{3d1}"),
        ("iota", "\u{3b9}"),
        ("kappa", "\u{3ba}"),
        ("varkappa", "\u{3f0}"),
        ("lambda", "\u{3bb}"),
        ("mu", "\u{3bc}"),
        ("nu", "\u{3bd}"),
        ("xi", "\u{3be}"),
        ("pi", "\u{3c0}"),
        ("varpi", "\u{3d6}"),
        ("rho", "\u{3c1}"),
        ("varrho", "\u{3f1}"),
        ("sigma", "\u{3c3}"),
        ("varsigma", "\u{3c2}"),
        ("tau", "\u{3c4}"),
        ("upsilon", "\u{3c5}"),
        ("phi", "\u{3d5}"),
        ("varphi", "\u{3c6}"),
        ("chi", "\u{3c7}"),
        ("psi", "\u{3c8}"),
        ("omega", "\u{3c9}"),
        ("Gamma", "\u{393}"),
        ("Delta", "\u{394}"),
        ("Theta", "\u{398}"),
        ("Lambda", "\u{39b}"),
        ("Xi", "\u{39e}"),
        ("Pi", "\u{3a0}"),
        ("Sigma", "\u{3a3}"),
        ("Upsilon", "\u{3a5}"),
        ("Phi", "\u{3a6}"),
        ("Psi", "\u{3a8}"),
        ("Omega", "\u{3a9}"),
        ("pm", "\u{b1}"),
        ("mp", "\u{2213}"),
        ("times", "\u{d7}"),
        ("div", "\u{f7}"),
        ("cdot", "\u{b7}"),
        ("ast", "\u{2217}"),
        ("star", "\u{22c6}"),
        ("circ", "\u{2218}"),
        ("bullet", "\u{2022}"),
        ("oplus", "\u{2295}"),
        ("ominus", "\u{2296}"),
        ("otimes", "\u{2297}"),
        ("oslash", "\u{2298}"),
        ("odot", "\u{2299}"),
        ("bigcirc", "\u{25cb}"),
        ("dagger", "\u{2020}"),
        ("ddagger", "\u{2021}"),
        ("amalg", "\u{2a3f}"),
        ("uplus", "\u{228e}"),
        ("sqcap", "\u{2293}"),
        ("sqcup", "\u{2294}"),
        ("triangleleft", "\u{25c1}"),
        ("triangleright", "\u{25b7}"),
        ("wr", "\u{2240}"),
        ("cap", "\u{2229}"),
        ("cup", "\u{222a}"),
        ("bigcap", "\u{22c2}"),
        ("bigcup", "\u{22c3}"),
        ("bigwedge", "\u{22c0}"),
        ("bigvee", "\u{22c1}"),
        ("bigsqcup", "\u{2a06}"),
        ("biguplus", "\u{2a04}"),
        ("bigoplus", "\u{2a01}"),
        ("bigotimes", "\u{2a02}"),
        ("bigodot", "\u{2a00}"),
        ("setminus", "\u{2216}"),
        ("in", "\u{2208}"),
        ("notin", "\u{2209}"),
        ("ni", "\u{220b}"),
        ("subset", "\u{2282}"),
        ("supset", "\u{2283}"),
        ("subseteq", "\u{2286}"),
        ("supseteq", "\u{2287}"),
        ("sqsubset", "\u{228f}"),
        ("sqsupset", "\u{2290}"),
        ("sqsubseteq", "\u{2291}"),
        ("sqsupseteq", "\u{2292}"),
        ("prec", "\u{227a}"),
        ("preceq", "\u{227c}"),
        ("succ", "\u{227b}"),
        ("succeq", "\u{227d}"),
        ("ll", "\u{226a}"),
        ("gg", "\u{226b}"),
        ("le", "\u{2264}"),
        ("leq", "\u{2264}"),
        ("leqslant", "\u{2264}"),
        ("ge", "\u{2265}"),
        ("geq", "\u{2265}"),
        ("geqslant", "\u{2265}"),
        ("ne", "\u{2260}"),
        ("neq", "\u{2260}"),
        ("equiv", "\u{2261}"),
        ("approx", "\u{2248}"),
        ("sim", "\u{223c}"),
        ("simeq", "\u{2243}"),
        ("cong", "\u{2245}"),
        ("asymp", "\u{224d}"),
        ("doteq", "\u{2250}"),
        ("propto", "\u{221d}"),
        ("parallel", "\u{2225}"),
        ("perp", "\u{22a5}"),
        ("mid", "\u{2223}"),
        ("vdash", "\u{22a2}"),
        ("dashv", "\u{22a3}"),
        ("models", "\u{22a8}"),
        ("Vdash", "\u{22a9}"),
        ("Vvdash", "\u{22aa}"),
        ("nvdash", "\u{22ac}"),
        ("nvDash", "\u{22ad}"),
        ("forall", "\u{2200}"),
        ("exists", "\u{2203}"),
        ("nexists", "\u{2204}"),
        ("neg", "\u{ac}"),
        ("land", "\u{2227}"),
        ("wedge", "\u{2227}"),
        ("lor", "\u{2228}"),
        ("vee", "\u{2228}"),
        ("to", "\u{2192}"),
        ("rightarrow", "\u{2192}"),
        ("longrightarrow", "\u{2192}"),
        ("leftarrow", "\u{2190}"),
        ("longleftarrow", "\u{2190}"),
        ("gets", "\u{2190}"),
        ("leftrightarrow", "\u{2194}"),
        ("longleftrightarrow", "\u{2194}"),
        ("hookleftarrow", "\u{21a9}"),
        ("hookrightarrow", "\u{21aa}"),
        ("twoheadleftarrow", "\u{219e}"),
        ("twoheadrightarrow", "\u{21a0}"),
        ("leftharpoonup", "\u{21bc}"),
        ("leftharpoondown", "\u{21bd}"),
        ("rightharpoonup", "\u{21c0}"),
        ("rightharpoondown", "\u{21c1}"),
        ("rightleftharpoons", "\u{21cc}"),
        ("leftrightharpoons", "\u{21cb}"),
        ("nearrow", "\u{2197}"),
        ("searrow", "\u{2198}"),
        ("swarrow", "\u{2199}"),
        ("nwarrow", "\u{2196}"),
        ("rightsquigarrow", "\u{21dd}"),
        ("leadsto", "\u{21dd}"),
        ("Rightarrow", "\u{21d2}"),
        ("Longrightarrow", "\u{21d2}"),
        ("Leftarrow", "\u{21d0}"),
        ("Longleftarrow", "\u{21d0}"),
        ("Leftrightarrow", "\u{21d4}"),
        ("Longleftrightarrow", "\u{21d4}"),
        ("implies", "\u{21d2}"),
        ("iff", "\u{21d4}"),
        ("mapsto", "\u{21a6}"),
        ("longmapsto", "\u{21a6}"),
        ("uparrow", "\u{2191}"),
        ("downarrow", "\u{2193}"),
        ("partial", "\u{2202}"),
        ("nabla", "\u{2207}"),
        ("int", "\u{222b}"),
        ("iint", "\u{222c}"),
        ("iiint", "\u{222d}"),
        ("oint", "\u{222e}"),
        ("sum", "\u{2211}"),
        ("prod", "\u{220f}"),
        ("coprod", "\u{2210}"),
        ("infty", "\u{221e}"),
        ("emptyset", "\u{2205}"),
        ("varnothing", "\u{2205}"),
        ("angle", "\u{2220}"),
        ("therefore", "\u{2234}"),
        ("because", "\u{2235}"),
        ("aleph", "\u{2135}"),
        ("beth", "\u{2136}"),
        ("gimel", "\u{2137}"),
        ("daleth", "\u{2138}"),
        ("top", "\u{22a4}"),
        ("bot", "\u{22a5}"),
        ("triangle", "\u{25b3}"),
        ("square", "\u{25a1}"),
        ("lozenge", "\u{25ca}"),
        ("checkmark", "\u{2713}"),
        ("complement", "\u{2201}"),
        ("wp", "\u{2118}"),
        ("prime", "\u{2032}"),
        ("ldots", "\u{2026}"),
        ("dots", "\u{2026}"),
        ("cdots", "\u{22ef}"),
        ("vdots", "\u{22ee}"),
        ("ddots", "\u{22f1}"),
        ("ell", "\u{2113}"),
        ("hbar", "\u{210f}"),
        ("Im", "\u{2111}"),
        ("Re", "\u{211c}"),
        ("langle", "\u{27e8}"),
        ("rangle", "\u{27e9}"),
        ("vert", "|"),
        ("lvert", "|"),
        ("rvert", "|"),
        ("Vert", "\u{2016}"),
        ("lVert", "\u{2016}"),
        ("rVert", "\u{2016}"),
        ("lbrace", "{"),
        ("rbrace", "}"),
        ("backslash", "\\\\"),
        ("lfloor", "\u{230a}"),
        ("rfloor", "\u{230b}"),
        ("lceil", "\u{2308}"),
        ("rceil", "\u{2309}"),
        ("colon", ":"),
    ]
    .into_iter()
    .collect()
}

fn superscripts() -> HashMap<char, &'static str> {
    [
        ('0', "\u{2070}"),
        ('1', "\u{b9}"),
        ('2', "\u{b2}"),
        ('3', "\u{b3}"),
        ('4', "\u{2074}"),
        ('5', "\u{2075}"),
        ('6', "\u{2076}"),
        ('7', "\u{2077}"),
        ('8', "\u{2078}"),
        ('9', "\u{2079}"),
        ('+', "\u{207a}"),
        ('-', "\u{207b}"),
        ('=', "\u{207c}"),
        ('(', "\u{207d}"),
        (')', "\u{207e}"),
        ('a', "\u{1d43}"),
        ('b', "\u{1d47}"),
        ('c', "\u{1d9c}"),
        ('d', "\u{1d48}"),
        ('e', "\u{1d49}"),
        ('f', "\u{1da0}"),
        ('g', "\u{1d4d}"),
        ('h', "\u{2b0}"),
        ('i', "\u{2071}"),
        ('j', "\u{2b2}"),
        ('k', "\u{1d4f}"),
        ('l', "\u{2e1}"),
        ('m', "\u{1d50}"),
        ('n', "\u{207f}"),
        ('o', "\u{1d52}"),
        ('p', "\u{1d56}"),
        ('r', "\u{2b3}"),
        ('s', "\u{2e2}"),
        ('t', "\u{1d57}"),
        ('u', "\u{1d58}"),
        ('v', "\u{1d5b}"),
        ('w', "\u{2b7}"),
        ('x', "\u{2e3}"),
        ('y', "\u{2b8}"),
        ('z', "\u{1dbb}"),
    ]
    .into_iter()
    .collect()
}

fn subscripts() -> HashMap<char, &'static str> {
    [
        ('0', "\u{2080}"),
        ('1', "\u{2081}"),
        ('2', "\u{2082}"),
        ('3', "\u{2083}"),
        ('4', "\u{2084}"),
        ('5', "\u{2085}"),
        ('6', "\u{2086}"),
        ('7', "\u{2087}"),
        ('8', "\u{2088}"),
        ('9', "\u{2089}"),
        ('+', "\u{208a}"),
        ('-', "\u{208b}"),
        ('=', "\u{208c}"),
        ('(', "\u{208d}"),
        (')', "\u{208e}"),
        ('a', "\u{2090}"),
        ('e', "\u{2091}"),
        ('h', "\u{2095}"),
        ('i', "\u{1d62}"),
        ('j', "\u{2c7c}"),
        ('k', "\u{2096}"),
        ('l', "\u{2097}"),
        ('m', "\u{2098}"),
        ('n', "\u{2099}"),
        ('o', "\u{2092}"),
        ('p', "\u{209a}"),
        ('r', "\u{1d63}"),
        ('s', "\u{209b}"),
        ('t', "\u{209c}"),
        ('u', "\u{1d64}"),
        ('v', "\u{1d65}"),
        ('x', "\u{2093}"),
    ]
    .into_iter()
    .collect()
}

fn negated_symbols() -> HashMap<&'static str, &'static str> {
    [
        ("<", "\u{226e}"),
        (">", "\u{226f}"),
        ("=", "\u{2260}"),
        ("∈", "\u{2209}"),
        ("∋", "\u{220c}"),
        ("∣", "\u{2224}"),
        ("∥", "\u{2226}"),
        ("∼", "\u{2241}"),
        ("≃", "\u{2244}"),
        ("≅", "\u{2247}"),
        ("≈", "\u{2249}"),
        ("≡", "\u{2262}"),
        ("≤", "\u{2270}"),
        ("≥", "\u{2271}"),
        ("≺", "\u{2280}"),
        ("≻", "\u{2281}"),
        ("⊂", "\u{2284}"),
        ("⊃", "\u{2285}"),
        ("⊆", "\u{2288}"),
        ("⊇", "\u{2289}"),
        ("⊢", "\u{22ac}"),
        ("⊨", "\u{22ad}"),
        ("↔", "\u{21ae}"),
        ("←", "\u{219a}"),
        ("→", "\u{219b}"),
        ("⇒", "\u{21cf}"),
        ("⇐", "\u{21cd}"),
        ("⇔", "\u{21ce}"),
        ("≼", "\u{22e0}"),
        ("≽", "\u{22e1}"),
    ]
    .into_iter()
    .collect()
}

fn blackboard() -> HashMap<char, &'static str> {
    [
        ('C', "\u{2102}"),
        ('H', "\u{210d}"),
        ('N', "\u{2115}"),
        ('P', "\u{2119}"),
        ('Q', "\u{211a}"),
        ('R', "\u{211d}"),
        ('Z', "\u{2124}"),
    ]
    .into_iter()
    .collect()
}

fn accents() -> HashMap<&'static str, &'static str> {
    [
        ("acute", "\u{301}"),
        ("bar", "\u{305}"),
        ("breve", "\u{306}"),
        ("check", "\u{30c}"),
        ("ddot", "\u{308}"),
        ("dot", "\u{307}"),
        ("grave", "\u{300}"),
        ("hat", "\u{302}"),
        ("mathring", "\u{30a}"),
        ("overleftarrow", "\u{20d6}"),
        ("overleftrightarrow", "\u{20e1}"),
        ("overline", "\u{305}"),
        ("overrightarrow", "\u{20d7}"),
        ("tilde", "\u{303}"),
        ("underline", "\u{332}"),
        ("vec", "\u{20d7}"),
        ("widehat", "\u{302}"),
        ("widetilde", "\u{303}"),
    ]
    .into_iter()
    .collect()
}

fn named_operators() -> HashSet<&'static str> {
    [
        "arccos", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "deg", "det",
        "dim", "exp", "gcd", "hom", "inf", "ker", "lg", "lim", "liminf", "limsup", "ln", "log",
        "max", "min", "Pr", "sec", "sin", "sinh", "sup", "tan", "tanh",
    ]
    .into_iter()
    .collect()
}

fn limit_operators() -> HashSet<&'static str> {
    [
        "argmax", "argmin", "inf", "injlim", "lim", "liminf", "limsup", "max", "min", "projlim",
        "sup",
    ]
    .into_iter()
    .collect()
}

fn display_limit_symbols() -> HashSet<&'static str> {
    [
        "bigcap",
        "bigcup",
        "bigodot",
        "bigoplus",
        "bigotimes",
        "bigsqcup",
        "biguplus",
        "bigvee",
        "bigwedge",
        "coprod",
        "int",
        "iint",
        "iiint",
        "oint",
        "prod",
        "sum",
    ]
    .into_iter()
    .collect()
}

fn relation_commands() -> HashSet<&'static str> {
    [
        "Leftarrow",
        "Leftrightarrow",
        "Longleftarrow",
        "Longleftrightarrow",
        "Longrightarrow",
        "Rightarrow",
        "Vdash",
        "Vvdash",
        "approx",
        "asymp",
        "cong",
        "dashv",
        "doteq",
        "downarrow",
        "equiv",
        "ge",
        "geq",
        "geqslant",
        "gets",
        "gg",
        "hookleftarrow",
        "hookrightarrow",
        "iff",
        "implies",
        "in",
        "leadsto",
        "le",
        "leftarrow",
        "leftharpoondown",
        "leftharpoonup",
        "leftrightarrow",
        "leftrightharpoons",
        "leq",
        "leqslant",
        "ll",
        "longleftarrow",
        "longleftrightarrow",
        "longmapsto",
        "longrightarrow",
        "mapsto",
        "mid",
        "models",
        "ne",
        "nearrow",
        "neq",
        "ni",
        "notin",
        "nvdash",
        "nvDash",
        "nwarrow",
        "parallel",
        "perp",
        "prec",
        "preceq",
        "propto",
        "rightharpoondown",
        "rightharpoonup",
        "rightleftharpoons",
        "rightarrow",
        "rightsquigarrow",
        "searrow",
        "sim",
        "simeq",
        "sqsubset",
        "sqsubseteq",
        "sqsupset",
        "sqsupseteq",
        "subset",
        "subseteq",
        "succ",
        "succeq",
        "supset",
        "supseteq",
        "swarrow",
        "to",
        "triangleleft",
        "triangleright",
        "twoheadleftarrow",
        "twoheadrightarrow",
        "uparrow",
        "vdash",
    ]
    .into_iter()
    .collect()
}

fn spacing_commands() -> HashSet<&'static str> {
    [
        ",",
        ":",
        ";",
        " ",
        ">",
        "enspace",
        "enskip",
        "medspace",
        "quad",
        "qquad",
        "thickspace",
        "thinspace",
    ]
    .into_iter()
    .collect()
}

fn negative_spacing() -> HashSet<&'static str> {
    ["!", "negmedspace", "negthickspace", "negthinspace"]
        .into_iter()
        .collect()
}

fn ignored_commands() -> HashSet<&'static str> {
    [
        "displaystyle",
        "limits",
        "nolimits",
        "scriptstyle",
        "scriptscriptstyle",
        "textstyle",
    ]
    .into_iter()
    .collect()
}

fn size_commands() -> HashSet<&'static str> {
    [
        "big", "Big", "bigg", "Bigg", "bigl", "Bigl", "biggl", "Biggl", "bigr", "Bigr", "biggr",
        "Biggr",
    ]
    .into_iter()
    .collect()
}

fn plain_wrappers() -> HashSet<&'static str> {
    [
        "emph",
        "mathcal",
        "mathbf",
        "mathfrak",
        "mathit",
        "mathrm",
        "mathnormal",
        "mathscr",
        "mathsf",
        "mathtt",
        "mathup",
        "mbox",
        "overbrace",
        "pmb",
        "smash",
        "substack",
        "text",
        "textbf",
        "textit",
        "textmd",
        "textnormal",
        "textrm",
        "textsc",
        "textsf",
        "textsl",
        "texttt",
        "textup",
        "underbrace",
        "bm",
        "boldsymbol",
    ]
    .into_iter()
    .collect()
}

fn replace_characters(value: &str, map: &HashMap<char, &'static str>) -> Option<String> {
    let mut out = String::new();
    for ch in value.chars() {
        out.push_str(map.get(&ch)?);
    }
    Some(out)
}

fn compact_script_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut compact = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && matches!(chars[j], '=' | '+' | '-') {
                compact.push(chars[j]);
                i = j + 1;
                continue;
            }
            if i > 0 && matches!(chars[i - 1], '=' | '+' | '-') {
                i = j;
                continue;
            }
        }
        compact.push(chars[i]);
        i += 1;
    }
    compact
}

fn format_script(value: &str, kind: &str) -> String {
    let value = value.trim();
    let compact = compact_script_value(value);
    let map = if kind == "sub" {
        subscripts()
    } else {
        superscripts()
    };
    if let Some(unicode) = replace_characters(&compact, &map) {
        return unicode;
    }
    let prefix = if kind == "sub" { "_" } else { "^" };
    let count = value.chars().count();
    if count == 1 || (kind == "sub" && value.chars().all(|c| c.is_ascii_alphabetic())) {
        return format!("{prefix}{value}");
    }
    format!("{prefix}({value})")
}

fn format_fraction(numerator: &str, denominator: &str) -> String {
    let numerator = numerator.trim();
    let denominator = denominator.trim();
    let simple_num =
        !numerator.is_empty() && numerator.chars().all(|c| c.is_alphanumeric() || c == '.');
    let simple_den = (!denominator.is_empty()
        && denominator.chars().all(|c| c.is_ascii_digit() || c == '.'))
        || denominator.chars().count() == 1;
    format!(
        "{}/{}",
        if simple_num {
            numerator.to_string()
        } else {
            format!("({numerator})")
        },
        if simple_den {
            denominator.to_string()
        } else {
            format!("({denominator})")
        }
    )
}

fn format_root(value: &str, symbol: &str) -> String {
    let value = value.trim();
    if !value.is_empty() && value.chars().all(|c| c.is_alphanumeric() || c == '.') {
        format!("{symbol}{value}")
    } else {
        format!("{symbol}({value})")
    }
}

fn normalize_output(value: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == NAMED_OPERATOR_START {
            if i > 0 {
                let prev = chars[i - 1];
                if prev.is_alphanumeric() || matches!(prev, ')' | ']' | '}' | LAYOUT_MARKER_END) {
                    out.push(' ');
                }
            }
            i += 1;
            continue;
        }
        if ch == NAMED_OPERATOR_END {
            if i + 1 < chars.len() {
                let next = chars[i + 1];
                if next.is_alphanumeric() || next == '\u{221a}' || next == LAYOUT_MARKER_START {
                    out.push(' ');
                }
            }
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out.lines()
        .map(|line| {
            let mut collapsed = String::new();
            let mut prev_space = false;
            for ch in line.chars() {
                if ch == ' ' || ch == '\t' {
                    if !prev_space {
                        collapsed.push(' ');
                    }
                    prev_space = true;
                } else {
                    collapsed.push(ch);
                    prev_space = false;
                }
            }
            collapsed.trim().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Parser {
    source: Vec<char>,
    position: usize,
    supported: bool,
    display: bool,
    stack_fractions: bool,
    layout_nodes: Vec<LayoutNode>,
}

impl Parser {
    fn new(source: &str, display: bool) -> Self {
        Self {
            source: source.chars().collect(),
            position: 0,
            supported: true,
            display,
            stack_fractions: true,
            layout_nodes: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.position).copied()
    }

    fn rest(&self) -> String {
        self.source[self.position..].iter().collect()
    }

    fn render(&mut self) -> Option<String> {
        let rendered = self.parse_sequence(None);
        if !self.supported || self.position != self.source.len() {
            return None;
        }
        Some(normalize_output(&rendered))
    }

    fn parse_sequence(&mut self, end: Option<char>) -> String {
        let mut result = String::new();
        while self.position < self.source.len() {
            let character = self.source[self.position];
            if end == Some(character) {
                self.position += 1;
                return result;
            }
            if character == '}' {
                self.supported = false;
                return result;
            }
            if character == '{' {
                self.position += 1;
                result.push_str(&self.parse_sequence(Some('}')));
                continue;
            }
            if character == '\\' {
                let command = self.parse_command();
                if command == NEGATIVE_SPACE.to_string() {
                    result = result.trim_end().to_string();
                    if result.ends_with(NAMED_OPERATOR_END) {
                        result.pop();
                    }
                } else {
                    result.push_str(&command);
                }
                continue;
            }
            if character == '^' || character == '_' {
                self.position += 1;
                result = result.trim_end().to_string();
                let script = format_script(
                    &self.parse_required_argument(false),
                    if character == '_' { "sub" } else { "sup" },
                );
                if result.ends_with(NAMED_OPERATOR_END) {
                    result.pop();
                    result.push_str(&script);
                    result.push(NAMED_OPERATOR_END);
                } else {
                    result.push_str(&script);
                }
                continue;
            }
            if character.is_whitespace() {
                result.push_str(&self.parse_whitespace());
                continue;
            }
            if matches!(character, '=' | '<' | '>') {
                result = format!("{} {character} ", result.trim_end());
                self.position += 1;
                continue;
            }
            if character == '&' {
                self.position += 1;
                continue;
            }
            if character == '~' {
                self.position += 1;
                result.push(' ');
                continue;
            }
            result.push(character);
            self.position += 1;
        }
        if end.is_some() {
            self.supported = false;
        }
        result
    }

    fn parse_whitespace(&mut self) -> String {
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.position += 1;
        }
        " ".into()
    }

    fn parse_command(&mut self) -> String {
        self.position += 1;
        if self.position >= self.source.len() {
            self.supported = false;
            return String::new();
        }
        let first = self.source[self.position];
        if first == '\n' || first == '\r' {
            self.position += 1;
            if first == '\r' && self.peek() == Some('\n') {
                self.position += 1;
            }
            return " ".into();
        }
        let command = if first.is_ascii_alphabetic() {
            let start = self.position;
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.position += 1;
            }
            self.source[start..self.position].iter().collect::<String>()
        } else {
            self.position += 1;
            first.to_string()
        };
        if command == "\\" {
            return "\n".into();
        }
        if spacing_commands().contains(command.as_str()) {
            return " ".into();
        }
        if negative_spacing().contains(command.as_str()) {
            return NEGATIVE_SPACE.to_string();
        }
        if ignored_commands().contains(command.as_str()) {
            return String::new();
        }
        if matches!(command.as_str(), "{" | "}" | "$" | "%" | "#" | "_" | "&") {
            return command;
        }
        if command == "|" {
            return "\u{2016}".into();
        }
        if command == "not" {
            let value = self.parse_required_argument(false);
            let value = value.trim();
            if let Some(negated) = negated_symbols().get(value) {
                return format!(" {negated} ");
            }
            let mut chars = value.chars();
            let Some(first) = chars.next() else {
                self.supported = false;
                return String::new();
            };
            return format!(" {first}\u{0338}{} ", chars.collect::<String>());
        }
        if limit_operators().contains(command.as_str()) {
            return self.parse_operator(&command, true, true, true);
        }
        if let Some(symbol) = latex_symbols().get(command.as_str()) {
            if display_limit_symbols().contains(command.as_str()) {
                return self.parse_operator(symbol, false, true, false);
            }
            if command == "cdot"
                || command == "times"
                || relation_commands().contains(command.as_str())
            {
                return format!(" {symbol} ");
            }
            return (*symbol).to_string();
        }
        if named_operators().contains(command.as_str()) {
            return format!("{NAMED_OPERATOR_START}{command}{NAMED_OPERATOR_END}");
        }
        if size_commands().contains(command.as_str()) {
            return String::new();
        }
        if matches!(command.as_str(), "left" | "middle" | "right") {
            if self.peek() == Some('.') {
                self.position += 1;
            }
            return String::new();
        }
        if matches!(command.as_str(), "frac" | "dfrac" | "tfrac") {
            let should_stack = self.display && self.stack_fractions && command != "tfrac";
            let numerator = self.parse_required_argument(!should_stack);
            let denominator = self.parse_required_argument(!should_stack);
            if should_stack {
                let index = self.layout_nodes.len();
                self.layout_nodes.push(LayoutNode::Fraction {
                    numerator: normalize_output(&numerator),
                    denominator: normalize_output(&denominator),
                });
                return format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}");
            }
            return format_fraction(&numerator, &denominator);
        }
        if command == "sqrt" {
            let degree = self.parse_optional_argument().map(|v| v.trim().to_string());
            let value = self.parse_required_argument(true);
            return match degree.as_deref() {
                None | Some("2") => format_root(&value, "\u{221a}"),
                Some("3") => format_root(&value, "\u{221b}"),
                Some("4") => format_root(&value, "\u{221c}"),
                Some(degree) => {
                    format!(
                        "{}{}",
                        format_script(degree, "sup"),
                        format_root(&value, "\u{221a}")
                    )
                }
            };
        }
        if command == "boxed" || command == "fbox" {
            return format!("[{}]", self.parse_required_argument(true).trim());
        }
        if matches!(command.as_str(), "binom" | "dbinom" | "tbinom") {
            return format!(
                "({} choose {})",
                self.parse_required_argument(true),
                self.parse_required_argument(true)
            );
        }
        if let Some(accent) = accents().get(command.as_str()) {
            let value = self.parse_required_argument(true);
            return if value.chars().count() == 1 {
                format!("{value}{accent}")
            } else {
                format!("{command}({value})")
            };
        }
        if command == "mathbb" {
            let value = self.parse_required_argument(true);
            let map = blackboard();
            return value
                .chars()
                .map(|c| {
                    map.get(&c)
                        .copied()
                        .map(str::to_string)
                        .unwrap_or_else(|| c.to_string())
                })
                .collect();
        }
        if command == "operatorname" {
            let starred = self.peek() == Some('*');
            if starred {
                self.position += 1;
            }
            let operator = normalize_output(&self.parse_required_argument(true));
            return self.parse_operator(operator.trim(), true, starred, true);
        }
        if command == "mod" || command == "bmod" {
            return " mod ".into();
        }
        if command == "pmod" || command == "pod" {
            let value = self.parse_required_argument(true);
            let value = value.trim();
            return if command == "pmod" {
                format!(" (mod {value})")
            } else {
                format!(" ({value})")
            };
        }
        if command == "overset" || command == "stackrel" {
            let upper = self.parse_required_argument(true);
            let value = self.parse_required_argument(true);
            return format!("{}{}", value.trim(), format_script(&upper, "sup"));
        }
        if command == "underset" {
            let lower = self.parse_required_argument(true);
            let value = self.parse_required_argument(true);
            return format!("{}{}", value.trim(), format_script(&lower, "sub"));
        }
        if plain_wrappers().contains(command.as_str()) {
            let value = self.parse_required_argument(true);
            return if command.starts_with("text") || command == "mbox" {
                value
            } else {
                value.trim().to_string()
            };
        }
        if command == "begin" {
            return self.parse_environment();
        }
        if command == "end" {
            self.supported = false;
            return String::new();
        }
        self.supported = false;
        format!("\\{command}")
    }

    fn parse_operator(
        &mut self,
        operator: &str,
        bracket_lower: bool,
        display_limits: bool,
        spaced: bool,
    ) -> String {
        let mut use_display = display_limits;
        let mut modifier_pos = self.position;
        while modifier_pos < self.source.len() && matches!(self.source[modifier_pos], ' ' | '\t') {
            modifier_pos += 1;
        }
        let rest: String = self.source[modifier_pos..].iter().collect();
        if rest.starts_with("\\limits") && !rest[7..].starts_with(|c: char| c.is_ascii_alphabetic())
        {
            use_display = true;
            self.position = modifier_pos + 7;
        } else if rest.starts_with("\\nolimits")
            && !rest[9..].starts_with(|c: char| c.is_ascii_alphabetic())
        {
            use_display = false;
            self.position = modifier_pos + 9;
        }
        let mut lower = None;
        let mut upper = None;
        loop {
            let mut script_pos = self.position;
            while script_pos < self.source.len() && matches!(self.source[script_pos], ' ' | '\t') {
                script_pos += 1;
            }
            let kind = self.source.get(script_pos).copied();
            if kind != Some('_') && kind != Some('^') {
                break;
            }
            self.position = script_pos + 1;
            let value = normalize_output(&self.parse_required_argument(false)).replace(' ', "");
            if kind == Some('_') {
                if lower.is_some() {
                    self.supported = false;
                }
                lower = Some(value);
            } else {
                if upper.is_some() {
                    self.supported = false;
                }
                upper = Some(value);
            }
        }
        if self.display && use_display && (lower.is_some() || upper.is_some()) {
            let index = self.layout_nodes.len();
            self.layout_nodes.push(LayoutNode::Operator {
                operator: operator.to_string(),
                lower,
                upper,
            });
            return format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}");
        }
        let mut rendered = operator.to_string();
        if let Some(lower) = &lower {
            if bracket_lower {
                rendered.push_str(&format!("[{lower}]"));
            } else {
                rendered.push_str(&format_script(lower, "sub"));
            }
        }
        if let Some(upper) = &upper {
            rendered.push_str(&format_script(upper, "sup"));
        }
        if spaced {
            format!(" {rendered} ")
        } else {
            rendered
        }
    }

    fn parse_required_argument(&mut self, stack_fractions: bool) -> String {
        let previous = self.stack_fractions;
        self.stack_fractions = previous && stack_fractions;
        let value = self.parse_required_argument_value();
        self.stack_fractions = previous;
        value
    }

    fn parse_required_argument_value(&mut self) -> String {
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.position += 1;
        }
        if self.position >= self.source.len() {
            self.supported = false;
            return String::new();
        }
        if self.peek() == Some('{') {
            self.position += 1;
            return self.parse_sequence(Some('}'));
        }
        if self.peek() == Some('\\') {
            return self.parse_command();
        }
        let value = self.peek().unwrap_or_default();
        self.position += 1;
        value.to_string()
    }

    fn parse_optional_argument(&mut self) -> Option<String> {
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.position += 1;
        }
        if self.peek() != Some('[') {
            return None;
        }
        let rest = self.rest();
        let end = rest.find(']')?;
        let value: String = rest.chars().skip(1).take(end.saturating_sub(1)).collect();
        self.position += end + 1;
        Some(self.render_nested(&value))
    }

    fn read_raw_group(&mut self) -> Option<String> {
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.position += 1;
        }
        if self.peek() != Some('{') {
            self.supported = false;
            return None;
        }
        self.position += 1;
        let start = self.position;
        let mut depth = 1;
        while self.position < self.source.len() {
            let character = self.source[self.position];
            if character == '\\' {
                self.position += 2;
                continue;
            }
            if character == '{' {
                depth += 1;
            }
            if character == '}' {
                depth -= 1;
            }
            if depth == 0 {
                let value: String = self.source[start..self.position].iter().collect();
                self.position += 1;
                return Some(value);
            }
            self.position += 1;
        }
        self.supported = false;
        None
    }

    fn render_nested(&self, source: &str) -> String {
        let mut nested = Parser::new(source, self.display);
        nested.stack_fractions = self.stack_fractions;
        nested.parse_sequence(None)
    }

    fn split_environment_rows(body: &str) -> Vec<String> {
        let mut rows = Vec::new();
        let mut current = String::new();
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                rows.push(std::mem::take(&mut current));
                i += 2;
                if i < chars.len() && chars[i] == '[' {
                    if let Some(end) = chars[i..].iter().position(|c| *c == ']') {
                        i += end + 1;
                    }
                }
                continue;
            }
            current.push(chars[i]);
            i += 1;
        }
        rows.push(current);
        rows
    }

    fn parse_environment(&mut self) -> String {
        let Some(environment) = self.read_raw_group() else {
            return String::new();
        };
        let end_marker = format!("\\end{{{environment}}}");
        let rest = self.rest();
        let Some(end) = rest.find(&end_marker) else {
            self.supported = false;
            return String::new();
        };
        let body: String = rest.chars().take(end).collect();
        self.position += end + end_marker.chars().count();
        if matches!(
            environment.as_str(),
            "equation" | "equation*" | "displaymath"
        ) {
            return self.render_nested(&body).trim().to_string();
        }
        if matches!(
            environment.as_str(),
            "aligned"
                | "align"
                | "align*"
                | "alignedat"
                | "alignat"
                | "alignat*"
                | "gather"
                | "gathered"
                | "multline"
                | "multline*"
                | "split"
        ) {
            let aligned_at = matches!(environment.as_str(), "alignedat" | "alignat" | "alignat*");
            let aligned_body = if aligned_at {
                strip_leading_group(&body)
            } else {
                body.clone()
            };
            return Self::split_environment_rows(&aligned_body)
                .into_iter()
                .map(|row| {
                    let cells: Vec<&str> = row.split('&').collect();
                    let source = if aligned_at {
                        cells
                            .chunks(2)
                            .map(|pair| pair.join(""))
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        cells.join("")
                    };
                    self.render_nested(&source).trim().to_string()
                })
                .filter(|row| !row.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
        }
        if environment == "cases" || environment == "cases*" {
            let rows: Vec<Vec<String>> = Self::split_environment_rows(&body)
                .into_iter()
                .map(|row| {
                    row.split('&')
                        .map(|cell| self.render_nested(cell).trim().to_string())
                        .collect()
                })
                .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
                .collect();
            return rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let mut value = row.first().cloned().unwrap_or_default();
                    if value.ends_with(',') {
                        value.pop();
                    }
                    let condition = row.get(1).cloned().unwrap_or_default();
                    let delimiter = if index == 0 {
                        "\u{23a7}"
                    } else if index + 1 == rows.len() {
                        "\u{23a9}"
                    } else {
                        "\u{23a8}"
                    };
                    let lower = condition.to_ascii_lowercase();
                    let prefix = if lower.starts_with("if")
                        || lower.starts_with("when")
                        || lower.starts_with("for")
                        || lower.starts_with("otherwise")
                    {
                        " "
                    } else {
                        " if "
                    };
                    if condition.is_empty() {
                        format!("{delimiter} {value}")
                    } else {
                        format!("{delimiter} {value}{prefix}{condition}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if matches!(
            environment.as_str(),
            "array"
                | "matrix"
                | "smallmatrix"
                | "pmatrix"
                | "bmatrix"
                | "Bmatrix"
                | "vmatrix"
                | "Vmatrix"
        ) {
            let matrix_body = if environment == "array" {
                strip_leading_group(&body)
            } else {
                body
            };
            return self.render_matrix(&environment, &matrix_body);
        }
        self.supported = false;
        body
    }

    fn render_matrix(&mut self, environment: &str, body: &str) -> String {
        let matrix: Vec<Vec<String>> = Self::split_environment_rows(body)
            .into_iter()
            .map(|row| {
                row.split('&')
                    .map(|cell| self.render_nested(cell).trim().to_string())
                    .collect()
            })
            .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
            .collect();
        let column_count = matrix.iter().map(|row| row.len()).max().unwrap_or(0);
        let widths: Vec<usize> = (0..column_count)
            .map(|col| {
                matrix
                    .iter()
                    .map(|row| visible_width(row.get(col).map(String::as_str).unwrap_or("")))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let rows: Vec<String> = matrix
            .iter()
            .map(|row| {
                (0..column_count)
                    .map(|col| {
                        let cell = row.get(col).cloned().unwrap_or_default();
                        let pad = widths[col].saturating_sub(visible_width(&cell));
                        format!("{cell}{}", PROTECTED_SPACE.to_string().repeat(pad))
                    })
                    .collect::<Vec<_>>()
                    .join(" \u{2502} ")
            })
            .collect();
        let lines = if matches!(environment, "array" | "matrix" | "smallmatrix") {
            rows
        } else {
            let delim = match environment {
                "pmatrix" => (
                    "\u{239b}", "\u{239e}", "\u{239c}", "\u{239f}", "\u{239d}", "\u{23a0}",
                ),
                "bmatrix" => (
                    "\u{23a1}", "\u{23a4}", "\u{23a2}", "\u{23a5}", "\u{23a3}", "\u{23a6}",
                ),
                "Bmatrix" => (
                    "\u{23a7}", "\u{23ab}", "\u{23a8}", "\u{23ac}", "\u{23a9}", "\u{23ad}",
                ),
                "vmatrix" => (
                    "\u{2502}", "\u{2502}", "\u{2502}", "\u{2502}", "\u{2502}", "\u{2502}",
                ),
                "Vmatrix" => (
                    "\u{2551}", "\u{2551}", "\u{2551}", "\u{2551}", "\u{2551}", "\u{2551}",
                ),
                _ => {
                    self.supported = false;
                    return rows.join("\n");
                }
            };
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    let (left, right) = if index == 0 {
                        (delim.0, delim.1)
                    } else if index + 1 == rows.len() {
                        (delim.4, delim.5)
                    } else {
                        (delim.2, delim.3)
                    };
                    format!("{left} {row} {right}")
                })
                .collect()
        };
        if lines.len() <= 1 {
            return lines.first().cloned().unwrap_or_default();
        }
        let index = self.layout_nodes.len();
        self.layout_nodes.push(LayoutNode::Matrix {
            lines,
            _baseline: 0,
        });
        format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}")
    }
}

fn strip_leading_group(body: &str) -> String {
    let trimmed = body.trim_start();
    if !trimmed.starts_with('{') {
        return body.to_string();
    }
    if let Some(end) = trimmed.find('}') {
        return trimmed[end + 1..].to_string();
    }
    body.to_string()
}

fn render_layout(source: &str, nodes: &[LayoutNode]) -> Vec<String> {
    let mut lines = Vec::new();
    for source_line in source.split('\n') {
        let mut text = String::new();
        let mut rest = source_line;
        while let Some(start) = rest.find(LAYOUT_MARKER_START) {
            text.push_str(&rest[..start]);
            let after = &rest[start + LAYOUT_MARKER_START.len_utf8()..];
            if let Some(end) = after.find(LAYOUT_MARKER_END) {
                if let Ok(index) = after[..end].parse::<usize>() {
                    if let Some(node) = nodes.get(index) {
                        match node {
                            LayoutNode::Fraction {
                                numerator,
                                denominator,
                            } => text.push_str(&format_fraction(numerator, denominator)),
                            LayoutNode::Operator {
                                operator,
                                lower,
                                upper,
                            } => {
                                let mut rendered = operator.clone();
                                if let Some(lower) = lower {
                                    rendered.push_str(&format!("[{lower}]"));
                                }
                                if let Some(upper) = upper {
                                    rendered.push_str(&format_script(upper, "sup"));
                                }
                                text.push_str(&rendered);
                            }
                            LayoutNode::Matrix { lines: matrix, .. } => {
                                if !text.is_empty() {
                                    lines.push(std::mem::take(&mut text));
                                }
                                lines.extend(matrix.iter().cloned());
                            }
                        }
                    }
                }
                rest = &after[end + LAYOUT_MARKER_END.len_utf8()..];
            } else {
                break;
            }
        }
        text.push_str(rest);
        if !text.is_empty() || lines.is_empty() {
            lines.push(text);
        }
    }
    lines
}

/// Render a LaTeX math expression as terminal Unicode.
/// Returns `None` when the expression contains unsupported syntax (TS contract).
pub fn render_latex(source: &str, display: bool) -> Option<String> {
    let mut parser = Parser::new(source, display);
    let rendered = parser.render()?;
    if parser.layout_nodes.is_empty() {
        return Some(rendered.replace(PROTECTED_SPACE, " "));
    }
    let lines = render_layout(&rendered, &parser.layout_nodes);
    let indentation = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    Some(
        lines
            .into_iter()
            .map(|line| {
                line.chars()
                    .skip(indentation)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .replace(PROTECTED_SPACE, " "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ts_locked_expressions() {
        assert_eq!(
            render_latex("\\alpha + \\pi", false).as_deref(),
            Some("\u{3b1} + \u{3c0}")
        );
        assert_eq!(
            render_latex("\\mathbb{C}^3 \\to \\mathbb{C}^3", false).as_deref(),
            Some("\u{2102}\u{b3} \u{2192} \u{2102}\u{b3}")
        );
        assert_eq!(
            render_latex("F_1 = -\\frac{1}{4x^2}.", false).as_deref(),
            Some("F\u{2081} = -1/(4x\u{b2}).")
        );
        assert_eq!(
            render_latex("x_{i=0}", false).as_deref(),
            Some("x\u{1d62}\u{208c}\u{2080}")
        );
        assert_eq!(
            render_latex("x\\neq0", false).as_deref(),
            Some("x \u{2260} 0")
        );
        assert_eq!(
            render_latex("A\\to B", false).as_deref(),
            Some("A \u{2192} B")
        );
        assert_eq!(
            render_latex("\\pi\\cdot\\frac{1}{\\pi}", false).as_deref(),
            Some("\u{3c0} \u{b7} 1/\u{3c0}")
        );
        assert_eq!(
            render_latex("\\sin\\theta", false).as_deref(),
            Some("sin \u{3b8}")
        );
        assert_eq!(
            render_latex("\\sin^2 x", false).as_deref(),
            Some("sin\u{b2} x")
        );
        assert_eq!(
            render_latex("\\sqrt{\\pi}", false).as_deref(),
            Some("\u{221a}\u{3c0}")
        );
        assert_eq!(
            render_latex("\\binom{n}{k}", false).as_deref(),
            Some("(n choose k)")
        );
        assert_eq!(
            render_latex("A\\not\\subseteq B", false).as_deref(),
            Some("A \u{2288} B")
        );
        let pmatrix = render_latex("\\begin{pmatrix}1&200\\\\3000&4\\end{pmatrix}", false).unwrap();
        assert!(pmatrix.contains('1') && pmatrix.contains("3000"));
        assert!(render_latex("\\unknowncommand{x}", false).is_none());
    }
}
