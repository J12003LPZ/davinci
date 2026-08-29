//! TypeScript `packages/tui/src/latex.ts` Unicode math renderer.

use crate::diff::visible_width;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const NEGATIVE_SPACE: char = '\u{0000}';
const NAMED_OPERATOR_START: char = '\u{f0004}';
const NAMED_OPERATOR_END: char = '\u{f0005}';
const LAYOUT_MARKER_START: char = '\u{f0000}';
const LAYOUT_MARKER_END: char = '\u{f0001}';
const PROTECTED_SPACE: char = '\u{f0002}';

fn symbols() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("alpha", "α");
        m.insert("beta", "β");
        m.insert("gamma", "γ");
        m.insert("delta", "δ");
        m.insert("epsilon", "ϵ");
        m.insert("varepsilon", "ε");
        m.insert("zeta", "ζ");
        m.insert("eta", "η");
        m.insert("theta", "θ");
        m.insert("vartheta", "ϑ");
        m.insert("iota", "ι");
        m.insert("kappa", "κ");
        m.insert("varkappa", "ϰ");
        m.insert("lambda", "λ");
        m.insert("mu", "μ");
        m.insert("nu", "ν");
        m.insert("xi", "ξ");
        m.insert("pi", "π");
        m.insert("varpi", "ϖ");
        m.insert("rho", "ρ");
        m.insert("varrho", "ϱ");
        m.insert("sigma", "σ");
        m.insert("varsigma", "ς");
        m.insert("tau", "τ");
        m.insert("upsilon", "υ");
        m.insert("phi", "ϕ");
        m.insert("varphi", "φ");
        m.insert("chi", "χ");
        m.insert("psi", "ψ");
        m.insert("omega", "ω");
        m.insert("Gamma", "Γ");
        m.insert("Delta", "Δ");
        m.insert("Theta", "Θ");
        m.insert("Lambda", "Λ");
        m.insert("Xi", "Ξ");
        m.insert("Pi", "Π");
        m.insert("Sigma", "Σ");
        m.insert("Upsilon", "Υ");
        m.insert("Phi", "Φ");
        m.insert("Psi", "Ψ");
        m.insert("Omega", "Ω");
        m.insert("pm", "±");
        m.insert("mp", "∓");
        m.insert("times", "×");
        m.insert("div", "÷");
        m.insert("cdot", "·");
        m.insert("ast", "∗");
        m.insert("star", "⋆");
        m.insert("circ", "∘");
        m.insert("bullet", "•");
        m.insert("oplus", "⊕");
        m.insert("ominus", "⊖");
        m.insert("otimes", "⊗");
        m.insert("oslash", "⊘");
        m.insert("odot", "⊙");
        m.insert("bigcirc", "○");
        m.insert("dagger", "†");
        m.insert("ddagger", "‡");
        m.insert("amalg", "⨿");
        m.insert("uplus", "⊎");
        m.insert("sqcap", "⊓");
        m.insert("sqcup", "⊔");
        m.insert("triangleleft", "◁");
        m.insert("triangleright", "▷");
        m.insert("wr", "≀");
        m.insert("cap", "∩");
        m.insert("cup", "∪");
        m.insert("bigcap", "⋂");
        m.insert("bigcup", "⋃");
        m.insert("bigwedge", "⋀");
        m.insert("bigvee", "⋁");
        m.insert("bigsqcup", "⨆");
        m.insert("biguplus", "⨄");
        m.insert("bigoplus", "⨁");
        m.insert("bigotimes", "⨂");
        m.insert("bigodot", "⨀");
        m.insert("setminus", "∖");
        m.insert("in", "∈");
        m.insert("notin", "∉");
        m.insert("ni", "∋");
        m.insert("subset", "⊂");
        m.insert("supset", "⊃");
        m.insert("subseteq", "⊆");
        m.insert("supseteq", "⊇");
        m.insert("sqsubset", "⊏");
        m.insert("sqsupset", "⊐");
        m.insert("sqsubseteq", "⊑");
        m.insert("sqsupseteq", "⊒");
        m.insert("prec", "≺");
        m.insert("preceq", "≼");
        m.insert("succ", "≻");
        m.insert("succeq", "≽");
        m.insert("ll", "≪");
        m.insert("gg", "≫");
        m.insert("le", "≤");
        m.insert("leq", "≤");
        m.insert("leqslant", "≤");
        m.insert("ge", "≥");
        m.insert("geq", "≥");
        m.insert("geqslant", "≥");
        m.insert("ne", "≠");
        m.insert("neq", "≠");
        m.insert("equiv", "≡");
        m.insert("approx", "≈");
        m.insert("sim", "∼");
        m.insert("simeq", "≃");
        m.insert("cong", "≅");
        m.insert("asymp", "≍");
        m.insert("doteq", "≐");
        m.insert("propto", "∝");
        m.insert("parallel", "∥");
        m.insert("perp", "⊥");
        m.insert("mid", "∣");
        m.insert("vdash", "⊢");
        m.insert("dashv", "⊣");
        m.insert("models", "⊨");
        m.insert("Vdash", "⊩");
        m.insert("Vvdash", "⊪");
        m.insert("nvdash", "⊬");
        m.insert("nvDash", "⊭");
        m.insert("forall", "∀");
        m.insert("exists", "∃");
        m.insert("nexists", "∄");
        m.insert("neg", "¬");
        m.insert("land", "∧");
        m.insert("wedge", "∧");
        m.insert("lor", "∨");
        m.insert("vee", "∨");
        m.insert("to", "→");
        m.insert("rightarrow", "→");
        m.insert("longrightarrow", "→");
        m.insert("leftarrow", "←");
        m.insert("longleftarrow", "←");
        m.insert("gets", "←");
        m.insert("leftrightarrow", "↔");
        m.insert("longleftrightarrow", "↔");
        m.insert("hookleftarrow", "↩");
        m.insert("hookrightarrow", "↪");
        m.insert("twoheadleftarrow", "↞");
        m.insert("twoheadrightarrow", "↠");
        m.insert("leftharpoonup", "↼");
        m.insert("leftharpoondown", "↽");
        m.insert("rightharpoonup", "⇀");
        m.insert("rightharpoondown", "⇁");
        m.insert("rightleftharpoons", "⇌");
        m.insert("leftrightharpoons", "⇋");
        m.insert("nearrow", "↗");
        m.insert("searrow", "↘");
        m.insert("swarrow", "↙");
        m.insert("nwarrow", "↖");
        m.insert("rightsquigarrow", "⇝");
        m.insert("leadsto", "⇝");
        m.insert("Rightarrow", "⇒");
        m.insert("Longrightarrow", "⇒");
        m.insert("Leftarrow", "⇐");
        m.insert("Longleftarrow", "⇐");
        m.insert("Leftrightarrow", "⇔");
        m.insert("Longleftrightarrow", "⇔");
        m.insert("implies", "⇒");
        m.insert("iff", "⇔");
        m.insert("mapsto", "↦");
        m.insert("longmapsto", "↦");
        m.insert("uparrow", "↑");
        m.insert("downarrow", "↓");
        m.insert("partial", "∂");
        m.insert("nabla", "∇");
        m.insert("int", "∫");
        m.insert("iint", "∬");
        m.insert("iiint", "∭");
        m.insert("oint", "∮");
        m.insert("sum", "∑");
        m.insert("prod", "∏");
        m.insert("coprod", "∐");
        m.insert("infty", "∞");
        m.insert("emptyset", "∅");
        m.insert("varnothing", "∅");
        m.insert("angle", "∠");
        m.insert("therefore", "∴");
        m.insert("because", "∵");
        m.insert("aleph", "ℵ");
        m.insert("beth", "ℶ");
        m.insert("gimel", "ℷ");
        m.insert("daleth", "ℸ");
        m.insert("top", "⊤");
        m.insert("bot", "⊥");
        m.insert("triangle", "△");
        m.insert("square", "□");
        m.insert("lozenge", "◊");
        m.insert("checkmark", "✓");
        m.insert("complement", "∁");
        m.insert("wp", "℘");
        m.insert("prime", "′");
        m.insert("ldots", "…");
        m.insert("dots", "…");
        m.insert("cdots", "⋯");
        m.insert("vdots", "⋮");
        m.insert("ddots", "⋱");
        m.insert("ell", "ℓ");
        m.insert("hbar", "ℏ");
        m.insert("Im", "ℑ");
        m.insert("Re", "ℜ");
        m.insert("langle", "⟨");
        m.insert("rangle", "⟩");
        m.insert("vert", "|");
        m.insert("lvert", "|");
        m.insert("rvert", "|");
        m.insert("Vert", "‖");
        m.insert("lVert", "‖");
        m.insert("rVert", "‖");
        m.insert("lbrace", "{");
        m.insert("rbrace", "}");
        m.insert("backslash", "\\\\");
        m.insert("lfloor", "⌊");
        m.insert("rfloor", "⌋");
        m.insert("lceil", "⌈");
        m.insert("rceil", "⌉");
        m.insert("colon", ":");
        m
    })
}

