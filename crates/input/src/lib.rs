//! Input to analysis.

#![deny(clippy::pedantic, missing_debug_implementations, missing_docs, rust_2018_idioms)]
// TODO remove once rustfmt support lands
#![allow(clippy::manual_let_else)]

mod lower_cm;
mod lower_mlb;
mod root;
mod topo;
mod types;
mod util;

use paths::{PathId, PathMap, WithPath};
use util::{ErrorKind, ErrorSource, GroupPathKind};

pub use types::{Group, Severities};
pub use util::Error;

/// The input to analysis.
#[derive(Debug, Default)]
pub struct Input {
  /// A map from source paths to their contents.
  pub sources: PathMap<String>,
  /// A map from group paths to their (parsed) contents.
  pub groups: PathMap<types::Group>,
  /// The root group ids.
  pub root_group_paths: Vec<PathId>,
  /// Severities to override.
  pub severities: types::Severities,
  /// The language config.
  pub lang: config::file::Language,
  /// Errors when getting input.
  pub errors: Vec<Error>,
}

impl Input {
  /// Get input anchored at the root.
  ///
  /// # Errors
  ///
  /// When getting input failed.
  pub fn new<F>(fs: &F, store: &mut paths::Store, root: &paths::CanonicalPathBuf) -> Input
  where
    F: paths::FileSystem,
  {
    let mut ret = Input::default();
    let root = root::Root::new(fs, store, root, &mut ret.errors);
    ret.severities = root.config.severities;
    for group in root.groups {
      let path = store.get_path(group.path).as_path();
      let parent = path.parent().expect("group path with no parent");
      let parent = match util::str_path(ErrorSource::default(), parent) {
        Ok(x) => x,
        Err(e) => {
          ret.errors.push(e);
          continue;
        }
      };
      let path_var_env = slash_var_path::resolve_env(parent, root.config.path_vars.clone());
      let f = match group.kind {
        GroupPathKind::Cm => lower_cm::get,
        GroupPathKind::Mlb => lower_mlb::get,
      };
      f(fs, &mut ret.sources, &mut ret.groups, store, &path_var_env, group.path, &mut ret.errors);
      ret.root_group_paths.push(group.path);
    }
    let bas_decs = ret.groups.iter().map(|(&a, b)| (a, &b.bas_dec));
    if let Err(err) = topo::check(bas_decs) {
      ret.errors.push(Error::new(
        ErrorSource::default(),
        store.get_path(err.witness()).as_path().to_owned(),
        ErrorKind::Cycle,
      ));
    }
    ret
  }

  /// Return an iterator over the source paths.
  pub fn iter_sources(&self) -> impl Iterator<Item = WithPath<&str>> + '_ {
    self.sources.iter().map(|(&path, s)| path.wrap(s.as_str()))
  }

  /// Returns a mutable ref to the source for this path.
  pub fn get_mut_source(&mut self, path: PathId) -> Option<&mut String> {
    self.sources.get_mut(&path)
  }
}
