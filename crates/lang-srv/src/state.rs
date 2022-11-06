//! See [`State`].

mod helpers;

use anyhow::{anyhow, bail, Result};
use crossbeam_channel::Sender;
use diagnostic_util::Code;
use fast_hash::FxHashSet;
use lsp_server::{ExtractError, Message, Notification, ReqQueue, Request, RequestId, Response};
use lsp_types::Url;
use std::ops::ControlFlow;

const LEARN_MORE: &str = "Learn more";

/// The state of the language server. Only this may do IO. (Well, also the [`lsp_server`] channels
/// that communicate over stdin and stdout.)
pub(crate) struct State {
  mode: Mode,
  has_diagnostics: FxHashSet<Url>,
  analysis: analysis::Analysis,
  sp: SPState,
}

impl State {
  pub(crate) fn new(init: lsp_types::InitializeParams, sender: Sender<Message>) -> Self {
    let options: config::Options = init
      .initialization_options
      .and_then(|v| match serde_json::from_value(v) {
        Ok(x) => Some(x),
        Err(e) => {
          log::warn!("invalid initialization_options: {e}");
          None
        }
      })
      .unwrap_or_default();
    let analysis = analysis::Analysis::new(
      analysis::StdBasis::Full,
      config::ErrorLines::Many,
      options.diagnostics_filter,
      options.format,
    );
    let mut sp = SPState {
      options,
      registered_for_watched_files: false,
      store: paths::Store::new(),
      file_system: paths::RealFileSystem::default(),
      sender,
      req_queue: ReqQueue::default(),
    };
    let mut root = init
      .root_uri
      .map(|url| helpers::canonical_path_buf(&sp.file_system, &url).map_err(|e| (e, url)))
      .transpose();
    let mut has_diagnostics = FxHashSet::<Url>::default();
    let mut ret = Self {
      // do this convoluted incantation because we need `ret` to show the error in the `Err` case.
      mode: match root.as_mut().ok().and_then(Option::take) {
        Some(path) => {
          let input = sp.try_get_input(&path, &mut has_diagnostics);
          Mode::Root(Root { path, input })
        }
        None => Mode::NoRoot,
      },
      has_diagnostics,
      analysis,
      sp,
    };
    if let Err((e, url)) = root {
      ret.sp.show_error(format!("cannot initialize workspace root {url}: {e:#}"), Code::n(1996));
    }
    let dynamic_registration = init
      .capabilities
      .workspace
      .and_then(|x| x.file_operations?.dynamic_registration)
      .unwrap_or_default();
    if dynamic_registration {
      if let Mode::Root(root) = &ret.mode {
        // not sure if possible to only listen to millet.toml. "nested alternate groups are not
        // allowed" at time of writing
        let glob_pattern =
          format!("{}/**/*.{{sml,sig,fun,cm,mlb,toml}}", root.path.as_path().display());
        let watchers = vec![lsp_types::FileSystemWatcher { glob_pattern, kind: None }];
        let did_changed_registration =
          helpers::registration::<lsp_types::notification::DidChangeWatchedFiles, _>(
            lsp_types::DidChangeWatchedFilesRegistrationOptions { watchers },
          );
        ret.sp.send_request::<lsp_types::request::RegisterCapability>(
          lsp_types::RegistrationParams { registrations: vec![did_changed_registration] },
          None,
        );
        ret.sp.registered_for_watched_files = true;
      };
    }
    ret.try_publish_diagnostics();
    if !ret.sp.registered_for_watched_files {
      log::warn!("millet will not necessarily receive notifications when files change on-disk.");
      log::warn!("this means the internal state of millet can get out of sync with what is");
      log::warn!("actually on disk, e.g. when using `git checkout` or other means of modifying");
      log::warn!("files not via the language client (i.e. the editor millet is attached to).");
    }
    ret
  }

