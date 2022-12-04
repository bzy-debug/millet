//! Test infra.

mod expect;
mod input;
mod reason;
mod show;

use diagnostic_util::Severity;

/// Given the string of an SML program with some expectation comments, panics iff the expectation
/// comments are not satisfied.
///
/// Expectation comments are regular SML comments except they:
///
/// - are always on only one line
/// - start with `(**`
/// - point at either:
///   - the specific things that should have errors with `^` or `v`
///   - lines on which an error should begin (but not necessarily end) with `+`
///
/// The expectation messages have a certain format:
///
/// - Error expects that must match **exactly** have no prefix.
/// - Error expects that must merely be **contained** begin with `contains: `.
/// - Hover expects begin with `hover: `, and the actual hover must merely contain the expectation.
///
/// To construct the string to pass without worrying about Rust string escape sequences, use the raw
/// string syntax: `r#"..."#`.
///
/// ```ignore
/// check(r#"
/// (**       vvv error about bar *)
/// val foo = bar quz
/// (**           ^^^ hover: info about quz *)
/// "#);
/// ```
///
/// Note that this also sets up logging.
#[track_caller]
pub(crate) fn check(s: &str) {
  check_multi(one_file_fs(s));
}

/// Like [`check`], but allows multiple files.
#[track_caller]
pub(crate) fn check_multi<const N: usize>(files: [(&str, &str); N]) {
  go(files, analysis::StdBasis::Minimal, Outcome::Pass, Severity::Error);
}

/// Like [`check`], but the expectation comments should be not satisfied.
///
/// For instance, the following program has an expectation comment that doesn't make sense, since
/// `1 + 2` should typecheck. but since `fail` expects the the comments to be unsatisfied, the test
/// passes.
///
/// ```ignore
/// fail(r#"
/// val _ = 1 + 2
/// (**     ^^^^^ contains: expected bool, found int *)
/// "#);
/// ```
///
/// This is useful if support for something is not implemented, but planned for later:
///
/// 1. Make a test that should eventually pass, but use `fail`.
/// 2. Later, implement the feature that test is testing.
/// 3. The test starts to actually pass, so `fail` fails.
/// 4. Update the test to use `check` instead so it actually passes.
///
/// Use `fail` instead of ignoring tests.
#[allow(dead_code)]
#[track_caller]
pub(crate) fn fail(s: &str) {
  go(one_file_fs(s), analysis::StdBasis::Minimal, Outcome::Fail, Severity::Error);
}

/// Like [`check`], but includes the full std basis.
#[track_caller]
pub(crate) fn check_with_std_basis(s: &str) {
  go(one_file_fs(s), analysis::StdBasis::Full, Outcome::Pass, Severity::Error);
}

/// An expected outcome from a test.
#[derive(Debug)]
pub(crate) enum Outcome {
  Pass,
  Fail,
}

/// The low-level impl that almost all top-level functions delegate to.
pub(crate) fn go<'a, I>(
  files: I,
  std_basis: analysis::StdBasis,
  want: Outcome,
  min_severity: Severity,
) where
  I: IntoIterator<Item = (&'a str, &'a str)>,
{
  // ignore the Err if we already initialized logging, since that's fine.
  let (input, store) = input::get(files);
  let input = input.expect("unexpectedly bad input");
  let mut ck = show::Show::new(
    store,
    input.iter_sources().map(|s| {
      let file = expect::File::new(s.val);
      (s.path, file)
    }),
  );
  let want_err_len: usize = ck
    .files
    .values()
    .map(|x| {
      x.iter()
        .filter(|(_, e)| matches!(e.kind, expect::Kind::ErrorExact | expect::Kind::ErrorContains))
        .count()
    })
    .sum();
  // NOTE: we used to emit an error here if want_err_len was not 0 or 1 but no longer. this
  // allows us to write multiple error expectations. e.g. in the diagnostics tests. but note that
  // only one expectation is actually used.
  let mut an = analysis::Analysis::new(
    std_basis,
    config::ErrorLines::One,
    config::DiagnosticsFilter::None,
    false,
    true,
  );
  let err = an
    .get_many(&input)
    .into_iter()
    .flat_map(|(id, errors)| {
      errors.into_iter().filter_map(move |e| (e.severity >= min_severity).then_some((id, e)))
    })
    .next();
  for (&path, file) in &ck.files {
    for (&region, expect) in file.iter() {
      if matches!(expect.kind, expect::Kind::Hover) {
        let pos = match region {
          expect::Region::Exact { line, col_start, .. } => {
            text_pos::Position { line, character: col_start }
          }
          expect::Region::Line(n) => {
            ck.reasons.push(reason::Reason::InexactHover(path.wrap(n)));
            continue;
          }
        };
        let r = match an.get_md(path.wrap(pos), true) {
          None => reason::Reason::NoHover(path.wrap(region)),
          Some((got, _)) => {
            if got.contains(&expect.msg) {
              continue;
            }
            reason::Reason::Mismatched(path.wrap(region), expect.msg.clone(), got)
          }
        };
        ck.reasons.push(r);
      }
    }
  }
  let had_error = match err {
    Some((id, e)) => {
      match reason::get(&ck.files, id, e.range, e.message) {
        Ok(()) => {}
        Err(r) => ck.reasons.push(r),
      }
      true
    }
    None => false,
  };
  if !had_error && want_err_len != 0 {
    ck.reasons.push(reason::Reason::NoErrorsEmitted(want_err_len));
  }
  match (want, ck.reasons.is_empty()) {
    (Outcome::Pass, true) | (Outcome::Fail, false) => {}
    (Outcome::Pass, false) => panic!("UNEXPECTED FAIL: {ck}"),
    (Outcome::Fail, true) => panic!("UNEXPECTED PASS: {ck}"),
  }
}

/// Asserts the input from the files generates an error at the given path containing the given
/// message.
#[track_caller]
pub(crate) fn check_bad_input<'a, I>(path: &str, msg: &str, files: I)
where
  I: IntoIterator<Item = (&'a str, &'a str)>,
{
  let (input, _) = input::get(files);
  let e = input.expect_err("unexpectedly good input");
  let got_path = e.abs_path().strip_prefix(input::ROOT.as_path()).expect("could not strip prefix");
  assert_eq!(std::path::Path::new(path), got_path, "wrong path with errors");
  let got_msg = e.display(input::ROOT.as_path()).to_string();
  assert!(got_msg.contains(msg), "want not contained in got\n  want: {msg}\n  got: {got_msg}");
}

fn one_file_fs(s: &str) -> [(&str, &str); 2] {
  [("file.sml", s), ("sources.mlb", "file.sml")]
}
