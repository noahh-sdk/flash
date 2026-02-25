use lightningcss::stylesheet::{ParserOptions, PrinterOptions};

pub fn minify_html(input: String) -> Result<String, String> {
    String::from_utf8(minify_html::minify(
        input.as_bytes(),
        &minify_html::Cfg {
            keep_closing_tags: true,
            ..Default::default()
        },
    ))
    .map_err(|e| format!("{e}"))
}

pub fn minify_js(input: String) -> Result<String, String> {
    // TODO: implement this
    // swc is a stupidly big dependency, and minify_js fails with an assertion
    Ok(input)
}

pub fn minify_css(input: String) -> Result<String, String> {
    let sheet = lightningcss::stylesheet::StyleSheet::parse(&input, ParserOptions::default())
        .map_err(|e| format!("{e}"))?;
    sheet
        .to_css(PrinterOptions {
            minify: true,
            ..PrinterOptions::default()
        })
        .map(|s| s.code)
        .map_err(|e| format!("{e}"))
}