  pub(crate) fn handle_request(&mut self, req: Request) {
    log::info!("got request: {req:?}");
    self.sp.req_queue.incoming.register(req.id.clone(), ());
    match self.handle_request_(req) {
      ControlFlow::Break(Ok(())) => {}
      ControlFlow::Break(Err(e)) => log::error!("couldn't handle request: {e}"),
      ControlFlow::Continue(req) => log::warn!("unhandled request: {req:?}"),
    }
  }

  fn handle_request_(&mut self, mut r: Request) -> ControlFlow<Result<()>, Request> {
    r = try_request::<lsp_types::request::HoverRequest, _>(r, |id, params| {
      let params = params.text_document_position_params;
      let pos = helpers::text_doc_pos_params(&self.sp.file_system, &mut self.sp.store, params)?;
      let res =
        self.analysis.get_md(pos, self.sp.options.show_token_hover).map(|(value, range)| {
          lsp_types::Hover {
            contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
              kind: lsp_types::MarkupKind::Markdown,
              value,
            }),
            range: Some(helpers::lsp_range(range)),
          }
        });
      self.sp.send_response(Response::new_ok(id, res));
      Ok(())
    })?;
    r = try_request::<lsp_types::request::GotoDefinition, _>(r, |id, params| {
      let params = params.text_document_position_params;
      let pos = helpers::text_doc_pos_params(&self.sp.file_system, &mut self.sp.store, params)?;
      let res = self.analysis.get_def(pos).and_then(|range| {
        helpers::lsp_location(&self.sp.store, range).map(lsp_types::GotoDefinitionResponse::Scalar)
      });
      self.sp.send_response(Response::new_ok(id, res));
      Ok(())
    })?;
    r = try_request::<lsp_types::request::GotoTypeDefinition, _>(r, |id, params| {
      let params = params.text_document_position_params;
      let pos = helpers::text_doc_pos_params(&self.sp.file_system, &mut self.sp.store, params)?;
      let locs: Vec<_> = self
        .analysis
        .get_ty_defs(pos)
        .into_iter()
        .flatten()
        .filter_map(|range| helpers::lsp_location(&self.sp.store, range))
        .collect();
      let res = (!locs.is_empty()).then_some(lsp_types::GotoDefinitionResponse::Array(locs));
      self.sp.send_response(Response::new_ok(id, res));
      Ok(())
    })?;
    // TODO do CodeActionResolveRequest and lazily compute the edit, instead of doing everything in
    // CodeActionRequest
    r = try_request::<lsp_types::request::CodeActionRequest, _>(r, |id, params| {
      let url = params.text_document.uri;
      let path = helpers::url_to_path_id(&self.sp.file_system, &mut self.sp.store, &url)?;
      let range = helpers::analysis_range(params.range);
      let mut actions = Vec::<lsp_types::CodeActionOrCommand>::new();
      if let Some((range, new_text)) = self.analysis.fill_case(path.wrap(range.start)) {
        actions.push(helpers::quick_fix("Fill case".to_owned(), url, range, new_text));
      }
      self.sp.send_response(Response::new_ok(id, actions));
      Ok(())
    })?;
    r = try_request::<lsp_types::request::Formatting, _>(r, |id, params| {
      if !self.sp.options.format {
        self.sp.send_response(Response::new_ok(id, None::<()>));
        return Ok(());
      }
      let url = params.text_document.uri;
      let path = helpers::url_to_path_id(&self.sp.file_system, &mut self.sp.store, &url)?;
      self.sp.send_response(Response::new_ok(
        id,
        self.analysis.format(path).ok().map(|(new_text, end)| {
          vec![lsp_types::TextEdit {
            range: lsp_types::Range {
              start: lsp_types::Position { line: 0, character: 0 },
              end: helpers::lsp_position(end),
            },
            new_text,
          }]
        }),
      ));
      Ok(())
    })?;
    ControlFlow::Continue(r)
  }

  pub(crate) fn handle_response(&mut self, res: Response) {
    log::info!("got response: {res:?}");
    let data = match self.sp.req_queue.outgoing.complete(res.id.clone()) {
      Some(x) => x,
      None => {
        log::warn!("received response for non-queued request: {res:?}");
        return;
      }
    };
    let code = match data {
      Some(x) => x,
      None => {
        log::info!("no error code associated with this request");
        return;
      }
    };
    let val = match res.result {
      Some(x) => x,
      None => {
        log::info!("user did not click to look at the error URL");
        return;
      }
    };
    let item = match serde_json::from_value::<lsp_types::MessageActionItem>(val) {
      Ok(x) => x,
      Err(e) => {
        log::error!("registered an error code, but got no message action item: {e}");
        return;
      }
    };
    if item.title != LEARN_MORE {
      log::warn!("unknown item.title: {}", item.title);
      return;
    }
    self.sp.send_request::<lsp_types::request::ShowDocument>(
      lsp_types::ShowDocumentParams {
        uri: helpers::error_url(code),
        external: Some(true),
        take_focus: Some(true),
        selection: None,
      },
      None,
    );
  }

  pub(crate) fn handle_notification(&mut self, notif: Notification) {
    log::info!("got notification: {notif:?}");
    match self.handle_notification_(notif) {
      ControlFlow::Break(Ok(())) => {}
      ControlFlow::Break(Err(e)) => log::error!("couldn't handle notification: {e}"),
      ControlFlow::Continue(notif) => log::warn!("unhandled notification: {notif:?}"),
    }
  }

  fn handle_notification_(&mut self, mut n: Notification) -> ControlFlow<Result<()>, Notification> {
    n = try_notification::<lsp_types::notification::DidChangeWatchedFiles, _>(
      n,
      |_| match &mut self.mode {
        Mode::Root(root) => {
          root.input = self.sp.try_get_input(&root.path, &mut self.has_diagnostics);
          self.try_publish_diagnostics();
          Ok(())
        }
        Mode::NoRoot => bail!("can't handle DidChangeWatchedFiles with no root"),
      },
    )?;
    n = try_notification::<lsp_types::notification::DidChangeTextDocument, _>(n, |params| {
      let url = params.text_document.uri;
      let root = match &mut self.mode {
        Mode::Root(x) => x,
        // TODO get this working. might require keeping the current contents of every open file
        // in the State, so we could feed it into publish_diagnostics_one?
        Mode::NoRoot => bail!("not implemented: DidChangeTextDocument with no root"),
      };
      let input = match &mut root.input {
        Some(x) => x,
        None => bail!("no input for DidChangeTextDocument"),
      };
      let path = helpers::url_to_path_id(&self.sp.file_system, &mut self.sp.store, &url)?;
      let text = match input.get_mut_source(path) {
        Some(x) => x,
        None => bail!("no source in the input for DidChangeTextDocument"),
      };
      apply_changes(text, params.content_changes);
      if self.sp.options.diagnostics_on_change {
        self.try_publish_diagnostics();
      } else if self.sp.options.format {
        // TODO this is expensive, but currently necessary to make formatting work. can we
        // make it just do it for formatting (i.e. syntax) only (no statics)?
        self.analysis.get_many(input);
      }
      Ok(())
    })?;
    n = try_notification::<lsp_types::notification::DidOpenTextDocument, _>(n, |params| {
      if let Mode::NoRoot = self.mode {
        let url = params.text_document.uri;
        let text = params.text_document.text;
        self.publish_diagnostics_one(url, &text);
      }
      Ok(())
    })?;
    n = try_notification::<lsp_types::notification::DidSaveTextDocument, _>(n, |params| {
      match &mut self.mode {
        Mode::Root(root) => {
          if self.sp.registered_for_watched_files {
            log::warn!("ignoring DidSaveTextDocument since we registered for watched file events");
          } else {
            root.input = self.sp.try_get_input(&root.path, &mut self.has_diagnostics);
            self.try_publish_diagnostics();
          }
        }
        Mode::NoRoot => match params.text {
          Some(text) => {
            let url = params.text_document.uri;
            self.publish_diagnostics_one(url, &text);
          }
          None => bail!("no text for DidSaveTextDocument"),
        },
      }
      Ok(())
    })?;
    n = try_notification::<lsp_types::notification::DidCloseTextDocument, _>(n, |params| {
      if let Mode::NoRoot = self.mode {
        let url = params.text_document.uri;
        self.sp.send_diagnostics(url, Vec::new());
      }
      Ok(())
    })?;
    ControlFlow::Continue(n)
  }

  // diagnostics //

  fn try_publish_diagnostics(&mut self) -> bool {
    let root = match &mut self.mode {
      Mode::Root(x) => x,
      Mode::NoRoot => return false,
    };
    let input = match &mut root.input {
      Some(x) => x,
      None => return false,
    };
    let got_many = self.analysis.get_many(input);
    let mut has_diagnostics = FxHashSet::<Url>::default();
    for (path_id, errors) in got_many {
      let path = self.sp.store.get_path(path_id);
      let url = match helpers::file_url(path.as_path()) {
        Ok(x) => x,
        Err(e) => {
          log::error!("couldn't get path as a file url: {e:#}");
          continue;
        }
      };
      let ds = helpers::diagnostics(errors, self.sp.options.diagnostics_more_info_hint);
      if ds.is_empty() {
        continue;
      }
      has_diagnostics.insert(url.clone());
      self.sp.send_diagnostics(url, ds);
    }
    // this is the old one.
    let old_has_diagnostics = std::mem::take(&mut self.has_diagnostics);
    for url in old_has_diagnostics {
      // this is the new one.
      if has_diagnostics.contains(&url) {
        // had old diagnostics, and has new diagnostics. we just sent the new ones.
        continue;
      }
      // had old diagnostics, but no new diagnostics. clear the old diagnostics.
      self.sp.send_diagnostics(url, Vec::new());
    }
    self.has_diagnostics = has_diagnostics;
    true
  }

  fn publish_diagnostics_one(&mut self, url: Url, text: &str) {
    self.sp.send_diagnostics(
      url,
      helpers::diagnostics(self.analysis.get_one(text), self.sp.options.diagnostics_more_info_hint),
    );
  }
}

