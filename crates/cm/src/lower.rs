use crate::types::{CMFile, Class, Error, ErrorKind, ParsedPath, PathKind, Result, Root};
use located::Located;

pub(crate) fn get(root: Root) -> Result<CMFile> {
  match root {
    Root::Alias(path) => Err(Error::new(ErrorKind::UnsupportedAlias, path.range)),
    Root::Desc(_, exports, members) => {
      let mut paths = Vec::<Located<ParsedPath>>::new();
      for member in members {
        // NOTE: just ignore dollar paths, since we include the full basis
        if member
          .pathname
          .val
          .as_os_str()
          .to_string_lossy()
          .starts_with('$')
        {
          continue;
        }
        let kind = match member.class() {
          Some(class) => match class.val {
            Class::Sml => PathKind::Sml,
            Class::Cm => PathKind::Cm,
            c => {
              return Err(Error::new(
                ErrorKind::UnsupportedClass(member.pathname.val, c),
                class.range,
              ))
            }
          },
          None => {
            return Err(Error::new(
              ErrorKind::CouldNotDetermineClass(member.pathname.val),
              member.pathname.range,
            ))
          }
        };
        paths.push(Located {
          val: ParsedPath {
            path: member.pathname.val,
            kind,
          },
          range: member.pathname.range,
        });
      }
      Ok(CMFile { exports, paths })
    }
  }
}
