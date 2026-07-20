mod analysis;
mod completion;
mod diagnostics;
mod goto;
mod hover;
mod pos;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use lsp_server::{Connection, Message, Request, RequestId, Response, Notification};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
        PublishDiagnostics, Notification as NotificationTrait,
    },
    request::{
        Completion, GotoDefinition, HoverRequest, Request as RequestTrait,
    },
    CompletionOptions, PublishDiagnosticsParams,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
    Url, WorkDoneProgressOptions,
};
use analysis::Analysis;

struct State {
    docs: HashMap<Url, Analysis>,
}

impl State {
    fn new() -> Self { State { docs: HashMap::new() } }

    fn update(&mut self, uri: Url, text: String) -> &Analysis {
        self.docs.insert(uri.clone(), Analysis::run(&text));
        &self.docs[&uri]
    }

    fn get(&self, uri: &Url) -> Option<&Analysis> {
        self.docs.get(uri)
    }

    fn remove(&mut self, uri: &Url) {
        self.docs.remove(uri);
    }
}

fn main() {
    let (connection, io_threads) = Connection::stdio();

    // Handshake
    let server_caps = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string()]),
            work_done_progress_options: WorkDoneProgressOptions::default(),
            ..Default::default()
        }),
        ..Default::default()
    }).unwrap();

    let (init_id, _init_params) = connection.initialize_start().unwrap();
    connection.initialize_finish(
        init_id,
        serde_json::json!({
            "capabilities": server_caps,
            "serverInfo": { "name": "certo-lsp", "version": "0.1.0" }
        }),
    ).unwrap();

    let mut state = State::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req).unwrap() {
                    break;
                }
                handle_request(&connection, req, &mut state);
            }
            Message::Notification(not) => {
                handle_notification(&connection, not, &mut state);
            }
            Message::Response(_) => {}
        }
    }

    io_threads.join().unwrap();
}

fn handle_request(conn: &Connection, req: Request, state: &mut State) {
    match req.method.as_str() {
        HoverRequest::METHOD => {
            let (id, params) = cast_req::<HoverRequest>(req);
            let uri = &params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            let result = state.get(uri)
                .and_then(|a| hover::handle_hover(a, pos))
                .map(|h| serde_json::to_value(h).unwrap())
                .unwrap_or(serde_json::Value::Null);
            send_response(conn, id, result);
        }
        GotoDefinition::METHOD => {
            let (id, params) = cast_req::<GotoDefinition>(req);
            let uri = &params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            let result = state.get(uri)
                .and_then(|a| goto::handle_goto_definition(a, uri, pos))
                .map(|g| serde_json::to_value(g).unwrap())
                .unwrap_or(serde_json::Value::Null);
            send_response(conn, id, result);
        }
        Completion::METHOD => {
            let (id, params) = cast_req::<Completion>(req);
            let uri = &params.text_document_position.text_document.uri;
            let pos = params.text_document_position.position;
            let result = state.get(uri)
                .map(|a| completion::handle_completion(a, pos))
                .map(|c| serde_json::to_value(c).unwrap())
                .unwrap_or(serde_json::Value::Null);
            send_response(conn, id, result);
        }
        _ => {
            // Send method-not-found for unknown requests
            let resp = Response::new_err(
                req.id,
                lsp_server::ErrorCode::MethodNotFound as i32,
                format!("method not supported: {}", req.method),
            );
            conn.sender.send(Message::Response(resp)).unwrap();
        }
    }
}

fn handle_notification(conn: &Connection, not: Notification, state: &mut State) {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: lsp_types::DidOpenTextDocumentParams =
                serde_json::from_value(not.params).unwrap();
            let uri  = params.text_document.uri;
            let text = params.text_document.text;
            let analysis = state.update(uri.clone(), text);
            publish_diagnostics(conn, &uri, analysis);
        }
        DidChangeTextDocument::METHOD => {
            let params: lsp_types::DidChangeTextDocumentParams =
                serde_json::from_value(not.params).unwrap();
            let uri = params.text_document.uri;
            if let Some(change) = params.content_changes.into_iter().last() {
                let analysis = state.update(uri.clone(), change.text);
                publish_diagnostics(conn, &uri, analysis);
            }
        }
        DidCloseTextDocument::METHOD => {
            let params: lsp_types::DidCloseTextDocumentParams =
                serde_json::from_value(not.params).unwrap();
            state.remove(&params.text_document.uri);
        }
        _ => {} // ignore unknown notifications
    }
}

fn publish_diagnostics(conn: &Connection, uri: &Url, analysis: &Analysis) {
    let diags = diagnostics::collect_diagnostics(analysis);
    let params = PublishDiagnosticsParams {
        uri:         uri.clone(),
        diagnostics: diags,
        version:     None,
    };
    let not = lsp_server::Notification::new(
        PublishDiagnostics::METHOD.to_string(),
        params,
    );
    conn.sender.send(Message::Notification(not)).unwrap();
}

fn send_response(conn: &Connection, id: RequestId, result: serde_json::Value) {
    let resp = Response { id, result: Some(result), error: None };
    conn.sender.send(Message::Response(resp)).unwrap();
}

fn cast_req<R: RequestTrait>(req: Request) -> (RequestId, R::Params) {
    req.extract(R::METHOD).unwrap()
}
