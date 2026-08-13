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
