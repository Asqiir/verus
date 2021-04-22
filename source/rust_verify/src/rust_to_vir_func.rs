use crate::rust_to_vir_expr::{
    expr_to_vir, get_fuel, get_mode, ident_to_var, pat_to_var, spanned_new, ty_to_vir,
};
use crate::{unsupported, unsupported_unless};
use rustc_ast::Attribute;
use rustc_hir::{Body, BodyId, Crate, FnDecl, FnHeader, FnSig, Generics, Param, Unsafety};
use rustc_middle::ty::TyCtxt;
use rustc_mir_build::thir;
use rustc_span::symbol::Ident;
use rustc_span::Span;
use std::rc::Rc;
use vir::ast::{ExprX, Exprs, FunctionX, KrateX, Mode, ParamX, StmtX, VirErr};
use vir::def::Spanned;

#[derive(Clone, Debug)]
struct Header {
    hidden: Vec<vir::ast::Ident>,
    require: Exprs,
}

fn read_header_block(block: &mut Vec<vir::ast::Stmt>) -> Result<Header, VirErr> {
    let mut hidden: Vec<vir::ast::Ident> = Vec::new();
    let mut require: Option<Exprs> = None;
    let mut n = 0;
    for stmt in block.iter() {
        match &stmt.x {
            StmtX::Expr(expr) => match &expr.x {
                ExprX::Call(x, es) if x.as_str() == "requires" => {
                    if require.is_some() {
                        return Err(Spanned::new(stmt.span.clone(),
                            "only one call to requires allowed (use requires([e1, ..., en]) for multiple expressions".to_string()));
                    }
                    require = Some(es.clone());
                }
                ExprX::Fuel(x, 0) => {
                    hidden.push(x.clone());
                }
                _ => break,
            },
            _ => break,
        }
        n += 1;
    }
    *block = block[n..].to_vec();
    Ok(Header { hidden, require: require.unwrap_or(Rc::new(vec![])) })
}

fn read_header(body: &mut vir::ast::Expr) -> Result<Header, VirErr> {
    match &body.x {
        ExprX::Block(stmts, expr) => {
            let mut block: Vec<vir::ast::Stmt> = (**stmts).clone();
            let header = read_header_block(&mut block)?;
            *body = Spanned::new(body.span.clone(), ExprX::Block(Rc::new(block), expr.clone()));
            Ok(header)
        }
        _ => read_header_block(&mut vec![]),
    }
}

fn body_to_vir<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: &BodyId,
    body: &'tcx Body<'tcx>,
) -> Result<vir::ast::Expr, VirErr> {
    let did = id.hir_id.owner;
    let arena = thir::Arena::default();
    let expr = thir::build_thir(
        tcx,
        rustc_middle::ty::WithOptConstParam::unknown(did),
        &arena,
        &body.value,
    );
    expr_to_vir(tcx, expr)
}

fn check_fn_decl<'tcx>(
    tcx: TyCtxt<'tcx>,
    decl: &'tcx FnDecl<'tcx>,
) -> Result<Option<vir::ast::Typ>, VirErr> {
    let FnDecl { inputs: _, output, c_variadic, implicit_self } = decl;
    unsupported_unless!(!c_variadic, "c_variadic");
    match implicit_self {
        rustc_hir::ImplicitSelfKind::None => {}
        _ => unsupported!("implicit_self"),
    }
    match output {
        rustc_hir::FnRetTy::DefaultReturn(_) => Ok(None),
        rustc_hir::FnRetTy::Return(ty) => Ok(Some(ty_to_vir(tcx, ty))),
    }
}

pub(crate) fn check_generics<'tcx>(generics: &'tcx Generics<'tcx>) -> Result<(), VirErr> {
    match generics {
        Generics { params, where_clause, span: _ } => {
            unsupported_unless!(params.len() == 0, "generics");
            unsupported_unless!(where_clause.predicates.len() == 0, "where clause");
        }
    }
    Ok(())
}

pub(crate) fn check_item_fn<'tcx>(
    tcx: TyCtxt<'tcx>,
    krate: &'tcx Crate<'tcx>,
    vir: &mut KrateX,
    id: Ident,
    attrs: &[Attribute],
    sig: &'tcx FnSig<'tcx>,
    generics: &Generics,
    body_id: &BodyId,
) -> Result<(), VirErr> {
    let ret = match sig {
        FnSig {
            header: FnHeader { unsafety, constness: _, asyncness: _, abi: _ },
            decl,
            span: _,
        } => {
            unsupported_unless!(*unsafety == Unsafety::Normal, "unsafe");
            check_fn_decl(tcx, decl)?
        }
    };
    check_generics(generics)?;
    let mode = get_mode(attrs);
    let fuel = get_fuel(attrs);
    match (mode, &ret) {
        (Mode::Exec, None) | (Mode::Proof, None) => {}
        (Mode::Exec, Some(_)) | (Mode::Proof, Some(_)) => {
            unsupported!("non-spec function return values");
        }
        (Mode::Spec, _) => {}
    }
    let body = &krate.bodies[body_id];
    let Body { params, value: _, generator_kind } = body;
    let mut vir_params: Vec<vir::ast::Param> = Vec::new();
    for (param, input) in params.iter().zip(sig.decl.inputs.iter()) {
        let Param { hir_id: _, pat, ty_span: _, span } = param;
        let name = Rc::new(pat_to_var(pat));
        let typ = ty_to_vir(tcx, input);
        let vir_param = spanned_new(*span, ParamX { name, typ });
        vir_params.push(vir_param);
    }
    match generator_kind {
        None => {}
        _ => {
            unsupported!("generator_kind", generator_kind);
        }
    }
    let mut vir_body = body_to_vir(tcx, body_id, body)?;
    let header = read_header(&mut vir_body)?;
    if mode == Mode::Spec && header.require.len() > 0 {
        let s = "spec functions cannot have requires/ensures";
        return Err(spanned_new(sig.span, s.to_string()));
    }
    let name = Rc::new(ident_to_var(&id));
    let params = Rc::new(vir_params);
    let func = FunctionX {
        name,
        mode,
        fuel,
        params,
        ret,
        require: header.require,
        hidden: Rc::new(header.hidden),
        body: Some(vir_body),
    };
    let function = spanned_new(sig.span, func);
    vir.functions.push(function);
    Ok(())
}

pub(crate) fn check_foreign_item_fn<'tcx>(
    tcx: TyCtxt<'tcx>,
    vir: &mut KrateX,
    id: Ident,
    span: Span,
    attrs: &[Attribute],
    decl: &'tcx FnDecl<'tcx>,
    idents: &[Ident],
    generics: &Generics,
) -> Result<(), VirErr> {
    let ret = check_fn_decl(tcx, decl)?;
    check_generics(generics)?;
    let mode = get_mode(attrs);
    let fuel = get_fuel(attrs);
    let mut vir_params: Vec<vir::ast::Param> = Vec::new();
    for (param, input) in idents.iter().zip(decl.inputs.iter()) {
        let name = Rc::new(ident_to_var(param));
        let typ = ty_to_vir(tcx, input);
        let vir_param = spanned_new(param.span, ParamX { name, typ });
        vir_params.push(vir_param);
    }
    let name = Rc::new(ident_to_var(&id));
    let params = Rc::new(vir_params);
    let func = FunctionX {
        name,
        fuel,
        mode,
        params,
        ret,
        require: Rc::new(vec![]),
        hidden: Rc::new(vec![]),
        body: None,
    };
    let function = spanned_new(span, func);
    vir.functions.push(function);
    Ok(())
}
