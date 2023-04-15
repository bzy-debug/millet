//! Bases. (The plural of "basis".)

use crate::core_info::{IdStatus, TyEnv, TyInfo, ValEnv, ValInfo};
use crate::disallow::{self, Disallow};
use crate::env::{Cx, Env, FunEnv, SigEnv, StrEnv};
use crate::sym::{Equality, Sym, Syms};
use crate::types::ty::{BoundTyVar, RecordData, Ty, TyScheme, TyVarKind, Tys};
use crate::{def::PrimitiveKind, get_env::get_mut_env, item::Item, overload};
use fast_hash::FxHashMap;

/// A basis.
#[derive(Debug, Default, Clone)]
pub struct Bs {
  pub(crate) env: Env,
  pub(crate) sig_env: SigEnv,
  pub(crate) fun_env: FunEnv,
}

impl Bs {
  pub(crate) fn as_cx(&self) -> Cx {
    Cx { env: self.env.clone(), fixed: FxHashMap::default() }
  }

  /// Append other onto self, emptying other.
  pub fn append(&mut self, mut other: Bs) {
    self.env.append(&mut other.env);
    self.sig_env.append(&mut other.sig_env);
    self.fun_env.append(&mut other.fun_env);
  }

  /// Consolidates internal memory for this, so that it will be faster to clone next time.
  pub fn consolidate(&mut self) {
    self.env.consolidate();
    self.sig_env.consolidate();
    self.fun_env.consolidate();
  }

  /// Adds the item named `other_name` from `other` into `self` with the name `name`, or
  /// returns `false` if this was not possible.
  pub fn add(
    &mut self,
    ns: sml_namespace::Module,
    name: str_util::Name,
    other: &Self,
    other_name: &str_util::Name,
  ) -> bool {
    match ns {
      sml_namespace::Module::Structure => match other.env.str_env.get(other_name) {
        Some(env) => {
          self.env.str_env.insert(name, env.clone());
          true
        }
        None => false,
      },
      sml_namespace::Module::Signature => match other.sig_env.get(other_name) {
        Some(env) => {
          self.sig_env.insert(name, env.clone());
          true
        }
        None => false,
      },
      sml_namespace::Module::Functor => match other.fun_env.get(other_name) {
        Some(env) => {
          self.fun_env.insert(name, env.clone());
          true
        }
        None => false,
      },
    }
  }

  /// Disallow a value.
  ///
  /// # Errors
  ///
  /// If the path couldn't be disallowed.
  pub fn disallow_val(&mut self, val: &sml_path::Path) -> Result<(), disallow::Error> {
    let env = match get_mut_env(&mut self.env, val.prefix()) {
      Ok(x) => x,
      Err(n) => return Err(disallow::ErrorKind::Undefined(Item::Struct, n.clone()).into()),
    };
    let val_info = match env.val_env.get_mut(val.last()) {
      Some(x) => x,
      None => return Err(disallow::ErrorKind::Undefined(Item::Val, val.last().clone()).into()),
    };
    match &val_info.disallow {
      None => {
        val_info.disallow = Some(Disallow::Directly);
        Ok(())
      }
      Some(x) => Err(disallow::ErrorKind::Already(x.clone()).into()),
    }
  }
}

