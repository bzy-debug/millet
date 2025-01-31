//! Tests to make sure Millet behaves as expected on the public documentation.

use crate::{check::raw, repo::root_dir};
use diagnostic::Severity;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

const SML: &str = "sml";

fn opts(limit: raw::Limit) -> raw::Opts<'static> {
  raw::Opts {
    std_basis: raw::StdBasis::Full,
    outcome: raw::Outcome::Pass,
    limit,
    min_severity: Severity::Warning,
    expected_input: raw::ExpectedInput::Good,
  }
}

fn check_all(contents: &str) {
  let mut options = Options::empty();
  options.insert(Options::ENABLE_TABLES);
  let parser = Parser::new_ext(contents, options);
  let mut inside = false;
  let mut ignore_next = false;
  let mut limit = raw::Limit::None;
  let mut ac = String::new();
  for ev in parser {
    match ev {
      Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
        if lang.as_ref() == SML {
          inside = true;
        }
      }
      Event::End(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
        if lang.as_ref() == SML {
          if !ignore_next {
            raw::get(raw::one_file_fs(ac.as_ref()), opts(limit));
          }
          ac.clear();
          inside = false;
          ignore_next = false;
          limit = raw::Limit::None;
        }
      }
      Event::Text(s) => {
        if inside {
          ac.push_str(s.as_ref());
        }
      }
      Event::Html(s) => {
        let s = s.trim();
        if s.strip_prefix("<!-- @ignore ").and_then(|s| s.strip_suffix(" -->")).is_some() {
          ignore_next = true;
        } else if s == "<!-- @limit first -->" {
          limit = raw::Limit::First;
        }
      }
      _ => {}
    }
  }
}

#[test]
fn diagnostics() {
  for entry in std::fs::read_dir(root_dir().join("docs").join("diagnostics")).unwrap() {
    let entry = entry.unwrap();
    let path = entry.path();
    let contents = std::fs::read_to_string(&path).unwrap();
    check_all(&contents);
  }
}

#[test]
fn primitives() {
  check_all(include_str!("../../../docs/primitives.md"));
}

#[test]
fn tokens() {
  check_all(include_str!("../../../docs/tokens.md"));
}