fn negated_symbols() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("<", "≮");
        m.insert(">", "≯");
        m.insert("=", "≠");
        m.insert("∈", "∉");
        m.insert("∋", "∌");
        m.insert("∣", "∤");
        m.insert("∥", "∦");
        m.insert("∼", "≁");
        m.insert("≃", "≄");
        m.insert("≅", "≇");
        m.insert("≈", "≉");
        m.insert("≡", "≢");
        m.insert("≤", "≰");
        m.insert("≥", "≱");
        m.insert("≺", "⊀");
        m.insert("≻", "⊁");
        m.insert("⊂", "⊄");
        m.insert("⊃", "⊅");
        m.insert("⊆", "⊈");
        m.insert("⊇", "⊉");
        m.insert("⊢", "⊬");
        m.insert("⊨", "⊭");
        m.insert("↔", "↮");
        m.insert("←", "↚");
        m.insert("→", "↛");
        m.insert("⇒", "⇏");
        m.insert("⇐", "⇍");
        m.insert("⇔", "⇎");
        m.insert("≼", "⋠");
        m.insert("≽", "⋡");
        m
    })
}

fn blackboard() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("C", "ℂ");
        m.insert("H", "ℍ");
        m.insert("N", "ℕ");
        m.insert("P", "ℙ");
        m.insert("Q", "ℚ");
        m.insert("R", "ℝ");
        m.insert("Z", "ℤ");
        m
    })
}

fn superscripts() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("0", "⁰");
        m.insert("1", "¹");
        m.insert("2", "²");
        m.insert("3", "³");
        m.insert("4", "⁴");
        m.insert("5", "⁵");
        m.insert("6", "⁶");
        m.insert("7", "⁷");
        m.insert("8", "⁸");
        m.insert("9", "⁹");
        m.insert("+", "⁺");
        m.insert("-", "⁻");
        m.insert("=", "⁼");
        m.insert("(", "⁽");
        m.insert(")", "⁾");
        m.insert("a", "ᵃ");
        m.insert("b", "ᵇ");
        m.insert("c", "ᶜ");
        m.insert("d", "ᵈ");
        m.insert("e", "ᵉ");
        m.insert("f", "ᶠ");
        m.insert("g", "ᵍ");
        m.insert("h", "ʰ");
        m.insert("i", "ⁱ");
        m.insert("j", "ʲ");
        m.insert("k", "ᵏ");
        m.insert("l", "ˡ");
        m.insert("m", "ᵐ");
        m.insert("n", "ⁿ");
        m.insert("o", "ᵒ");
        m.insert("p", "ᵖ");
        m.insert("r", "ʳ");
        m.insert("s", "ˢ");
        m.insert("t", "ᵗ");
        m.insert("u", "ᵘ");
        m.insert("v", "ᵛ");
        m.insert("w", "ʷ");
        m.insert("x", "ˣ");
        m.insert("y", "ʸ");
        m.insert("z", "ᶻ");
        m
    })
}

fn subscripts() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("0", "₀");
        m.insert("1", "₁");
        m.insert("2", "₂");
        m.insert("3", "₃");
        m.insert("4", "₄");
        m.insert("5", "₅");
        m.insert("6", "₆");
        m.insert("7", "₇");
        m.insert("8", "₈");
        m.insert("9", "₉");
        m.insert("+", "₊");
        m.insert("-", "₋");
        m.insert("=", "₌");
        m.insert("(", "₍");
        m.insert(")", "₎");
        m.insert("a", "ₐ");
        m.insert("e", "ₑ");
        m.insert("h", "ₕ");
        m.insert("i", "ᵢ");
        m.insert("j", "ⱼ");
        m.insert("k", "ₖ");
        m.insert("l", "ₗ");
        m.insert("m", "ₘ");
        m.insert("n", "ₙ");
        m.insert("o", "ₒ");
        m.insert("p", "ₚ");
        m.insert("r", "ᵣ");
        m.insert("s", "ₛ");
        m.insert("t", "ₜ");
        m.insert("u", "ᵤ");
        m.insert("v", "ᵥ");
        m.insert("x", "ₓ");
        m
    })
}

fn accents() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("acute", "́");
        m.insert("bar", "̅");
        m.insert("breve", "̆");
        m.insert("check", "̌");
        m.insert("ddot", "̈");
        m.insert("dot", "̇");
        m.insert("grave", "̀");
        m.insert("hat", "̂");
        m.insert("mathring", "̊");
        m.insert("overleftarrow", "⃖");
        m.insert("overleftrightarrow", "⃡");
        m.insert("overline", "̅");
        m.insert("overrightarrow", "⃗");
        m.insert("tilde", "̃");
        m.insert("underline", "̲");
        m.insert("vec", "⃗");
        m.insert("widehat", "̂");
        m.insert("widetilde", "̃");
        m
    })
}

fn named_operators() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("arccos");
        s.insert("arcsin");
        s.insert("arctan");
        s.insert("arg");
        s.insert("cos");
        s.insert("cosh");
        s.insert("cot");
        s.insert("coth");
        s.insert("csc");
        s.insert("deg");
        s.insert("det");
        s.insert("dim");
        s.insert("exp");
        s.insert("gcd");
        s.insert("hom");
        s.insert("inf");
        s.insert("ker");
        s.insert("lg");
        s.insert("lim");
        s.insert("liminf");
        s.insert("limsup");
        s.insert("ln");
        s.insert("log");
        s.insert("max");
        s.insert("min");
        s.insert("Pr");
        s.insert("sec");
        s.insert("sin");
        s.insert("sinh");
        s.insert("sup");
        s.insert("tan");
        s.insert("tanh");
        s
    })
}

fn limit_operators() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("argmax");
        s.insert("argmin");
        s.insert("inf");
        s.insert("injlim");
        s.insert("lim");
        s.insert("liminf");
        s.insert("limsup");
        s.insert("max");
        s.insert("min");
        s.insert("projlim");
        s.insert("sup");
        s
    })
}

fn display_limit_symbols() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("bigcap");
        s.insert("bigcup");
        s.insert("bigodot");
        s.insert("bigoplus");
        s.insert("bigotimes");
        s.insert("bigsqcup");
        s.insert("biguplus");
        s.insert("bigvee");
        s.insert("bigwedge");
        s.insert("coprod");
        s.insert("int");
        s.insert("iint");
        s.insert("iiint");
        s.insert("oint");
        s.insert("prod");
        s.insert("sum");
        s
    })
}