enum Mode {
  Root(Root),
  NoRoot,
}

struct Root {
  path: paths::CanonicalPathBuf,
  input: Option<analysis::input::Input>,
}

/// Semi-Permanent state. Some things on this are totally immutable after initialization. Other
/// things are mutable, but nothing on this will ever get "replaced" entirely; instead, _if_ it's
/// mutable, _when_ it's mutate, it'll only be "tweaked" a bit.
struct SPState {
  options: config::Options,
  registered_for_watched_files: bool,
  store: paths::Store,
  file_system: paths::RealFileSystem,
  sender: Sender<Message>,
  req_queue: ReqQueue<(), Option<Code>>,
}

impl SPState {
  fn send(&self, msg: Message) {
    log::info!("sending {msg:?}");
    self.sender.send(msg).unwrap()
  }

  fn send_request<R>(&mut self, params: R::Params, data: Option<Code>)
  where
    R: lsp_types::request::Request,
  {
    let req = self.req_queue.outgoing.register(R::METHOD.to_owned(), params, data);
    self.send(req.into())
  }

  fn send_response(&mut self, res: Response) {
    match self.req_queue.incoming.complete(res.id.clone()) {
      Some(()) => self.send(res.into()),
      None => log::warn!("tried to respond to a non-queued request: {res:?}"),
    }
  }

