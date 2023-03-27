//! Collect all the paths to ty cons in an env.

use crate::env::Env;
use crate::get_env::{get_env, GetEnvResult};
use fast_hash::FxHashSet;

pub(crate) fn get(env: &Env, path: &sml_path::Path) -> GetEnvResult<FxHashSet<sml_path::Path>> {
  get_env(env, path.all_names()).map(|env| {
    let mut ty_cons = FxHashSet::<sml_path::Path>::default();
    go(&mut Vec::new(), &mut ty_cons, env);
    ty_cons
  })
}

fn go(prefix: &mut Vec<str_util::Name>, ac: &mut FxHashSet<sml_path::Path>, env: &Env) {
  ac.extend(env.ty_env.iter().map(|(name, _)| sml_path::Path::new(prefix.clone(), name.clone())));
  for (name, env) in env.str_env.iter() {
    prefix.push(name.clone());
    go(prefix, ac, env);
    prefix.pop().unwrap();
  }
}

/// Joins two sequential paths into one.
pub(crate) fn join_paths(p1: &sml_path::Path, p2: &sml_path::Path) -> sml_path::Path {
  sml_path::Path::new(p1.all_names().chain(p2.prefix()).cloned(), p2.last().clone())
}
