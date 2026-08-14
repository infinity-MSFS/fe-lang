mod codegen;
mod emit;
mod ir;
mod sema;
pub mod symbols;

pub use emit::Stats;
pub use fe_lang;
pub use fe_lang::diagnostics::{Diagnostic, Diagnostics, Label, Severity, codes};
pub use fe_lang::span::{Location, SourceMap, SourceUnit, Span, UnitId};
pub use symbols::{
    ControlKind, ControlSpec, ControlSymbol, RegistryError, Resolved, StateSymbol, SymbolRegistry,
    ValueType,
};

#[derive(Clone, Debug)]
pub struct Compiled {
    bytes: Vec<u8>,
    warnings: Vec<Diagnostic>,
    stats: Stats,
}

impl Compiled {
    /// the compiled database
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// take ownership of the compiled database
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// non-fatal diagnostics produced during compilation
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// a reader over the freshly compiled bytes
    pub fn database(&self) -> fe_runtime::ProcedureDatabase<'_> {
        fe_runtime::ProcedureDatabase::from_bytes(&self.bytes)
            .expect("compiler produced a database it cannot read")
    }
}

#[derive(Clone, Debug)]
pub struct CompileError {
    diagnostics: Vec<Diagnostic>,
}

impl CompileError {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn render(&self, units: &[SourceUnit]) -> String {
        let sources = SourceMap::new(units);
        let mut out = String::new();
        for diagnostic in &self.diagnostics {
            out.push_str(&diagnostic.render(&sources));
            out.push('\n');
        }
        out
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let errors = self.errors().count();
        write!(f, "compilation failed with {errors} error(s)")?;
        if let Some(first) = self.errors().next() {
            write!(f, ": [{}] {}", first.code, first.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug)]
pub struct Checked {
    pub asts: Vec<fe_lang::Ast>,
    pub diagnostics: Diagnostics,
    pub compiled: Option<Compiled>,
}

impl Checked {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.has_errors()
    }
}

pub fn check_full(units: &[SourceUnit], registry: &SymbolRegistry) -> Checked {
    let mut diagnostics = Diagnostics::new();
    let parsed = parse_all(units, &mut diagnostics);
    let asts = parsed.iter().map(|(_, ast)| ast.clone()).collect();

    let bail = |diagnostics: Diagnostics| Checked {
        asts: Vec::new(),
        diagnostics,
        compiled: None,
    };

    let Some(module) = sema::analyze(&parsed, registry, &mut diagnostics) else {
        return Checked {
            asts,
            ..bail(diagnostics)
        };
    };

    let (bytes, stats) = match emit::emit(&module) {
        Ok(result) => result,
        Err(errors) => {
            for error in errors {
                diagnostics.push(error);
            }
            return Checked {
                asts,
                ..bail(diagnostics)
            };
        }
    };

    if let Err(error) = fe_runtime::ProcedureDatabase::from_bytes(&bytes) {
        diagnostics.push(
            Diagnostic::error(
                "E0999",
                format!("internal error: the compiler produced an invalid database ({error})"),
                Label::bare(Span::new(UnitId(0), 0, 0)),
            )
            .with_note("this is a bug in fe-compiler, not in the procedure source"),
        );
        return Checked {
            asts,
            ..bail(diagnostics)
        };
    }

    Checked {
        asts,
        compiled: Some(Compiled {
            bytes,
            warnings: diagnostics.iter().cloned().collect(),
            stats,
        }),
        diagnostics,
    }
}

pub fn compile(units: &[SourceUnit], registry: &SymbolRegistry) -> Result<Compiled, CompileError> {
    let checked = check_full(units, registry);
    match checked.compiled {
        Some(compiled) => Ok(compiled),
        None => Err(CompileError {
            diagnostics: checked.diagnostics.into_vec(),
        }),
    }
}

pub fn check(units: &[SourceUnit], registry: &SymbolRegistry) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    let parsed = parse_all(units, &mut diagnostics);
    let _ = sema::analyze(&parsed, registry, &mut diagnostics);
    diagnostics
}

pub fn parse(units: &[SourceUnit]) -> (Vec<fe_lang::Ast>, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let parsed = parse_all(units, &mut diagnostics);
    (
        parsed.into_iter().map(|(_, ast)| ast).collect(),
        diagnostics,
    )
}

fn parse_all(units: &[SourceUnit], diagnostics: &mut Diagnostics) -> Vec<(UnitId, fe_lang::Ast)> {
    units
        .iter()
        .enumerate()
        .map(|(index, unit)| {
            let id = UnitId(index as u32);
            (id, fe_lang::parse_unit(id, unit.text(), diagnostics))
        })
        .collect()
}
