//! Overloaded types.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Basic {
  Int,
  Real,
  Word,
  String,
  Char,
}

impl Basic {
  pub(crate) fn as_str(self) -> &'static str {
    match self {
      Basic::Int => "int",
      Basic::Real => "real",
      Basic::Word => "word",
      Basic::String => "string",
      Basic::Char => "char",
    }
  }
}

impl fmt::Display for Basic {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.as_str())
  }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Composite {
  WordInt,
  RealInt,
  Num,
  NumTxt,
  /// equality-only subset of NumTxt. used only for unification
  NumTxtEq,
}

impl Composite {
  pub(crate) fn as_basics(self) -> &'static [Basic] {
    match self {
      Self::WordInt => &[Basic::Word, Basic::Int],
      Self::RealInt => &[Basic::Real, Basic::Int],
      Self::Num => &[Basic::Word, Basic::Real, Basic::Int],
      Self::NumTxt => &[Basic::Word, Basic::Real, Basic::Int, Basic::String, Basic::Char],
      Self::NumTxtEq => &[Basic::Word, Basic::Int, Basic::String, Basic::Char],
    }
  }

  /// we could also probably use `as_basics`, set intersection, then a "reverse" `as_basics` here.
  pub(crate) fn unify(self, other: Self) -> Overload {
    match (self, other) {
      (Self::WordInt, Self::WordInt | Self::Num | Self::NumTxt)
      | (Self::Num | Self::NumTxt, Self::WordInt) => Overload::Composite(Self::WordInt),
      (Self::WordInt | Self::NumTxtEq, Self::RealInt)
      | (Self::RealInt, Self::WordInt | Self::NumTxtEq) => Overload::Basic(Basic::Int),
      (Self::RealInt, Self::RealInt | Self::Num | Self::NumTxt)
      | (Self::Num | Self::NumTxt, Self::RealInt) => Overload::Composite(Self::RealInt),
      (Self::Num, Self::Num | Self::NumTxt) | (Self::NumTxt, Self::Num) => {
        Overload::Composite(Self::Num)
      }
      (Self::NumTxt, Self::NumTxt) => Overload::Composite(Self::NumTxt),
      (Self::NumTxtEq, Self::NumTxtEq | Self::NumTxt) | (Self::NumTxt, Self::NumTxtEq) => {
        Overload::Composite(Self::NumTxtEq)
      }
      (Self::NumTxtEq, Self::WordInt | Self::Num) | (Self::WordInt | Self::Num, Self::NumTxtEq) => {
        Overload::Composite(Self::WordInt)
      }
    }
  }
}

impl fmt::Display for Composite {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Composite::WordInt => f.write_str("<wordint>"),
      Composite::RealInt => f.write_str("<realint>"),
      Composite::Num => f.write_str("<num>"),
      Composite::NumTxt => f.write_str("<numtxt>"),
      Composite::NumTxtEq => f.write_str("<numtxteq>"),
    }
  }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Overload {
  Basic(Basic),
  Composite(Composite),
}

impl Overload {
  pub(crate) fn as_basics(&self) -> &[Basic] {
    match self {
      Overload::Basic(b) => std::slice::from_ref(b),
      Overload::Composite(c) => c.as_basics(),
    }
  }

  /// returns `None` iff the overloads could not be unified.
  pub(crate) fn unify(self, other: Self) -> Option<Self> {
    match (self, other) {
      (Self::Basic(b1), Self::Basic(b2)) => {
        if b1 == b2 {
          Some(Self::Basic(b1))
        } else {
          None
        }
      }
      (Self::Basic(b), Self::Composite(c)) | (Self::Composite(c), Self::Basic(b)) => {
        c.as_basics().iter().find(|&&x| x == b).copied().map(Self::Basic)
      }
      (Self::Composite(c1), Self::Composite(c2)) => Some(c1.unify(c2)),
    }
  }
}

impl fmt::Display for Overload {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Overload::Basic(b) => b.fmt(f),
      Overload::Composite(c) => c.fmt(f),
    }
  }
}
