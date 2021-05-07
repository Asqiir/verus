use crate::ast::{Expr, ExprX, Function, Ident, Krate, Mode, Path, Variants, VirErr};
use crate::ast_util::err_string;
use crate::ast_visitor::map_expr_visitor;
use crate::def::FUEL_ID;
use air::ast::{Command, CommandX, Commands, DeclX, MultiOp, Span};
use air::ast_util::str_typ;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Ctx {
    pub(crate) datatypes: HashMap<Path, Variants>,
    pub(crate) functions: Vec<Function>,
    pub(crate) func_map: HashMap<Ident, Function>,
    pub(crate) chosen_triggers: std::cell::RefCell<Vec<(Span, Vec<Vec<String>>)>>, // diagnostics
}

impl Ctx {
    fn check_defined_earlier(
        func_map: &HashMap<Ident, Function>,
        expr: &Expr,
    ) -> Result<Expr, VirErr> {
        match &expr.x {
            ExprX::Call(x, _, _) | ExprX::Fuel(x, _) => {
                if !func_map.contains_key(x) {
                    return err_string(
                        &expr.span,
                        format!(
                            "because support for recursion isn't yet implemented, {} must be defined before it is called",
                            &x
                        ),
                    );
                }
            }
            _ => {}
        }
        Ok(expr.clone())
    }

    fn check_no_recursion(
        func_map: &HashMap<Ident, Function>,
        function: &Function,
    ) -> Result<(), VirErr> {
        // Recursion is not implemented yet, so make sure there is no recursion.
        // Check this by simply forcing all the declarations to be in order.
        if let Some(body) = &function.x.body {
            map_expr_visitor(body, &mut |expr| Self::check_defined_earlier(func_map, expr))?;
        }
        Ok(())
    }

    pub fn new(krate: &Krate) -> Result<Self, VirErr> {
        let datatypes = krate
            .datatypes
            .iter()
            .map(|d| (d.x.path.clone(), d.x.variants.clone()))
            .collect::<HashMap<_, _>>();
        let mut functions: Vec<Function> = Vec::new();
        let mut func_map: HashMap<Ident, Function> = HashMap::new();
        for function in krate.functions.iter() {
            Self::check_no_recursion(&func_map, function)?;
            functions.push(function.clone());
            func_map.insert(function.x.name.clone(), function.clone());
        }
        let chosen_triggers: std::cell::RefCell<Vec<(Span, Vec<Vec<String>>)>> =
            std::cell::RefCell::new(Vec::new());
        Ok(Ctx { datatypes, functions, func_map, chosen_triggers })
    }

    pub fn prelude(&self) -> Commands {
        let nodes = crate::prelude::prelude_nodes();
        air::print_parse::nodes_to_commands(&nodes).expect("internal error: malformed prelude")
    }

    pub fn fuel(&self) -> Commands {
        let mut ids: Vec<air::ast::Expr> = Vec::new();
        let mut commands: Vec<Command> = Vec::new();
        for function in &self.functions {
            match (function.x.mode, function.x.body.as_ref()) {
                (Mode::Spec, Some(_)) => {
                    let id = crate::def::prefix_fuel_id(&function.x.name);
                    ids.push(air::ast_util::ident_var(&id));
                    let typ_fuel_id = str_typ(&FUEL_ID);
                    let decl = Rc::new(DeclX::Const(id, typ_fuel_id));
                    commands.push(Rc::new(CommandX::Global(decl)));
                }
                _ => {}
            }
        }
        let distinct = Rc::new(air::ast::ExprX::Multi(MultiOp::Distinct, Rc::new(ids)));
        let decl = Rc::new(DeclX::Axiom(distinct));
        commands.push(Rc::new(CommandX::Global(decl)));
        Rc::new(commands)
    }

    // Report chosen triggers as strings for printing diagnostics
    pub fn get_chosen_triggers(&self) -> Vec<(Span, Vec<Vec<String>>)> {
        self.chosen_triggers.borrow().clone()
    }
}