  fn send_notification<N>(&self, params: N::Params)
  where
    N: lsp_types::notification::Notification,
  {
    let notif = Notification::new(N::METHOD.to_owned(), params);
    self.send(notif.into())
  }

  fn send_diagnostics(&mut self, url: Url, diagnostics: Vec<lsp_types::Diagnostic>) {
    self.send_notification::<lsp_types::notification::PublishDiagnostics>(
      lsp_types::PublishDiagnosticsParams { uri: url, diagnostics, version: None },
    );
  }

  fn show_error(&mut self, message: String, code: Code) {
    self.send_request::<lsp_types::request::ShowMessageRequest>(
      lsp_types::ShowMessageRequestParams {
        typ: lsp_types::MessageType::ERROR,
        message,
        actions: Some(vec![lsp_types::MessageActionItem {
          title: LEARN_MORE.to_owned(),
          properties: Default::default(),
        }]),
      },
      Some(code),
    );
  }

  fn try_get_input(
    &mut self,
    root: &paths::CanonicalPathBuf,
    has_diagnostics: &mut FxHashSet<Url>,
  ) -> Option<analysis::input::Input> {
    let input = elapsed::log("Input::new", || {
      analysis::input::Input::new(&self.file_system, &mut self.store, root)
    });
    let err = match input {
      Ok(x) => return Some(x),
      Err(x) => x,
    };
    for url in std::mem::take(has_diagnostics) {
      self.send_diagnostics(url, Vec::new());
    }
    let did_send_as_diagnostic = if err.abs_path().is_file() {
      match helpers::file_url(err.abs_path()) {
        Ok(url) => {
          has_diagnostics.insert(url.clone());
          self.send_diagnostics(
            url,
            vec![helpers::diagnostic(
              err.display(root.as_path()).to_string(),
              err.range(),
              err.code(),
              err.severity(),
              self.options.diagnostics_more_info_hint,
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
      self.show_error(
        format!(
          "{}: {}",
          err.maybe_rel_path(root.as_path()).display(),
          err.display(root.as_path())
        ),
        err.code(),
      );
    }
    None
  }
}

fn try_request<R, F>(req: Request, f: F) -> ControlFlow<Result<()>, Request>
where
  R: lsp_types::request::Request,
  F: FnOnce(RequestId, R::Params) -> Result<()>,
{
  match req.extract::<R::Params>(R::METHOD) {
    Ok((id, params)) => ControlFlow::Break(f(id, params)),
    Err(e) => extract_error(e),
  }
}

fn try_notification<N, F>(notif: Notification, f: F) -> ControlFlow<Result<()>, Notification>
where
  N: lsp_types::notification::Notification,
  F: FnOnce(N::Params) -> Result<()>,
{
  match notif.extract::<N::Params>(N::METHOD) {
    Ok(params) => ControlFlow::Break(f(params)),
    Err(e) => extract_error(e),
  }
}

/// adapted from rust-analyzer.
fn apply_changes(text: &mut String, changes: Vec<lsp_types::TextDocumentContentChangeEvent>) {
  let mut pos_db = text_pos::PositionDb::new(text);
  let mut up_to_line = None::<u32>;
  for change in changes {
    match change.range {
      Some(range) => {
        if up_to_line.map_or(false, |utl| utl <= range.end.line) {
          pos_db = text_pos::PositionDb::new(text);
        }
        match pos_db.text_range(helpers::analysis_range(range)) {
          Some(text_range) => {
            text.replace_range(std::ops::Range::<usize>::from(text_range), &change.text);
            up_to_line = Some(range.start.line);
          }
          None => log::error!("unable to apply text document change {change:?}"),
        }
      }
      None => {
        *text = change.text;
        up_to_line = Some(0);
      }
    }
  }
}

fn extract_error<T>(e: ExtractError<T>) -> ControlFlow<Result<()>, T> {
  match e {
    ExtractError::MethodMismatch(x) => ControlFlow::Continue(x),
    ExtractError::JsonError { method, error } => {
      ControlFlow::Break(Err(anyhow!("couldn't deserialize for {method}: {error}")))
    }
  }
}