fn relation_commands() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("Leftarrow");
        s.insert("Leftrightarrow");
        s.insert("Longleftarrow");
        s.insert("Longleftrightarrow");
        s.insert("Longrightarrow");
        s.insert("Rightarrow");
        s.insert("Vdash");
        s.insert("Vvdash");
        s.insert("approx");
        s.insert("asymp");
        s.insert("cong");
        s.insert("dashv");
        s.insert("doteq");
        s.insert("downarrow");
        s.insert("equiv");
        s.insert("ge");
        s.insert("geq");
        s.insert("geqslant");
        s.insert("gets");
        s.insert("gg");
        s.insert("hookleftarrow");
        s.insert("hookrightarrow");
        s.insert("iff");
        s.insert("implies");
        s.insert("in");
        s.insert("leadsto");
        s.insert("le");
        s.insert("leftarrow");
        s.insert("leftharpoondown");
        s.insert("leftharpoonup");
        s.insert("leftrightarrow");
        s.insert("leftrightharpoons");
        s.insert("leq");
        s.insert("leqslant");
        s.insert("ll");
        s.insert("longleftarrow");
        s.insert("longleftrightarrow");
        s.insert("longmapsto");
        s.insert("longrightarrow");
        s.insert("mapsto");
        s.insert("mid");
        s.insert("models");
        s.insert("ne");
        s.insert("nearrow");
        s.insert("neq");
        s.insert("ni");
        s.insert("notin");
        s.insert("nvdash");
        s.insert("nvDash");
        s.insert("nwarrow");
        s.insert("parallel");
        s.insert("perp");
        s.insert("prec");
        s.insert("preceq");
        s.insert("propto");
        s.insert("rightharpoondown");
        s.insert("rightharpoonup");
        s.insert("rightleftharpoons");
        s.insert("rightarrow");
        s.insert("rightsquigarrow");
        s.insert("searrow");
        s.insert("sim");
        s.insert("simeq");
        s.insert("sqsubset");
        s.insert("sqsubseteq");
        s.insert("sqsupset");
        s.insert("sqsupseteq");
        s.insert("subset");
        s.insert("subseteq");
        s.insert("succ");
        s.insert("succeq");
        s.insert("supset");
        s.insert("supseteq");
        s.insert("swarrow");
        s.insert("to");
        s.insert("triangleleft");
        s.insert("triangleright");
        s.insert("twoheadleftarrow");
        s.insert("twoheadrightarrow");
        s.insert("uparrow");
        s.insert("vdash");
        s
    })
}

fn spacing_commands() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert(",");
        s.insert(":");
        s.insert(";");
        s.insert(" ");
        s.insert(">");
        s.insert("enspace");
        s.insert("enskip");
        s.insert("medspace");
        s.insert("quad");
        s.insert("qquad");
        s.insert("thickspace");
        s.insert("thinspace");
        s
    })
}

fn negative_spacing_commands() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("!");
        s.insert("negmedspace");
        s.insert("negthickspace");
        s.insert("negthinspace");
        s
    })
}

fn ignored_commands() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("displaystyle");
        s.insert("limits");
        s.insert("nolimits");
        s.insert("scriptstyle");
        s.insert("scriptscriptstyle");
        s.insert("textstyle");
        s
    })
}

fn size_commands() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("big");
        s.insert("Big");
        s.insert("bigg");
        s.insert("Bigg");
        s.insert("bigl");
        s.insert("Bigl");
        s.insert("biggl");
        s.insert("Biggl");
        s.insert("bigr");
        s.insert("Bigr");
        s.insert("biggr");
        s.insert("Biggr");
        s
    })
}

fn plain_wrappers() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&str>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("emph");
        s.insert("mathcal");
        s.insert("mathbf");
        s.insert("mathfrak");
        s.insert("mathit");
        s.insert("mathrm");
        s.insert("mathnormal");
        s.insert("mathscr");
        s.insert("mathsf");
        s.insert("mathtt");
        s.insert("mathup");
        s.insert("mbox");
        s.insert("overbrace");
        s.insert("pmb");
        s.insert("smash");
        s.insert("substack");
        s.insert("text");
        s.insert("textbf");
        s.insert("textit");
        s.insert("textmd");
        s.insert("textnormal");
        s.insert("textrm");
        s.insert("textsc");
        s.insert("textsf");
        s.insert("textsl");
        s.insert("texttt");
        s.insert("textup");
        s.insert("underbrace");
        s.insert("bm");
        s.insert("boldsymbol");
        s
    })
}

fn is_letter(ch: char) -> bool {
    ch.is_alphabetic()
}

fn is_number(ch: char) -> bool {
    ch.is_numeric()
}

fn is_letter_or_number(ch: char) -> bool {
    is_letter(ch) || is_number(ch)
}

fn replace_characters(value: &str, replacements: &HashMap<&str, &str>) -> Option<String> {
    let mut result = String::new();
    for character in value.chars() {
        let key = character.to_string();
        result.push_str(replacements.get(key.as_str())?);
    }
    Some(result)
}

fn strip_spaces_around_ops(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && matches!(chars[j], '=' | '+' | '-') {
                let op = chars[j];
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                out.push(op);
                i = k;
                continue;
            }
        }
        if matches!(chars[i], '=' | '+' | '-') {
            let mut k = i + 1;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            out.push(chars[i]);
            i = k;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn format_script(value: &str, kind: ScriptKind) -> String {
    let value = value.trim();
    let replacements = match kind {
        ScriptKind::Sub => subscripts(),
        ScriptKind::Sup => superscripts(),
    };
    let collapsed = strip_spaces_around_ops(value);
    if let Some(unicode) = replace_characters(&collapsed, replacements) {
        return unicode;
    }
    let prefix = match kind {
        ScriptKind::Sub => "_",
        ScriptKind::Sup => "^",
    };
    let char_count = value.chars().count();
    if char_count == 1
        || (kind == ScriptKind::Sub && value.chars().all(|ch| ch.is_ascii_alphabetic()))
    {
        format!("{prefix}{value}")
    } else {
        format!("{prefix}({value})")
    }
}

fn format_fraction(numerator: &str, denominator: &str) -> String {
    let numerator = numerator.trim();
    let denominator = denominator.trim();
    let simple_numerator = !numerator.is_empty()
        && numerator
            .chars()
            .all(|ch| is_letter_or_number(ch) || ch == '.');
    let simple_denominator = (!denominator.is_empty()
        && denominator.chars().all(|ch| is_number(ch) || ch == '.'))
        || denominator.chars().count() == 1;
    let num = if simple_numerator {
        numerator.to_string()
    } else {
        format!("({numerator})")
    };
    let den = if simple_denominator {
        denominator.to_string()
    } else {
        format!("({denominator})")
    };
    format!("{num}/{den}")
}

fn format_root(value: &str, symbol: &str) -> String {
    let value = value.trim();
    if !value.is_empty() && value.chars().all(|ch| is_letter_or_number(ch) || ch == '.') {
        format!("{symbol}{value}")
    } else {
        format!("{symbol}({value})")
    }
}

fn normalize_output(value: &str) -> String {
    let mut chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == NAMED_OPERATOR_START && i > 0 {
            let prev = chars[i - 1];
            if is_letter_or_number(prev)
                || prev == ')'
                || prev == ']'
                || prev == '}'
                || prev == LAYOUT_MARKER_END
            {
                chars[i] = ' ';
            }
        }
        i += 1;
    }
    let mut cleaned = String::new();
    for ch in chars {
        if ch != NAMED_OPERATOR_START {
            cleaned.push(ch);
        }
    }
    let chars: Vec<char> = cleaned.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == NAMED_OPERATOR_END {
            let next = chars.get(i + 1).copied();
            if next.is_some_and(|n| is_letter_or_number(n) || n == '√' || n == LAYOUT_MARKER_START)
            {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    let lines: Vec<String> = out
        .split('\n')
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
        .collect();
    let last = lines.len().saturating_sub(1);
    lines
        .into_iter()
        .enumerate()
        .filter(|(index, line)| !line.is_empty() || (*index > 0 && *index < last))
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScriptKind {
    Sub,
    Sup,
}

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
        baseline: usize,
    },
}

struct Layout {
    lines: Vec<String>,
    width: usize,
    baseline: usize,
}

fn pad_layout_line(line: &str, width: usize, centered: bool) -> String {
    let padding = width.saturating_sub(visible_width(line));
    let left = if centered { padding / 2 } else { 0 };
    format!("{}{}{}", " ".repeat(left), line, " ".repeat(padding - left))
}

fn join_layouts(layouts: &[Layout]) -> Layout {
    if layouts.is_empty() {
        return Layout {
            lines: vec![String::new()],
            width: 0,
            baseline: 0,
        };
    }
    let baseline = layouts.iter().map(|l| l.baseline).max().unwrap_or(0);
    let below = layouts
        .iter()
        .map(|l| l.lines.len().saturating_sub(l.baseline).saturating_sub(1))
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for row in 0..=baseline + below {
        let mut line = String::new();
        for layout in layouts {
            let source_row = row as isize - baseline as isize + layout.baseline as isize;
            if source_row >= 0 && (source_row as usize) < layout.lines.len() {
                line.push_str(&pad_layout_line(
                    &layout.lines[source_row as usize],
                    layout.width,
                    false,
                ));
            } else {
                line.push_str(&" ".repeat(layout.width));
            }
        }
        lines.push(line.trim_end().to_string());
    }
    Layout {
        width: layouts.iter().map(|l| l.width).sum(),
        baseline,
        lines,
    }
}

fn find_markers(source: &str) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let start = LAYOUT_MARKER_START.to_string();
    let end = LAYOUT_MARKER_END.to_string();
    let mut search = 0;
    while let Some(rel) = source[search..].find(&start) {
        let index = search + rel;
        let after = index + start.len();
        if let Some(end_rel) = source[after..].find(&end) {
            let num = &source[after..after + end_rel];
            if let Ok(id) = num.parse::<usize>() {
                let total = start.len() + num.len() + end.len();
                out.push((index, id, total));
            }
            search = after + end_rel + end.len();
        } else {
            break;
        }
    }
    out
}