/// Returns the minimal basis and symbols.
///
/// This is distinct from `std_basis` in analysis. This (mostly) just has the definitions that can't
/// be expressed with regular SML files, like `int` and `real` and `string`. Also `bool` and `list`
/// because rebinding their constructor names is forbidden.
///
/// # Panics
///
/// Upon internal error.
#[must_use]
pub fn minimal() -> (Syms, Tys, Bs) {
  let mut tys = Tys::default();
  // @sync(special_sym_order)
  let mut syms = Syms::default();
  for sym in [Sym::INT, Sym::WORD, Sym::REAL, Sym::CHAR, Sym::STRING] {
    insert_special(&mut syms, sym, basic_datatype(&mut tys, sym, &[]));
  }
  syms.overloads_mut().int.push(Sym::INT);
  syms.overloads_mut().word.push(Sym::WORD);
  syms.overloads_mut().real.push(Sym::REAL);
  syms.overloads_mut().char.push(Sym::CHAR);
  syms.overloads_mut().string.push(Sym::STRING);
  let bool_info = basic_datatype(&mut tys, Sym::BOOL, &[PrimitiveKind::True, PrimitiveKind::False]);
  insert_special(&mut syms, Sym::BOOL, bool_info);
  let list_info = {
    let list = |tys: &mut Tys, a: Ty| tys.con(vec![a], Sym::LIST);
    let alpha_list = ty_scheme_one(&mut tys, TyVarKind::Regular, list);
    let cons = ty_scheme_one(&mut tys, TyVarKind::Regular, |tys, a| {
      let a_list = list(tys, a);
      let pair_a_a_list = pair(tys, a, a_list);
      tys.fun(pair_a_a_list, a_list)
    });
    TyInfo {
      ty_scheme: alpha_list.clone(),
      val_env: datatype_ve([(PrimitiveKind::Nil, alpha_list), (PrimitiveKind::Cons, cons)]),
      def: Some(PrimitiveKind::List.into()),
      disallow: None,
    }
  };
  insert_special(&mut syms, Sym::LIST, list_info);
  let ref_info = {
    let ref_ = |tys: &mut Tys, a: Ty| tys.con(vec![a], Sym::REF);
    let con = ty_scheme_one(&mut tys, TyVarKind::Regular, |tys, a| {
      let a_ref = ref_(tys, a);
      tys.fun(a, a_ref)
    });
    TyInfo {
      ty_scheme: ty_scheme_one(&mut tys, TyVarKind::Regular, ref_),
      val_env: datatype_ve([(PrimitiveKind::RefVal, con)]),
      def: Some(PrimitiveKind::RefTy.into()),
      disallow: None,
    }
  };
  insert_special(&mut syms, Sym::REF, ref_info);
  let unit = tys.record(RecordData::new());
  let aliases = [(PrimitiveKind::Unit, unit), (PrimitiveKind::Exn, tys.exn())];
  let ty_env: TyEnv = syms
    .iter_syms()
    .map(|sym_info| {
      assert!(sym_info.path.prefix().is_empty(), "only built-in types are currently in this Syms");
      (sym_info.path.last().clone(), sym_info.ty_info.clone())
    })
    .chain(aliases.into_iter().map(|(name, ty)| {
      let ti = TyInfo {
        ty_scheme: TyScheme::zero(ty),
        val_env: ValEnv::default(),
        def: Some(name.into()),
        disallow: None,
      };
      (str_util::Name::new(name.as_str()), ti)
    }))
    .collect();
  let fns = {
    let num_pair_to_num =
      ov_fn(&mut tys, overload::Composite::Num.into(), |tys, a| (dup(tys, a), a));
    let real_pair_to_real =
      ov_fn(&mut tys, overload::Basic::Real.into(), |tys, a| (dup(tys, a), a));
    let numtxt_pair_to_bool =
      ov_fn(&mut tys, overload::Composite::NumTxt.into(), |tys, a| (dup(tys, a), tys.bool()));
    let realint_to_realint = ov_fn(&mut tys, overload::Composite::RealInt.into(), |_, a| (a, a));
    let wordint_pair_to_wordint =
      ov_fn(&mut tys, overload::Composite::WordInt.into(), |tys, a| (dup(tys, a), a));
    let equality_pair_to_bool = ty_scheme_one(&mut tys, TyVarKind::Equality, |tys, a| {
      let a_a = dup(tys, a);
      let b = tys.bool();
      tys.fun(a_a, b)
    });
    let s = tys.string();
    [
      (PrimitiveKind::Mul, num_pair_to_num.clone()),
      (PrimitiveKind::Add, num_pair_to_num.clone()),
      (PrimitiveKind::Sub, num_pair_to_num),
      (PrimitiveKind::RealDiv, real_pair_to_real),
      (PrimitiveKind::Lt, numtxt_pair_to_bool.clone()),
      (PrimitiveKind::LtEq, numtxt_pair_to_bool.clone()),
      (PrimitiveKind::Gt, numtxt_pair_to_bool.clone()),
      (PrimitiveKind::GtEq, numtxt_pair_to_bool),
      (PrimitiveKind::Neg, realint_to_realint.clone()),
      (PrimitiveKind::Abs, realint_to_realint),
      (PrimitiveKind::Div, wordint_pair_to_wordint.clone()),
      (PrimitiveKind::Mod, wordint_pair_to_wordint),
      (PrimitiveKind::Eq, equality_pair_to_bool.clone()),
      (PrimitiveKind::Neq, equality_pair_to_bool),
      (PrimitiveKind::Use, TyScheme::zero(tys.fun(s, unit))),
    ]
  };
  let val_env: ValEnv = ty_env
    .iter()
    .flat_map(|(_, ti)| ti.val_env.iter().map(|(a, b)| (a.clone(), b.clone())))
    .chain(fns.into_iter().map(|(name, ty_scheme)| {
      let vi = ValInfo {
        ty_scheme,
        id_status: IdStatus::Val,
        defs: fast_hash::set([name.into()]),
        disallow: None,
      };
      (str_util::Name::new(name.as_str()), vi)
    }))
    .collect();
  let bs = Bs {
    fun_env: FunEnv::default(),
    sig_env: SigEnv::default(),
    env: Env { str_env: StrEnv::default(), ty_env, val_env, def: None, disallow: None },
  };
  (syms, tys, bs)
}

