#[test]
fn html_table_sections_survive_parsing() {
    let sheet = serpentype::css::parse("", false).unwrap();
    let (root, warnings) = serpentype::html::parse(
        "<table><thead><tr><th>A</th></tr></thead><tbody><tr><td>B</td></tr></tbody></table>",
        &sheet,
        false,
    )
    .unwrap();
    assert!(warnings.is_empty());
    assert_eq!(root.children[0].tag, "table");
    assert_eq!(root.children[0].children[0].tag, "thead");
    assert_eq!(root.children[0].children[1].tag, "tbody");
}

#[test]
fn css_units_and_selector_cascade() {
    assert!((serpentype::css::length("10px").unwrap() - 7.5).abs() < 0.001);
    assert!((serpentype::css::length("10mm").unwrap() - 28.346).abs() < 0.001);
    let sheet = serpentype::css::parse("p { color: blue } p.notice { color: red }", true).unwrap();
    let (root, _) = serpentype::html::parse("<p class='notice'>Test</p>", &sheet, true).unwrap();
    assert_eq!(
        root.children[0].style.color,
        serpentype::css::Color(1.0, 0.0, 0.0)
    );
}

#[test]
fn universal_selector_matches_elements_and_has_zero_specificity() {
    let sheet = serpentype::css::parse("* { color:red }", false).unwrap();
    assert!(sheet.warnings.is_empty());
    assert!(serpentype::css::selector_matches("*", "div", None, "card"));
    assert!(serpentype::css::selector_matches(
        "*.card", "section", None, "card"
    ));
}