fn trailing_marker(result: &str) -> Option<usize> {
    let start = LAYOUT_MARKER_START.to_string();
    let end = LAYOUT_MARKER_END.to_string();
    if !result.ends_with(&end) {
        return None;
    }
    let without_end = &result[..result.len() - end.len()];
    let start_at = without_end.rfind(&start)?;
    without_end[start_at + start.len()..].parse().ok()
}

fn render_layout(source: &str, nodes: &[LayoutNode]) -> Layout {
    let mut rendered_lines = Vec::new();
    let mut first_baseline = 0;
    for source_line in source.split('\n') {
        let mut layouts = Vec::new();
        let mut position = 0;
        let mut previous_node: Option<&LayoutNode> = None;
        for (index, node_id, match_len) in find_markers(source_line) {
            let Some(node) = nodes.get(node_id) else {
                continue;
            };
            if index > position {
                let sliced = &source_line[position..index];
                let trimmed = if previous_node.is_some() {
                    sliced.trim_start().trim_end()
                } else {
                    sliced.trim_end()
                };
                let preserve_leading = matches!(previous_node, Some(LayoutNode::Matrix { .. }))
                    && sliced.starts_with(char::is_whitespace);
                let preserve_trailing = matches!(node, LayoutNode::Matrix { .. })
                    && sliced.ends_with(char::is_whitespace);
                let text = if !trimmed.is_empty() {
                    format!(
                        "{}{}{}",
                        if preserve_leading { " " } else { "" },
                        trimmed,
                        if preserve_trailing { " " } else { "" }
                    )
                } else if preserve_leading || preserve_trailing {
                    " ".to_string()
                } else {
                    String::new()
                };
                layouts.push(Layout {
                    width: visible_width(&text),
                    baseline: 0,
                    lines: vec![text],
                });
            }
            match node {
                LayoutNode::Fraction {
                    numerator,
                    denominator,
                } => {
                    let numerator = render_layout(numerator, nodes);
                    let denominator = render_layout(denominator, nodes);
                    let content_width = numerator.width.max(denominator.width).max(1);
                    let width = content_width + 2;
                    let mut lines: Vec<String> = numerator
                        .lines
                        .iter()
                        .map(|line| pad_layout_line(line, width, true))
                        .collect();
                    lines.push(format!(" {} ", "─".repeat(content_width)));
                    lines.extend(
                        denominator
                            .lines
                            .iter()
                            .map(|line| pad_layout_line(line, width, true)),
                    );
                    let baseline = numerator.lines.len();
                    layouts.push(Layout {
                        lines,
                        width,
                        baseline,
                    });
                }
                LayoutNode::Operator {
                    operator,
                    lower,
                    upper,
                } => {
                    let content_width = visible_width(operator)
                        .max(lower.as_deref().map(visible_width).unwrap_or(0))
                        .max(upper.as_deref().map(visible_width).unwrap_or(0));
                    let mut lines = Vec::new();
                    if let Some(upper) = upper {
                        lines.push(format!("{} ", pad_layout_line(upper, content_width, true)));
                    }
                    lines.push(format!(
                        "{} ",
                        pad_layout_line(operator, content_width, true)
                    ));
                    if let Some(lower) = lower {
                        lines.push(format!("{} ", pad_layout_line(lower, content_width, true)));
                    }
                    layouts.push(Layout {
                        baseline: if upper.is_some() { 1 } else { 0 },
                        width: content_width + 1,
                        lines,
                    });
                }
                LayoutNode::Matrix { lines, baseline } => {
                    let width = lines
                        .iter()
                        .map(|line| visible_width(line))
                        .max()
                        .unwrap_or(0);
                    layouts.push(Layout {
                        lines: lines
                            .iter()
                            .map(|line| pad_layout_line(line, width, false))
                            .collect(),
                        width,
                        baseline: *baseline,
                    });
                }
            }
            position = index + match_len;
            previous_node = Some(node);
        }
        if position < source_line.len() {
            let sliced = &source_line[position..];
            let trimmed = if previous_node.is_some() {
                sliced.trim_start()
            } else {
                sliced
            };
            let text = if matches!(previous_node, Some(LayoutNode::Matrix { .. }))
                && sliced.starts_with(char::is_whitespace)
            {
                format!(" {trimmed}")
            } else {
                trimmed.to_string()
            };
            layouts.push(Layout {
                width: visible_width(&text),
                baseline: 0,
                lines: vec![text],
            });
        }
        let line_layout = join_layouts(&layouts);
        if rendered_lines.is_empty() {
            first_baseline = line_layout.baseline;
        }
        rendered_lines.extend(line_layout.lines);
    }
    Layout {
        width: rendered_lines
            .iter()
            .map(|line| visible_width(line))
            .max()
            .unwrap_or(0),
        baseline: first_baseline,
        lines: rendered_lines,
    }
}

struct LatexParser<'a> {
    source: &'a str,
    layout_nodes: &'a mut Vec<LayoutNode>,
    display: bool,
    position: usize,
    supported: bool,
    stack_fractions: bool,
}

impl<'a> LatexParser<'a> {
    fn new(source: &'a str, layout_nodes: &'a mut Vec<LayoutNode>, display: bool) -> Self {
        Self {
            source,
            layout_nodes,
            display,
            position: 0,
            supported: true,
            stack_fractions: true,
        }
    }

