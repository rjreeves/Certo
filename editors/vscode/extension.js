const { LanguageClient, TransportKind } = require('vscode-languageclient/node');
const vscode = require('vscode');

let client;

function activate(context) {
    const config     = vscode.workspace.getConfiguration('certo');
    const serverPath = config.get('serverPath') || 'certo-lsp';

    const serverOptions = {
        run:   { command: serverPath, transport: TransportKind.stdio },
        debug: { command: serverPath, transport: TransportKind.stdio },
    };

    const clientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'certo' },
        ],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.{certo,cto}'),
        },
    };

    client = new LanguageClient(
        'certo-lsp',
        'Certo Language Server',
        serverOptions,
        clientOptions,
    );

    client.start();
}

function deactivate() {
    return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
