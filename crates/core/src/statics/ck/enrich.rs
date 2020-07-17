//! Check whether an environment enriches another environment.

// TODO remove when done
#![allow(unused)]

use crate::loc::Loc;
use crate::statics::types::{
  Env, Error, Result, Subst, SymTys, TyFcn, TyInfo, TyScheme, ValEnv, ValInfo,
};
use crate::util::eq_iter;

/// Returns Ok(()) iff got enriches want as per the Definition.
pub fn ck(loc: Loc, sym_tys: &SymTys, got: &Env, want: &Env) -> Result<()> {
  // For these for loops, we need the iteration order to be the same across different runs of the
  // program on the same file to ensure we get the same error message every time. This is useful for
  // testing and also is far less surprising for the user. Because of this need, the maps are
  // BTreeMaps, not HashMaps. See types.rs.
  for (name, want) in want.str_env.iter() {
    match got.str_env.get(name) {
      None => return Err(loc.wrap(Error::Todo("missing a struct"))),
      Some(got) => ck(loc, sym_tys, got, want)?,
    }
  }
  for (name, want) in want.ty_env.inner.iter() {
    match got.ty_env.inner.get(name) {
      None => return Err(loc.wrap(Error::Todo("missing a type"))),
      Some(got) => ck_ty_info(loc, sym_tys, got, want)?,
    }
  }
  for (name, want) in want.val_env.iter() {
    match got.val_env.get(name) {
      None => return Err(loc.wrap(Error::Todo("missing a value"))),
      Some(got) => ck_val_info(loc, sym_tys, got, want)?,
    }
  }
  Ok(())
}

fn ck_val_info(loc: Loc, sym_tys: &SymTys, got: &ValInfo, want: &ValInfo) -> Result<()> {
  if got.id_status != want.id_status && !want.id_status.is_val() {
    return Err(loc.wrap(Error::Todo("incompatible id statuses")));
  }
  ck_generalizes(loc, sym_tys, want.ty_scheme.clone(), got.ty_scheme.clone())?;
  Ok(())
}

fn ck_ty_info(loc: Loc, sym_tys: &SymTys, got: &TyInfo, want: &TyInfo) -> Result<()> {
  ck_ty_fcn_eq(loc, sym_tys, got.ty_fcn(sym_tys), want.ty_fcn(sym_tys))?;
  let want = match want {
    TyInfo::Alias(_) => return Ok(()),
    TyInfo::Sym(sym) => &sym_tys.get(sym).unwrap().val_env,
  };
  if want.is_empty() {
    return Ok(());
  }
  let got = match got {
    TyInfo::Alias(_) => return Err(loc.wrap(Error::Todo("got empty want non-empty"))),
    TyInfo::Sym(sym) => &sym_tys.get(sym).unwrap().val_env,
  };
  ck_val_env_eq(loc, sym_tys, got, want)
}

fn ck_val_env_eq(loc: Loc, sym_tys: &SymTys, got: &ValEnv, want: &ValEnv) -> Result<()> {
  if !eq_iter(want.keys(), got.keys()) {
    return Err(loc.wrap(Error::Todo("unequal keys")));
  }
  for (name, want_vi) in want {
    let got_vi = got.get(name).unwrap();
    if want_vi.id_status != got_vi.id_status {
      return Err(loc.wrap(Error::Todo("unequal id statuses")));
    }
    ck_ty_fcn_eq(loc, sym_tys, &got_vi.ty_scheme, &want_vi.ty_scheme)?;
  }
  Ok(())
}

/// Returns `Ok(())` if want = got. From the Definition: "It can be shown that lhs = rhs iff lhs >>>
/// rhs and rhs >>> lhs."
fn ck_ty_fcn_eq(loc: Loc, sym_tys: &SymTys, got: &TyFcn, want: &TyFcn) -> Result<()> {
  ck_generalizes(loc, sym_tys, want.clone(), got.clone())?;
  ck_generalizes(loc, sym_tys, got.clone(), want.clone())?;
  Ok(())
}

/// Returns Ok(s) iff want generalizes got as per the Definition, and s is this witness to this
/// fact. TODO is this right?
fn ck_generalizes(loc: Loc, sym_tys: &SymTys, want: TyScheme, got: TyScheme) -> Result<Subst> {
  let want_free_tvs = want.free_ty_vars();
  for tv in got.ty_vars.iter() {
    if want_free_tvs.contains(tv) {
      return Err(loc.wrap(Error::Todo("bad free ty var")));
    }
  }
  let mut ret = Subst::default();
  ret.unify(loc, &sym_tys, want.ty, got.ty)?;
  // TODO
  Ok(ret)
}
