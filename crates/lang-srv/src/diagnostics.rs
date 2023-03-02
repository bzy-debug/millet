//! Publish diagnostics.

use crate::convert;
use crate::state::{Mode, St};
use fast_hash::FxHashSet;
use lsp_types::Url;

pub(crate) fn try_publish(st: &mut St) -> bool {
  let root = match &mut st.mode {
    Mode::Root(x) => x,
    Mode::NoRoot(_) => return false,
  };
  let got_many = st.analysis.get_many(&root.input);
  let mut has_diagnostics = FxHashSet::<Url>::default();
  for err in &root.input.errors {
    let did_send_as_diagnostic = if err.abs_path().is_file() {
      match convert::file_url(err.abs_path()) {
        Ok(url) => {
          has_diagnostics.insert(url.clone());
          st.cx.send_diagnostics(
            url,
            vec![convert::diagnostic(
              err.display(root.path.as_path()).to_string(),
              err.range(),
              err.code(),
              err.severity(),
              st.cx.options.diagnostics_more_info_hint,
            )],
          );
          true
        }
        Err(_) => false,
      }
    } else {
      false
    };
    if !did_send_as_diagnostic {
      st.cx.show_error(
        format!(
          "{}: {}",
          err.maybe_rel_path(root.path.as_path()).display(),
          err.display(root.path.as_path())
        ),
        err.code(),
      );
    }
  }
  for (path_id, errors) in got_many {
    let path = st.cx.store.get_path(path_id);
    let url = match convert::file_url(path.as_path()) {
      Ok(x) => x,
      Err(e) => {
        log::error!("couldn't get path as a file url: {e:#}");
        continue;
      }
    };
    let ds = convert::diagnostics(errors, st.cx.options.diagnostics_more_info_hint);
    if ds.is_empty() {
      continue;
    }
    has_diagnostics.insert(url.clone());
    st.cx.send_diagnostics(url, ds);
  }
  // iter over the old list of urls with diagnostics.
  for url in std::mem::take(&mut st.has_diagnostics) {
    if has_diagnostics.contains(&url) {
      // had old and new diagnostics. just sent the new ones.
      continue;
    }
    // had old diagnostics, but no new diagnostics. clear the old diagnostics.
    st.cx.send_diagnostics(url, Vec::new());
  }
  st.has_diagnostics = has_diagnostics;
  true
}
