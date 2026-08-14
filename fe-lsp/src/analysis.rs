//! One pass of the compiler over the whole project, and everything the editor
//! features read out of it.

use std::path::{Path, PathBuf};

use fe_compiler::{Diagnostic, SourceUnit, SymbolRegistry};
use fe_lang::Ast;
use fe_lang::span::UnitId;

use crate::workspace::{Mode, Workspace};

pub struct Analysis {
    /// Indexed by `UnitId`. `units[i]`, `paths[i]` and `asts[i]` describe the
    /// same file — that correspondence is what turns a diagnostic's span back
    /// into somewhere the client can put a squiggle.
    pub paths: Vec<PathBuf>,
    pub units: Vec<SourceUnit>,
    pub asts: Vec<Ast>,
    pub diagnostics: Vec<Diagnostic>,
    pub registry: Option<SymbolRegistry>,
    pub mode: Mode,
}

impl Analysis {
    pub fn run(workspace: &Workspace) -> Analysis {
        let root = workspace.root().to_path_buf();
        let mut paths = Vec::new();
        let mut units = Vec::new();

        for (path, document) in workspace.iter() {
            // The unit's name is what a rendered diagnostic prints, so it should
            // read the way the build's output does: relative to the project.
            let name = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            units.push(SourceUnit::new(name, document.text.clone()));
            paths.push(path.clone());
        }

        let mode = workspace.mode();
        let (asts, diagnostics) = match workspace.registry() {
            // Everything: names, types, actions, ranges, limits — including the
            // two that are only reachable once code has been generated.
            Some(registry) => {
                let checked = fe_compiler::check_full(&units, registry);
                (checked.asts, checked.diagnostics.into_vec())
            }
            // Syntax only. Anything semantic would be an accusation the server
            // has no grounds for.
            None => {
                let (asts, diagnostics) = fe_compiler::parse(&units);
                (asts, diagnostics.into_vec())
            }
        };

        Analysis {
            paths,
            units,
            asts,
            diagnostics,
            registry: workspace.registry().cloned(),
            mode,
        }
    }

    pub fn unit_of(&self, path: &Path) -> Option<UnitId> {
        let path = crate::workspace::normalize(path);
        self.paths
            .iter()
            .position(|p| *p == path)
            .map(|index| UnitId(index as u32))
    }

    pub fn path(&self, unit: UnitId) -> Option<&Path> {
        self.paths.get(unit.0 as usize).map(PathBuf::as_path)
    }

    pub fn text(&self, unit: UnitId) -> Option<&str> {
        self.units.get(unit.0 as usize).map(SourceUnit::text)
    }

    pub fn ast(&self, unit: UnitId) -> Option<&Ast> {
        self.asts.get(unit.0 as usize)
    }

    pub fn asts(&self) -> impl Iterator<Item = (UnitId, &Ast)> {
        self.asts
            .iter()
            .enumerate()
            .map(|(index, ast)| (UnitId(index as u32), ast))
    }

    /// Every procedure declared anywhere in the project.
    ///
    /// One flat namespace across all files, which is why this is not per-file:
    /// `call HYD_2_ELECTRIC_PUMP_START` resolves to whichever file happens to
    /// declare it.
    pub fn procedures(&self) -> impl Iterator<Item = (UnitId, &fe_lang::ast::ProcedureDecl)> {
        self.asts()
            .flat_map(|(unit, ast)| ast.procedures.iter().map(move |decl| (unit, decl)))
    }

    pub fn procedure(&self, name: &str) -> Option<(UnitId, &fe_lang::ast::ProcedureDecl)> {
        self.procedures().find(|(_, decl)| decl.id.text == name)
    }
}
