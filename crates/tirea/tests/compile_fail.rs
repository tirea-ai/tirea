#[test]
fn tirea_does_not_reexport_ag_ui_protocol_types() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/no_agui_reexports.rs");
}
