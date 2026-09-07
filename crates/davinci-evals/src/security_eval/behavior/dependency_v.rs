pub fn render_template(input: &str) -> String {
    yaml_template(input)
}

fn yaml_template(input: &str) -> String {
    format!("yaml:{input}")
}
