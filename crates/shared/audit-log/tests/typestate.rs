#[test]
fn typestate_compile_failures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/typestate_compile_fail/*.rs");
}