fn insert_special(syms: &mut Syms, sym: Sym, ty_info: TyInfo) {
  assert_ne!(sym, Sym::EXN);
  let equality = if sym == Sym::REF {
    Equality::Always
  } else if sym == Sym::REAL {
    Equality::Never
  } else {
    Equality::Sometimes
  };
  let started =
    syms.start(sml_path::Path::one(str_util::Name::new(sym.primitive().unwrap().as_str())));
  assert_eq!(sym, started.sym());
  syms.finish(started, ty_info, equality);
}

fn basic_datatype(tys: &mut Tys, sym: Sym, ctors: &'static [PrimitiveKind]) -> TyInfo {
  let ty_scheme = TyScheme::zero(tys.con(Vec::new(), sym));
  let val_env = datatype_ve(ctors.iter().map(|&x| (x, ty_scheme.clone())));
  TyInfo { ty_scheme, val_env, def: Some(sym.primitive().unwrap().into()), disallow: None }
}

fn datatype_ve<I>(xs: I) -> ValEnv
where
  I: IntoIterator<Item = (PrimitiveKind, TyScheme)>,
{
  xs.into_iter()
    .map(|(name, ty_scheme)| {
      let vi = ValInfo {
        ty_scheme,
        id_status: IdStatus::Con,
        defs: fast_hash::set([name.into()]),
        disallow: None,
      };
      (str_util::Name::new(name.as_str()), vi)
    })
    .collect()
}

fn dup(tys: &mut Tys, ty: Ty) -> Ty {
  pair(tys, ty, ty)
}

fn pair(tys: &mut Tys, t1: Ty, t2: Ty) -> Ty {
  tys.record(RecordData::from([(sml_hir::Lab::Num(1), t1), (sml_hir::Lab::Num(2), t2)]))
}

fn ov_fn<F>(tys: &mut Tys, ov: overload::Overload, f: F) -> TyScheme
where
  F: FnOnce(&mut Tys, Ty) -> (Ty, Ty),
{
  ty_scheme_one(tys, TyVarKind::Overloaded(ov), |tys, a| {
    let (a, b) = f(tys, a);
    tys.fun(a, b)
  })
}

fn ty_scheme_one<F>(tys: &mut Tys, k: TyVarKind, f: F) -> TyScheme
where
  F: FnOnce(&mut Tys, Ty) -> Ty,
{
  let mut bound_vars = Vec::<TyVarKind>::new();
  let mut ty = None::<Ty>;
  BoundTyVar::add_to_binder(&mut bound_vars, |bv| {
    ty = Some(Ty::bound_var(bv));
    k
  });
  TyScheme { bound_vars, ty: f(tys, ty.unwrap()) }
}
