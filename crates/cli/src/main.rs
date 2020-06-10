//! A CLI for millet.

mod args;
mod reporter;

use millet_core::source::SourceMap;
use millet_core::{lex, parse};

fn run() -> bool {
  let args = args::get();
  let stdout = std::io::stdout();
  let mut m = SourceMap::new();
  let mut w = reporter::Reporter::new(stdout.lock());
  for name in args.files {
    match std::fs::read_to_string(&name) {
      Ok(s) => m.insert(name, s),
      Err(e) => {
        w.report_io(&name, e).unwrap();
        return false;
      }
    }
  }
  for (id, file) in m.iter() {
    match parse::get(lex::get(id, file.as_bytes())) {
      Ok(xs) => eprintln!("parsed: {:#?}", xs),
      Err(e) => {
        w.report(&m, id, e).unwrap();
        return false;
      }
    }
  }
  true
}

fn main() {
  if !run() {
    std::process::exit(1);
  }
  println!("OK");
}
