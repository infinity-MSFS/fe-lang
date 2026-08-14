mod support;

#[test]
fn compiles_the_examples() {
    let bytes = support::compile_examples();
    let db = fe_runtime::ProcedureDatabase::from_bytes(&bytes).unwrap();
    for p in db.procedures() {
        println!("{} :: {} ({} bytes)", p.id, p.name, p.code_size());
        let mut text = String::new();
        fe_runtime::disassemble(&p, &mut text).unwrap();
        println!("{text}");
    }
    println!("total {} bytes", bytes.len());
}

#[test]
fn check_full_returns_the_trees_alongside_the_verdict() {
    let units = support::example_units();
    let checked = fe_compiler::check_full(&units, &support::registry());

    assert!(
        !checked.has_errors(),
        "{}",
        checked
            .diagnostics
            .render(&fe_compiler::SourceMap::new(&units))
    );
    assert_eq!(checked.asts.len(), units.len());
    assert!(checked.compiled.is_some());
    assert_eq!(
        checked.compiled.as_ref().unwrap().as_bytes(),
        support::compile_examples(),
    );

    let declared: usize = checked.asts.iter().map(|a| a.procedures.len()).sum();
    assert_eq!(
        declared,
        checked.compiled.unwrap().database().procedures().count()
    );
}

#[test]
fn check_full_keeps_the_trees_when_analysis_fails() {
    let units = vec![fe_compiler::SourceUnit::new(
        "broken.fe",
        "procedure P {\n name \"P\"\n category normal\n check NO_SUCH_CONTROL\n}",
    )];
    let checked = fe_compiler::check_full(&units, &support::registry());

    assert!(checked.has_errors());
    assert!(checked.compiled.is_none());
    assert_eq!(checked.asts.len(), 1);
    assert_eq!(checked.asts[0].procedures[0].id.text, "P");
    assert!(checked.diagnostics.iter().any(|d| d.code == "E0201"));
}
