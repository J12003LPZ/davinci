pub fn render_template(input: &str) -> String {
    json_template(input)
}

fn json_template(input: &str) -> String {
    format!("json:{input}")
}