    fn render(&mut self) -> Option<String> {
        let rendered = self.parse_sequence(None);
        if !self.supported || self.position != self.source.len() {
            return None;
        }
        Some(normalize_output(&rendered))
    }

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.position += ch.len_utf8();
        Some(ch)
    }

    fn parse_sequence(&mut self, end_character: Option<char>) -> String {
        let mut result = String::new();
        while self.position < self.source.len() {
            let Some(character) = self.peek() else {
                break;
            };
            if end_character == Some(character) {
                self.bump();
                return result;
            }
            if character == '}' {
                self.supported = false;
                return result;
            }
            if character == '{' {
                self.bump();
                result.push_str(&self.parse_sequence(Some('}')));
                continue;
            }
            if character == '\\' {
                let command = self.parse_command();
                if command == NEGATIVE_SPACE.to_string() {
                    result = result.trim_end().to_string();
                    if result.ends_with(NAMED_OPERATOR_END) {
                        result.truncate(result.len() - NAMED_OPERATOR_END.len_utf8());
                    }
                } else {
                    result.push_str(&command);
                }
                continue;
            }
            if character == '^' || character == '_' {
                self.bump();
                result = result.trim_end().to_string();
                let script = format_script(
                    &self.parse_required_argument(false),
                    if character == '_' {
                        ScriptKind::Sub
                    } else {
                        ScriptKind::Sup
                    },
                );
                if result.ends_with(NAMED_OPERATOR_END) {
                    result.truncate(result.len() - NAMED_OPERATOR_END.len_utf8());
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
            if character == '=' || character == '<' || character == '>' {
                result = format!("{} {character} ", result.trim_end());
                self.bump();
                continue;
            }
            if character == '&' {
                self.bump();
                continue;
            }
            if character == '~' {
                self.bump();
                result.push(' ');
                continue;
            }
            if character == '.' {
                if let Some(id) = trailing_marker(&result) {
                    if let Some(LayoutNode::Matrix { lines, .. }) = self.layout_nodes.get_mut(id) {
                        if let Some(last) = lines.last_mut() {
                            last.push(character);
                            self.bump();
                            continue;
                        }
                    }
                }
            }
            result.push(character);
            self.bump();
        }
        if end_character.is_some() {
            self.supported = false;
        }
        result
    }

    fn parse_whitespace(&mut self) -> String {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
        " ".to_string()
    }

    fn parse_command(&mut self) -> String {
        self.bump();
        if self.position >= self.source.len() {
            self.supported = false;
            return String::new();
        }
        let first = self.peek().unwrap_or('\0');
        if first == '\n' || first == '\r' {
            self.bump();
            if first == '\r' && self.peek() == Some('\n') {
                self.bump();
            }
            return " ".to_string();
        }
        let command = if first.is_ascii_alphabetic() {
            let start = self.position;
            while self.peek().is_some_and(|ch| ch.is_ascii_alphabetic()) {
                self.bump();
            }
            self.source[start..self.position].to_string()
        } else {
            self.bump();
            first.to_string()
        };
        if command == "\\" {
            return "\n".to_string();
        }
        if spacing_commands().contains(command.as_str()) {
            return " ".to_string();
        }
        if negative_spacing_commands().contains(command.as_str()) {
            return NEGATIVE_SPACE.to_string();
        }
        if ignored_commands().contains(command.as_str()) {
            return String::new();
        }
        if matches!(command.as_str(), "{" | "}" | "$" | "%" | "#" | "_" | "&") {
            return command;
        }
        if command == "|" {
            return "‖".to_string();
        }
        if command == "not" {
            let value = self.parse_required_argument(false);
            let value = value.trim();
            if let Some(negated) = negated_symbols().get(value) {
                return format!(" {negated} ");
            }
            let characters: Vec<char> = value.chars().collect();
            if characters.is_empty() {
                self.supported = false;
                return String::new();
            }
            return format!(
                " {}{}{} ",
                characters[0],
                '\u{0338}',
                characters[1..].iter().collect::<String>()
            );
        }
        if limit_operators().contains(command.as_str()) {
            return self.parse_operator(&command, "bracket", true, true);
        }
        if let Some(symbol) = symbols().get(command.as_str()) {
            if display_limit_symbols().contains(command.as_str()) {
                return self.parse_operator(symbol, "script", true, false);
            }
            return if command == "cdot"
                || command == "times"
                || relation_commands().contains(command.as_str())
            {
                format!(" {symbol} ")
            } else {
                (*symbol).to_string()
            };
        }
        if named_operators().contains(command.as_str()) {
            return format!("{NAMED_OPERATOR_START}{command}{NAMED_OPERATOR_END}");
        }
        if size_commands().contains(command.as_str()) {
            return String::new();
        }
        if matches!(command.as_str(), "left" | "middle" | "right") {
            if self.peek() == Some('.') {
                self.bump();
            }
            return String::new();
        }
        if matches!(command.as_str(), "frac" | "dfrac" | "tfrac") {
            let should_stack = self.display && self.stack_fractions && command != "tfrac";
            let numerator = self.parse_required_argument(!should_stack);
            let denominator = self.parse_required_argument(!should_stack);
            if should_stack {
                self.layout_nodes.push(LayoutNode::Fraction {
                    numerator: normalize_output(&numerator),
                    denominator: normalize_output(&denominator),
                });
                let index = self.layout_nodes.len() - 1;
                return format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}");
            }
            return format_fraction(&numerator, &denominator);
        }
        if command == "sqrt" {
            let degree = self.parse_optional_argument().map(|v| v.trim().to_string());
            let value = self.parse_required_argument(true);
            return match degree.as_deref() {
                None | Some("2") => format_root(&value, "√"),
                Some("3") => format_root(&value, "∛"),
                Some("4") => format_root(&value, "∜"),
                Some(degree) => format!(
                    "{}{}",
                    format_script(degree, ScriptKind::Sup),
                    format_root(&value, "√")
                ),
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
            return value
                .chars()
                .map(|ch| {
                    let key = ch.to_string();
                    blackboard()
                        .get(key.as_str())
                        .copied()
                        .unwrap_or("")
                        .to_string()
                        .chars()
                        .next()
                        .unwrap_or(ch)
                })
                .collect();
        }
        if command == "operatorname" {
            let starred = self.peek() == Some('*');
            if starred {
                self.bump();
            }
            let operator = normalize_output(&self.parse_required_argument(true))
                .trim()
                .to_string();
            return self.parse_operator(&operator, "bracket", starred, true);
        }
        if command == "mod" || command == "bmod" {
            return " mod ".to_string();
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
            return format!("{}{}", value.trim(), format_script(&upper, ScriptKind::Sup));
        }
        if command == "underset" {
            let lower = self.parse_required_argument(true);
            let value = self.parse_required_argument(true);
            return format!("{}{}", value.trim(), format_script(&lower, ScriptKind::Sub));
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
        inline_lower_style: &str,
        display_limits: bool,
        spaced: bool,
    ) -> String {
        let mut use_display_limits = display_limits;
        let mut modifier_position = self.position;
        while self.source[modifier_position..]
            .chars()
            .next()
            .is_some_and(|ch| ch == ' ' || ch == '\t')
        {
            modifier_position += 1;
        }
        let rest = &self.source[modifier_position..];
        if let Some(stripped) = rest.strip_prefix("\\limits") {
            if !stripped
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            {
                use_display_limits = true;
                self.position = modifier_position + "\\limits".len();
            }
        } else if let Some(stripped) = rest.strip_prefix("\\nolimits") {
            if !stripped
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            {
                use_display_limits = false;
                self.position = modifier_position + "\\nolimits".len();
            }
        }
        let mut lower = None;
        let mut upper = None;
        loop {
            let mut script_position = self.position;
            while self.source[script_position..]
                .chars()
                .next()
                .is_some_and(|ch| ch == ' ' || ch == '\t')
            {
                script_position += 1;
            }
            let kind = self.source[script_position..].chars().next();
            if kind != Some('_') && kind != Some('^') {
                break;
            }
            self.position = script_position + 1;
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
        if self.display && use_display_limits && (lower.is_some() || upper.is_some()) {
            self.layout_nodes.push(LayoutNode::Operator {
                operator: operator.to_string(),
                lower,
                upper,
            });
            let index = self.layout_nodes.len() - 1;
            return format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}");
        }
        let mut rendered = operator.to_string();
        if let Some(lower) = &lower {
            let lower_text = if inline_lower_style == "bracket" {
                format!("[{lower}]")
            } else {
                format_script(lower, ScriptKind::Sub)
            };
            rendered.push_str(&lower_text);
        }
        if let Some(upper) = &upper {
            rendered.push_str(&format_script(upper, ScriptKind::Sup));
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
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
        if self.position >= self.source.len() {
            self.supported = false;
            return String::new();
        }
        if self.peek() == Some('{') {
            self.bump();
            return self.parse_sequence(Some('}'));
        }
        if self.peek() == Some('\\') {
            return self.parse_command();
        }
        self.bump().unwrap_or('\0').to_string()
    }

    fn parse_optional_argument(&mut self) -> Option<String> {
        while self.peek() == Some(' ') || self.peek() == Some('\t') {
            self.bump();
        }
        if self.peek() != Some('[') {
            return None;
        }
        let rest = &self.source[self.position + 1..];
        let Some(end) = rest.find(']') else {
            self.supported = false;
            return None;
        };
        let abs_end = self.position + 1 + end;
        let value = self.source[self.position + 1..abs_end].to_string();
        self.position = abs_end + 1;
        Some(self.render_nested(&value, true))
    }

    fn read_raw_group(&mut self) -> Option<String> {
        while self.peek() == Some(' ') || self.peek() == Some('\t') {
            self.bump();
        }
        if self.peek() != Some('{') {
            self.supported = false;
            return None;
        }
        self.bump();
        let start = self.position;
        let mut depth = 1;
        while self.position < self.source.len() {
            let character = self.peek().unwrap_or('\0');
            if character == '\\' {
                self.bump();
                self.bump();
                continue;
            }
            if character == '{' {
                depth += 1;
            }
            if character == '}' {
                depth -= 1;
            }
            if depth == 0 {
                let value = self.source[start..self.position].to_string();
                self.bump();
                return Some(value);
            }
            self.bump();
        }
        self.supported = false;
        None
    }

    fn split_environment_rows(body: &str) -> Vec<String> {
        let mut rows = Vec::new();
        let mut current = String::new();
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                i += 2;
                if i < chars.len() && chars[i] == '[' {
                    if let Some(rel) = chars[i + 1..]
                        .iter()
                        .position(|ch| *ch == ']' || *ch == '\n')
                    {
                        if chars[i + 1 + rel] == ']' {
                            i = i + 2 + rel;
                        }
                    }
                }
                rows.push(std::mem::take(&mut current));
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
        let Some(end) = self.source[self.position..].find(&end_marker) else {
            self.supported = false;
            return String::new();
        };
        let body = self.source[self.position..self.position + end].to_string();
        self.position += end + end_marker.len();
        if matches!(
            environment.as_str(),
            "equation" | "equation*" | "displaymath"
        ) {
            return self.render_nested(&body, true).trim().to_string();
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
            let mut lines = Vec::new();
            for row in Self::split_environment_rows(&aligned_body) {
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
                let trimmed = self.render_nested(&source, true).trim().to_string();
                if !trimmed.is_empty() {
                    lines.push(trimmed);
                }
            }
            return lines.join("\n");
        }
        if environment == "cases" || environment == "cases*" {
            let mut rows: Vec<Vec<String>> = Vec::new();
            for row in Self::split_environment_rows(&body) {
                let cells: Vec<String> = row
                    .split('&')
                    .map(|cell| self.render_nested(cell, false).trim().to_string())
                    .collect();
                if cells.iter().any(|cell| !cell.is_empty()) {
                    rows.push(cells);
                }
            }
            return rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let value = strip_trailing_comma(row.first().map(String::as_str).unwrap_or(""));
                    let condition = row.get(1).cloned().unwrap_or_default();
                    let delimiter = if index == 0 {
                        "⎧"
                    } else if index + 1 == rows.len() {
                        "⎩"
                    } else {
                        "⎨"
                    };
                    let condition_prefix = if condition_has_natural_word(&condition) {
                        " "
                    } else {
                        " if "
                    };
                    if condition.is_empty() {
                        format!("{delimiter} {value}")
                    } else {
                        format!("{delimiter} {value}{condition_prefix}{condition}")
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
        let mut matrix: Vec<Vec<String>> = Vec::new();
        for row in Self::split_environment_rows(body) {
            let cells: Vec<String> = row
                .split('&')
                .map(|cell| self.render_nested(cell, false).trim().to_string())
                .collect();
            if cells.iter().any(|cell| !cell.is_empty()) {
                matrix.push(cells);
            }
        }
        let column_count = matrix.iter().map(|row| row.len()).max().unwrap_or(0);
        let column_widths: Vec<usize> = (0..column_count)
            .map(|column| {
                matrix
                    .iter()
                    .map(|row| visible_width(row.get(column).map(String::as_str).unwrap_or("")))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let rows: Vec<String> = matrix
            .iter()
            .map(|row| {
                (0..column_count)
                    .map(|column| {
                        let cell = row.get(column).cloned().unwrap_or_default();
                        let pad = column_widths[column].saturating_sub(visible_width(&cell));
                        format!(
                            "{cell}{}",
                            std::iter::repeat(PROTECTED_SPACE)
                                .take(pad)
                                .collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" │ ")
            })
            .collect();
        let lines = if matches!(environment, "array" | "matrix" | "smallmatrix") {
            rows
        } else {
            let delimiter = match environment {
                "pmatrix" => ["⎛", "⎞", "⎜", "⎟", "⎝", "⎠"],
                "bmatrix" => ["⎡", "⎤", "⎢", "⎥", "⎣", "⎦"],
                "Bmatrix" => ["⎧", "⎫", "⎨", "⎬", "⎩", "⎭"],
                "vmatrix" => ["│", "│", "│", "│", "│", "│"],
                "Vmatrix" => ["║", "║", "║", "║", "║", "║"],
                _ => {
                    self.supported = false;
                    return rows.join("\n");
                }
            };
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    let left = if index == 0 {
                        delimiter[0]
                    } else if index + 1 == rows.len() {
                        delimiter[4]
                    } else {
                        delimiter[2]
                    };
                    let right = if index == 0 {
                        delimiter[1]
                    } else if index + 1 == rows.len() {
                        delimiter[5]
                    } else {
                        delimiter[3]
                    };
                    format!("{left} {row} {right}")
                })
                .collect()
        };
        if lines.len() <= 1 {
            return lines.first().cloned().unwrap_or_default();
        }
        self.layout_nodes
            .push(LayoutNode::Matrix { lines, baseline: 0 });
        let index = self.layout_nodes.len() - 1;
        format!("{LAYOUT_MARKER_START}{index}{LAYOUT_MARKER_END}")
    }

    fn render_nested(&mut self, source: &str, stack_fractions: bool) -> String {
        let display = self.display && stack_fractions;
        let rendered = {
            let mut nested = LatexParser::new(source, self.layout_nodes, display);
            nested.render()
        };
        match rendered {
            Some(rendered) => rendered,
            None => {
                self.supported = false;
                source.to_string()
            }
        }
    }
}

fn strip_trailing_comma(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end > 0 && bytes[end - 1] == b',' {
        end -= 1;
        value[..end].to_string()
    } else {
        value.to_string()
    }
}

fn strip_leading_group(body: &str) -> String {
    let trimmed = body.trim_start();
    if let Some(rest) = trimmed.strip_prefix('{') {
        if let Some(end) = rest.find('}') {
            return rest[end + 1..].to_string();
        }
    }
    body.to_string()
}

fn condition_has_natural_word(condition: &str) -> bool {
    let lower = condition.to_ascii_lowercase();
    ["if", "when", "for", "otherwise"].iter().any(|word| {
        lower == *word
            || lower.starts_with(&format!("{word} "))
            || lower.starts_with(&format!("{word}\t"))
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderLatexOptions {
    pub display: bool,
}

pub fn render_latex(source: &str, options: RenderLatexOptions) -> Option<String> {
    let mut layout_nodes = Vec::new();
    let rendered = LatexParser::new(source, &mut layout_nodes, options.display).render()?;
    if layout_nodes.is_empty() {
        return Some(rendered.replace(PROTECTED_SPACE, " "));
    }
    let lines = render_layout(&rendered, &layout_nodes).lines;
    let indentation = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    Some(
        lines
            .into_iter()
            .map(|line| line.get(indentation..).unwrap_or("").trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .replace(PROTECTED_SPACE, " "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(source: &str) -> String {
        render_latex(source, RenderLatexOptions::default()).expect(source)
    }

    fn display(source: &str) -> String {
        render_latex(source, RenderLatexOptions { display: true }).expect(source)
    }

    #[test]
    fn jacobian_and_symbol_fixtures() {
        assert_eq!(render(r"\mathbb{C}^3 \to \mathbb{C}^3"), "ℂ³ → ℂ³");
        assert_eq!(render(r"F_1 = -\frac{1}{4x^2}."), "F₁ = -1/(4x²).");
        assert_eq!(render("-2"), "-2");
        assert_eq!(render(r"\deg q = 3"), "deg q = 3");
        assert_eq!(render(r"s \to \infty"), "s → ∞");
        assert_eq!(render(r"\Rightarrow"), "⇒");
        assert_eq!(render(r"\ge 2"), "≥ 2");
        assert_eq!(render(r"\mathrm{diag}(-1/2,1,1)"), "diag(-1/2,1,1)");
        assert_eq!(
            render(r"\sum_{i=0}^n \alpha_i + \int_0^\infty e^{-x^2}\,dx = \sqrt{\pi}"),
            "∑ᵢ₌₀ⁿ αᵢ + ∫₀^∞ e^(-x²) dx = √π",
        );
        assert_eq!(
            render(r"\binom{n}{k}+\vec{x}+\hat{y}+\overline{AB}"),
            "(n choose k)+x⃗+ŷ+overline(AB)",
        );
        assert_eq!(render(r"A\not\subseteq B,\quad x\not\in X"), "A ⊈ B, x ∉ X",);
        assert_eq!(
            render(r"\lvert{x}\rvert+\lVert{v}\rVert+\left.\frac{dy}{dx}\right|_{x=0}"),
            "|x|+‖v‖+dy/(dx)|ₓ₌₀",
        );
        assert_eq!(
            render(r"\left\lbrace x \middle| x>0 \right\rbrace"),
            "{ x | x > 0 }",
        );
        assert_eq!(
            render(r"\operatorname*{arg\,max}_{x\in X} f(x)"),
            "arg max[x∈X] f(x)",
        );
        assert_eq!(
            render(r"a\bmod n,\quad a\equiv b\pmod n"),
            "a mod n, a ≡ b (mod n)",
        );
        assert_eq!(
            render(r"\overset{!}{=}+\underset{n}{x}+\stackrel{def}{=}"),
            "=^!+xₙ+=ᵈᵉᶠ",
        );
        assert_eq!(
            render(r"\sqrt[2]{x}+\sqrt[3]{x}+\sqrt[4]{x}+\sqrt[n]{x}+\sqrt[k]{x+1}"),
            "√x+∛x+∜x+ⁿ√x+ᵏ√(x+1)",
        );
        assert_eq!(
            render(r"\textnormal{hello}+\mbox{world}+\boldsymbol{x}"),
            "hello+world+x",
        );
        assert_eq!(
            render(r"\begin{equation}\begin{split}a&=b\\&=c\end{split}\end{equation}"),
            "a = b\n= c",
        );
        assert_eq!(
            render(r"\begin{cases}a & x<0 \\ b & \text{if }x=0 \\ c & \text{otherwise}\end{cases}"),
            "⎧ a if x < 0\n⎨ b if x = 0\n⎩ c otherwise",
        );
        assert_eq!(
            render(r"\begin{pmatrix}1&200\\3000&4\end{pmatrix}"),
            "⎛ 1    │ 200 ⎞\n⎝ 3000 │ 4   ⎠",
        );
        assert_eq!(render(r"\sin\theta"), "sin θ");
        assert_eq!(render(r"\sin^2 x"), "sin² x");
        assert_eq!(render(r"i\sin\theta"), "i sin θ");
        assert_eq!(render(r"\det(A)"), "det(A)");
        assert_eq!(render(r"\pi\cdot\frac{1}{\pi}"), "π · 1/π");
        for source in ["x=y", "x =y", "x=\ny", "x\n=\ny"] {
            assert_eq!(render(source), "x = y", "{source}");
        }
    }

    #[test]
    fn display_limits_and_fractions() {
        assert_eq!(display(r"\sum_{i=0}^n x_i"), " n\n ∑  xᵢ\ni=0");
        assert_eq!(display(r"\min_{x\in X} f(x)"), "min f(x)\nx∈X");
        assert_eq!(
            display(r"\operatorname*{arg\,max}_{x\in X} f(x)"),
            "arg max f(x)\n  x∈X",
        );
        assert_eq!(display(r"\int\nolimits_0^1 f(x)\,dx"), "∫₀¹ f(x) dx");
        assert_eq!(display(r"\int\limits_0^1 f(x)\,dx"), "1\n∫ f(x) dx\n0");
        assert_eq!(display(r"\frac{x^2+1}{x-1}"), "x²+1\n────\nx-1");
        assert_eq!(display("\\frac{1}\n{2}"), "1\n─\n2");
        assert_eq!(display(r"e^{\frac{1}{2}}"), "e^(1/2)");
        assert_eq!(display(r"\tfrac{1}{2}"), "1/2");
        assert_eq!(
            display(r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}"),
            "    -b±√(b²-4ac)\nx = ────────────\n         2a",
        );
    }

    #[test]
    fn unsupported_and_malformed() {
        assert!(render_latex(r"x + \unknown{y}", RenderLatexOptions::default()).is_none());
        for source in [r"\frac{1}{x", "x}", r"\begin{matrix}1 & 2", "x\\"] {
            assert!(
                render_latex(source, RenderLatexOptions::default()).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn satellite_and_jacobian_session_fixtures() {
        assert_eq!(
            render(r"E \approx \frac{0.1\ \text{lux}}{100\ \text{lm/W}} = 0.001\ \text{W/m}^2"),
            "E ≈ (0.1 lux)/(100 lm/W) = 0.001 W/m²",
        );
        assert_eq!(
            render(r"\boxed{1\ \text{milliwatt per square metre}}"),
            "[1 milliwatt per square metre]",
        );
        assert_eq!(
            render(r"5\ \text{km}^2 = 5{,}000{,}000\ \text{m}^2"),
            "5 km² = 5,000,000 m²",
        );
        assert_eq!(
            render(
                r"P_{\text{light}} = 0.001 \times 5{,}000{,}000
= \boxed{5{,}000\ \text{W}}"
            ),
            "P_light = 0.001 × 5,000,000 = [5,000 W]",
        );
        assert_eq!(
            render(r"\pi(2.5\ \text{km})^2 = 19.6\ \text{km}^2"),
            "π(2.5 km)² = 19.6 km²",
        );
        assert_eq!(
            render(r"\det\!\left(\frac{\partial(F_1,F_2,F_3)}{\partial(x,y,z)}\right)=-2."),
            "det((∂(F₁,F₂,F₃))/(∂(x,y,z))) = -2.",
        );
        assert_eq!(
            render(
                r"\begin{aligned}
F(0,0,-\tfrac14)&=(-\tfrac14,0,0),\\
F(1,-\tfrac32,\tfrac{13}2)&=(-\tfrac14,0,0),\\
F(-1,\tfrac32,\tfrac{13}2)&=(-\tfrac14,0,0).
\end{aligned}"
            ),
            "F(0,0,-1/4) = (-1/4,0,0),\nF(1,-3/2,13/2) = (-1/4,0,0),\nF(-1,3/2,13/2) = (-1/4,0,0).",
        );
        assert_eq!(
            render(
                r"J = \begin{pmatrix}
\frac{\partial f_1}{\partial x} & \frac{\partial f_1}{\partial y} & \frac{\partial f_1}{\partial z} \\
\frac{\partial f_2}{\partial x} & \frac{\partial f_2}{\partial y} & \frac{\partial f_2}{\partial z} \\
\frac{\partial f_3}{\partial x} & \frac{\partial f_3}{\partial y} & \frac{\partial f_3}{\partial z}
\end{pmatrix}"
            ),
            "J = ⎛ (∂ f₁)/(∂ x) │ (∂ f₁)/(∂ y) │ (∂ f₁)/(∂ z) ⎞\n    ⎜ (∂ f₂)/(∂ x) │ (∂ f₂)/(∂ y) │ (∂ f₂)/(∂ z) ⎟\n    ⎝ (∂ f₃)/(∂ x) │ (∂ f₃)/(∂ y) │ (∂ f₃)/(∂ z) ⎠",
        );
        assert_eq!(render(r"F: \mathbb{C}^3 \to \mathbb{C}^3"), "F: ℂ³ → ℂ³");
        assert_eq!(render(r"\mathbb{P}^3"), "ℙ³");
        assert_eq!(render(r"x \neq 0"), "x ≠ 0");
        assert_eq!(render(r"n \geq 2"), "n ≥ 2");
        assert_eq!(render(r"f_1^{\text{ut}}, f_2^{\text{ut}}"), "f₁ᵘᵗ, f₂ᵘᵗ");
    }

    #[test]
    fn stress_test_and_display_matrix_fixtures() {
        assert_eq!(render(r"e^{i\pi}+1=0"), "e^(iπ)+1 = 0");
        assert_eq!(
            render(r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}"),
            "x = (-b±√(b²-4ac))/(2a)",
        );
        assert_eq!(
            render(r"\int_0^\infty e^{-x^2}\,dx=\frac{\sqrt{\pi}}{2}"),
            "∫₀^∞ e^(-x²) dx = (√π)/2",
        );
        assert_eq!(
            render(r"\sum_{n=1}^{\infty}\frac{1}{n^2}=\frac{\pi^2}{6}"),
            "∑ₙ₌₁^∞1/(n²) = π²/6",
        );
        assert_eq!(
            render(r"\lim_{x\to 0}\frac{\sin x}{x}=1"),
            "lim[x→0] (sin x)/x = 1"
        );
        assert_eq!(
            render(r"\epsilon+\varepsilon+\varsigma+\varkappa+\oplus+\otimes+\therefore+\because"),
            "ϵ+ε+ς+ϰ+⊕+⊗+∴+∵",
        );
        assert_eq!(
            render(r"\begin{alignedat}{2}a&=b&\quad c&=d\\e&=f&g&=h\end{alignedat}"),
            "a = b c = d\ne = f g = h",
        );
        assert_eq!(
            render(r"\begin{cases}a & x<0 \\ b & x=0 \\ c & x>0\end{cases}"),
            "⎧ a if x < 0\n⎨ b if x = 0\n⎩ c if x > 0",
        );
        assert_eq!(
            display(
                r"R\left(\frac{\pi}{4}\right)
=
\begin{pmatrix}
\frac{\sqrt{2}}{2} & -\frac{\sqrt{2}}{2}\\
\frac{\sqrt{2}}{2} & \frac{\sqrt{2}}{2}
\end{pmatrix}."
            ),
            "   π\nR( ─ ) = ⎛ (√2)/2 │ -(√2)/2 ⎞\n   4     ⎝ (√2)/2 │ (√2)/2  ⎠.",
        );
        assert_eq!(
            display(r"\sum_{i=0}^n x_i=\begin{pmatrix}a&b\\c&d\end{pmatrix}."),
            " n\n ∑  xᵢ = ⎛ a │ b ⎞\ni=0      ⎝ c │ d ⎠.",
        );
        assert_eq!(
            display(r"\frac{\frac{x^2+1}{x-1}-\frac{2x}{x+1}}{\frac{x}{x^2-1}}"),
            "(x²+1)/(x-1)-2x/(x+1)\n─────────────────────\n      x/(x²-1)",
        );
        let boxed = r"\boxed{
(1,1,1),\ (1,1,2),\ (1,2,5),\ (1,5,13),\ (2,5,29),\
(1,13,34),\ (1,34,89)
}.";
        assert_eq!(
            display(boxed),
            "[(1,1,1), (1,1,2), (1,2,5), (1,5,13), (2,5,29), (1,13,34), (1,34,89)].",
        );
        assert_eq!(render("a\\\r\nb"), "a b");
        assert_eq!(
            render(
                r"P_{\text{electric}} = 5\ \text{kW} \times 0.2
= \boxed{1\ \text{kW}}"
            ),
            "P_electric = 5 kW × 0.2 = [1 kW]",
        );
        assert_eq!(
            render(r"e^{i\theta}=\cos\theta+i\sin\theta"),
            "e^(iθ) = cos θ+i sin θ",
        );
        assert_eq!(
            render(r"\lim_{n\to\infty}\left(1+\frac{1}{n}\right)^n=e"),
            "lim[n→∞] (1+1/n)ⁿ = e",
        );
        assert_eq!(
            render(r"\int_0^1 \frac{x^2}{1+x^3}\,dx=\frac{1}{3}\ln 2"),
            "∫₀¹ x²/(1+x³) dx = 1/3 ln 2",
        );
        assert_eq!(
            render(r"\sum_{k=1}^{n}\frac{k}{k+1}=n+1-H_{n+1}"),
            "∑ₖ₌₁ⁿk/(k+1) = n+1-Hₙ₊₁",
        );
        assert_eq!(
            display(
                r"A\mathbf e_1=\begin{pmatrix}\pi\\0\end{pmatrix},\qquad A\mathbf e_2=\begin{pmatrix}0\\\frac{1}{\pi}\end{pmatrix}."
            ),
            "Ae₁ = ⎛ π ⎞, Ae₂ = ⎛ 0   ⎞\n      ⎝ 0 ⎠        ⎝ 1/π ⎠.",
        );
        assert_eq!(
            display(
                r"\mathbf w
=
R\left(\frac{\pi}{4}\right)
\begin{pmatrix}1\\0\end{pmatrix}
=
\begin{pmatrix}\frac{\sqrt{2}}{2}\\\frac{\sqrt{2}}{2}\end{pmatrix}."
            ),
            "       π\nw = R( ─ ) ⎛ 1 ⎞ = ⎛ (√2)/2 ⎞\n       4   ⎝ 0 ⎠   ⎝ (√2)/2 ⎠.",
        );
    }
}
