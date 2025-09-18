import * as vscode from 'vscode';
import { ChildProcess, spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

let outputChannel: vscode.OutputChannel;
let panel: vscode.WebviewPanel | undefined;
let journalWatcher: fs.FSWatcher | undefined;
let mcpdClient: McpdClient | undefined;
let workspaceRoot: string | undefined;

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('DevIt');

    workspaceRoot = resolveWorkspaceRoot();
    if (!workspaceRoot) {
        outputChannel.appendLine('DevIt: no workspace folder detected; extension idle.');
    } else {
        try {
            const mcpdBin = resolveBinary('devit-mcpd', workspaceRoot, process.env.DEVIT_MCPD_BIN);
            mcpdClient = new McpdClient(mcpdBin, workspaceRoot, outputChannel);
            mcpdClient
                .ping()
                .then(() => outputChannel.appendLine('DevIt MCPD: ready'))
                .catch((err) => outputChannel.appendLine(`DevIt MCPD ping failed: ${err.message}`));
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            outputChannel.appendLine(`DevIt MCPD start failed: ${message}`);
            vscode.window.showWarningMessage(`DevIt MCPD failed to start: ${message}`);
        }
    }

    const showPanelDisposable = vscode.commands.registerCommand('devit.showPanel', () => {
        if (!workspaceRoot) {
            vscode.window.showInformationMessage('DevIt: open a workspace to view the timeline.');
            return;
        }
        ensurePanel(workspaceRoot, context.extensionUri);
    });

    const approveDisposable = vscode.commands.registerCommand('devit.approveLastRequest', async () => {
        if (!workspaceRoot) {
            vscode.window.showInformationMessage('DevIt: open a workspace to approve requests.');
            return;
        }
        const approval = findLastApprovalRequest(workspaceRoot);
        if (!approval) {
            vscode.window.showInformationMessage('DevIt: no approval-required event found in journal.');
            return;
        }
        if (!mcpdClient) {
            vscode.window.showErrorMessage('DevIt: devit-mcpd is not running.');
            return;
        }
        try {
            const response = await mcpdClient.callServerApprove(approval);
            outputChannel.appendLine(`server.approve → ${JSON.stringify(response)}`);
            vscode.window.showInformationMessage('DevIt: approval sent.');
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            outputChannel.appendLine(`server.approve failed: ${message}`);
            vscode.window.showErrorMessage(`DevIt: approval failed (${message}).`);
        }
    });

    const runRecipeDisposable = vscode.commands.registerCommand('devit.runRecipe', async () => {
        if (!workspaceRoot) {
            vscode.window.showInformationMessage('DevIt: open a workspace to run a recipe.');
            return;
        }
        try {
            const recipes = await listRecipes(workspaceRoot);
            if (!recipes.length) {
                vscode.window.showInformationMessage('DevIt: no recipes discovered.');
                return;
            }
            const picked = await vscode.window.showQuickPick(
                recipes.map((r) => ({
                    label: r.name,
                    description: r.id,
                    detail: r.description ?? '',
                })),
                { placeHolder: 'Select a recipe' }
            );
            if (!picked) {
                return;
            }
            const id = picked.description ?? picked.label;
            await runRecipeById(workspaceRoot, id);
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            outputChannel.appendLine(`Recipe run failed: ${message}`);
            vscode.window.showErrorMessage(`DevIt: recipe run failed (${message}).`);
        }
    });

    const runRecipeDirectDisposable = vscode.commands.registerCommand(
        'devit.runRecipeId',
        async (recipeId: string) => {
            if (!workspaceRoot) {
                vscode.window.showInformationMessage('DevIt: open a workspace to run a recipe.');
                return;
            }
            try {
                await runRecipeById(workspaceRoot, recipeId);
            } catch (err) {
                const message = err instanceof Error ? err.message : String(err);
                outputChannel.appendLine(`Recipe run failed: ${message}`);
                vscode.window.showErrorMessage(`DevIt: recipe run failed (${message}).`);
            }
        }
    );

    if (workspaceRoot) {
        registerRecipeCodeActions(context, workspaceRoot);
    }

    context.subscriptions.push(
        showPanelDisposable,
        approveDisposable,
        runRecipeDisposable,
        runRecipeDirectDisposable
    );

    context.subscriptions.push({
        dispose: () => {
            journalWatcher?.close();
            mcpdClient?.dispose();
        },
    });
}

export function deactivate() {
    journalWatcher?.close();
    journalWatcher = undefined;
    mcpdClient?.dispose();
    mcpdClient = undefined;
    panel?.dispose();
    panel = undefined;
    workspaceRoot = undefined;
}

function resolveWorkspaceRoot(): string | undefined {
    const folders = vscode.workspace.workspaceFolders;
    if (!folders || folders.length === 0) {
        return undefined;
    }
    return folders[0].uri.fsPath;
}

function ensurePanel(workspaceRoot: string, extensionUri: vscode.Uri) {
    if (!panel) {
        panel = vscode.window.createWebviewPanel(
            'devitPanel',
            'DevIt Panel',
            vscode.ViewColumn.Two,
            { enableScripts: false }
        );
        panel.onDidDispose(() => {
            panel = undefined;
            journalWatcher?.close();
            journalWatcher = undefined;
        });
    }
    updatePanel(workspaceRoot);
    if (!journalWatcher) {
        const journalPath = path.join(workspaceRoot, '.devit', 'journal.jsonl');
        try {
            journalWatcher = fs.watch(journalPath, { persistent: false }, () => updatePanel(workspaceRoot));
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            outputChannel.appendLine(`DevIt: cannot watch journal (${message}).`);
        }
    }
}

function updatePanel(workspaceRoot: string) {
    if (!panel) {
        return;
    }
    const events = readJournalEvents(workspaceRoot, 10);
    const html = renderPanelHtml(events);
    panel.webview.html = html;
}

type FormattedEvent = {
    raw: string;
    summary: string;
};

function readJournalEvents(workspaceRoot: string, limit: number): FormattedEvent[] {
    const journalPath = path.join(workspaceRoot, '.devit', 'journal.jsonl');
    if (!fs.existsSync(journalPath)) {
        return [];
    }
    try {
        const data = fs.readFileSync(journalPath, 'utf8');
        const lines = data.split(/\r?\n/).filter((line) => line.trim().length > 0);
        const tail = lines.slice(-limit);
        return tail
            .map((line) => {
                try {
                    const parsed = JSON.parse(line);
                    return { raw: line, summary: summariseEvent(parsed) };
                } catch (err) {
                    return { raw: line, summary: line.slice(0, 120) };
                }
            })
            .reverse();
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`DevIt: failed to read journal (${message}).`);
        return [];
    }
}

function summariseEvent(obj: any): string {
    if (obj?.type && obj?.payload) {
        if (obj.payload.approval_required) {
            const tool = obj.payload.tool ?? 'unknown';
            const phase = obj.payload.phase ?? 'phase';
            return `⚠️ approval: ${tool} (${phase})`;
        }
        if (obj.type === 'tool.result') {
            const tool = obj.payload.name ?? 'tool';
            return `✅ ${tool}`;
        }
        if (obj.type === 'tool.error') {
            return `❌ ${obj.payload.reason ?? 'error'}`;
        }
    }
    if (obj?.tool && obj?.phase) {
        const status = obj.ok === false ? '❌' : '✅';
        return `${status} ${obj.tool} (${obj.phase})`;
    }
    if (obj?.event) {
        return `📝 ${JSON.stringify(obj.event)}`;
    }
    return JSON.stringify(obj);
}

function renderPanelHtml(events: FormattedEvent[]): string {
    const items = events
        .map(
            (evt) =>
                `<li><div class="item-summary">${escapeHtml(evt.summary)}</div><pre>${escapeHtml(evt.raw)}</pre></li>`
        )
        .join('\n');
    return `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8" />
    <style>
        body { font-family: var(--vscode-font-family); padding: 0 16px; }
        h1 { font-size: 1.2rem; }
        ul { list-style: none; padding-left: 0; }
        li { margin-bottom: 12px; border-bottom: 1px solid rgba(255,255,255,0.1); padding-bottom: 8px; }
        .item-summary { font-weight: bold; }
        pre { white-space: pre-wrap; word-break: break-word; background: rgba(255,255,255,0.04); padding: 8px; border-radius: 4px; }
    </style>
</head>
<body>
    <h1>DevIt Timeline</h1>
    <ul>
        ${items || '<li>No events.</li>'}
    </ul>
</body>
</html>`;
}

function escapeHtml(input: string): string {
    return input
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#039;');
}

interface RecipeSummary {
    id: string;
    name: string;
    description?: string;
}

async function listRecipes(workspaceRoot: string): Promise<RecipeSummary[]> {
    const bin = resolveBinary('devit', workspaceRoot, process.env.DEVIT_BIN);
    const result = await runProcess(bin, ['recipe', 'list'], workspaceRoot);
    const stdout = result.stdout.trim();
    if (!stdout) {
        return [];
    }
    try {
        const parsed = JSON.parse(stdout);
        return parsed.recipes ?? [];
    } catch (err) {
        throw new Error('Unexpected JSON from devit recipe list');
    }
}

async function runRecipeDryRun(workspaceRoot: string, id: string): Promise<string> {
    const bin = resolveBinary('devit', workspaceRoot, process.env.DEVIT_BIN);
    const result = await runProcess(bin, ['recipe', 'run', id, '--dry-run'], workspaceRoot);
    return result.stdout.trim() || result.stderr.trim();
}

async function runRecipeById(workspaceRoot: string, id: string): Promise<void> {
    const run = await runRecipeDryRun(workspaceRoot, id);
    outputChannel.appendLine(`devit recipe run ${id} --dry-run → ${run}`);
    vscode.window.showInformationMessage(`DevIt: dry-run for ${id} completed.`);
}

function findLastApprovalRequest(workspaceRoot: string): ApprovalRequest | undefined {
    const journalPath = path.join(workspaceRoot, '.devit', 'journal.jsonl');
    if (!fs.existsSync(journalPath)) {
        return undefined;
    }
    try {
        const data = fs.readFileSync(journalPath, 'utf8');
        const lines = data.split(/\r?\n/).filter((line) => line.trim().length > 0);
        for (let i = lines.length - 1; i >= 0; i -= 1) {
            const line = lines[i];
            try {
                const parsed = JSON.parse(line);
                const payload = parsed?.payload;
                if (payload?.approval_required) {
                    const tool = payload.tool ?? 'unknown';
                    const pluginId = payload.plugin_id as string | undefined;
                    const reason = payload.reason as string | undefined;
                    return { tool, pluginId, reason };
                }
            } catch (err) {
                continue;
            }
        }
    } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        outputChannel.appendLine(`DevIt: failed to scan journal for approvals (${message}).`);
    }
    return undefined;
}

interface ApprovalRequest {
    tool: string;
    pluginId?: string;
    reason?: string;
}

async function runProcess(
    bin: string,
    args: string[],
    cwd: string
): Promise<{ stdout: string; stderr: string; code: number }> {
    return new Promise((resolve, reject) => {
        const proc = spawn(bin, args, { cwd, shell: false });
        let stdout = '';
        let stderr = '';
        proc.stdout.on('data', (chunk) => {
            stdout += chunk.toString();
        });
        proc.stderr.on('data', (chunk) => {
            stderr += chunk.toString();
        });
        proc.on('error', (err) => reject(err));
        proc.on('close', (code) => {
            if (code === 0) {
                resolve({ stdout, stderr, code: code ?? 0 });
            } else {
                const err = new Error(stderr.trim() || `Process exited with code ${code}`);
                (err as any).stdout = stdout;
                (err as any).stderr = stderr;
                reject(err);
            }
        });
    });
}

function resolveBinary(name: string, workspaceRoot: string, override?: string): string {
    const candidates: string[] = [];
    const ext = process.platform === 'win32' ? '.exe' : '';
    if (override && override.trim()) {
        candidates.push(override.trim());
    }
    const envVar = process.env[`${name.toUpperCase().replace(/[-.]/g, '_')}_BIN`];
    if (envVar) {
        candidates.push(envVar);
    }
    candidates.push(path.join(workspaceRoot, 'target', 'debug', `${name}${ext}`));
    candidates.push(path.join(workspaceRoot, 'target', 'release', `${name}${ext}`));
    candidates.push(`${name}${ext}`);
    for (const candidate of candidates) {
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }
    // Last resort: rely on PATH
    return candidates[candidates.length - 1];
}

function registerRecipeCodeActions(
    context: vscode.ExtensionContext,
    root: string
): void {
    const provider = new RecipeCodeActionProvider(root);
    const selector: vscode.DocumentSelector = [
        { language: 'rust', scheme: 'file' },
        { language: 'json', scheme: 'file' },
        { language: 'jsonc', scheme: 'file' },
        { language: 'javascript', scheme: 'file' },
        { language: 'javascriptreact', scheme: 'file' },
        { language: 'typescript', scheme: 'file' },
        { language: 'typescriptreact', scheme: 'file' }
    ];
    const metadata: vscode.CodeActionProviderMetadata = {
        providedCodeActionKinds: [vscode.CodeActionKind.QuickFix],
    };
    context.subscriptions.push(
        vscode.languages.registerCodeActionsProvider(selector, provider, metadata)
    );
}

class McpdClient {
    private readonly proc: ChildProcess;
    private readonly pending: Array<{ resolve: (value: any) => void; reject: (err: Error) => void }> = [];
    private buffer = '';

    constructor(binary: string, cwd: string, out: vscode.OutputChannel) {
        this.proc = spawn(binary, ['--json-only'], { cwd, stdio: ['pipe', 'pipe', 'pipe'] });
        this.proc.stdout?.on('data', (chunk: Buffer) => this.handleStdout(chunk.toString(), out));
        this.proc.stderr?.on('data', (chunk: Buffer) => {
            out.appendLine(`[devit-mcpd] ${chunk.toString().trim()}`);
        });
        this.proc.on('exit', (code) => {
            out.appendLine(`devit-mcpd exited (${code ?? 'unknown'})`);
            while (this.pending.length) {
                const pending = this.pending.shift();
                if (pending) {
                    pending.reject(new Error('devit-mcpd terminated'));
                }
            }
        });
    }

    dispose() {
        if (!this.proc.killed) {
            this.proc.kill();
        }
    }

    async ping(): Promise<void> {
        const response = await this.send({ type: 'ping' });
        if (response?.type !== 'pong') {
            throw new Error('Unexpected ping response');
        }
    }

    async callServerApprove(request: ApprovalRequest): Promise<any> {
        const payload: any = {
            type: 'tool.call',
            payload: {
                name: 'server.approve',
                args: {
                    name: request.tool,
                    scope: 'once',
                },
            },
        };
        if (request.pluginId) {
            payload.payload.args.plugin_id = request.pluginId;
        }
        if (request.reason) {
            payload.payload.args.reason = request.reason;
        }
        const response = await this.send(payload);
        if (!response) {
            throw new Error('Empty response from server.approve');
        }
        if (response.type === 'tool.error') {
            throw new Error('server.approve rejected');
        }
        return response;
    }

    private handleStdout(chunk: string, out: vscode.OutputChannel) {
        this.buffer += chunk;
        let idx = this.buffer.indexOf('\n');
        while (idx !== -1) {
            const line = this.buffer.slice(0, idx).trim();
            this.buffer = this.buffer.slice(idx + 1);
            if (line.length > 0) {
                try {
                    const parsed = JSON.parse(line);
                    const pending = this.pending.shift();
                    if (pending) {
                        pending.resolve(parsed);
                    } else {
                        out.appendLine(`[devit-mcpd] ${line}`);
                    }
                } catch (err) {
                    out.appendLine(`[devit-mcpd] invalid json: ${line}`);
                }
            }
            idx = this.buffer.indexOf('\n');
        }
    }

    private send(message: any): Promise<any> {
        return new Promise((resolve, reject) => {
            if (!this.proc.stdin) {
                reject(new Error('devit-mcpd stdin unavailable'));
                return;
            }
            this.pending.push({ resolve, reject });
            const payload = `${JSON.stringify(message)}\n`;
            this.proc.stdin.write(payload, (err) => {
                if (err) {
                    this.pending.pop();
                    reject(err);
                }
            });
        });
    }
}

class RecipeCodeActionProvider implements vscode.CodeActionProvider {
    constructor(private readonly root: string) {}

    provideCodeActions(
        document: vscode.TextDocument
    ): vscode.ProviderResult<vscode.CodeAction[]> {
        const actions: vscode.CodeAction[] = [];
        const filePath = document.fileName;
        const fileName = path.basename(filePath).toLowerCase();
        const relPath = path.relative(this.root, filePath).toLowerCase();

        if (fileName === 'cargo.toml') {
            actions.push(this.createRecipeAction('Run recipe: add-ci', 'add-ci'));
        }

        if (this.matchesJest(relPath, fileName)) {
            actions.push(
                this.createRecipeAction('Run recipe: migrate-jest-vitest', 'migrate-jest-vitest')
            );
        }

        return actions;
    }

    private matchesJest(relPath: string, fileName: string): boolean {
        if (fileName.startsWith('jest')) {
            return true;
        }
        return relPath.includes('jest');
    }

    private createRecipeAction(title: string, recipeId: string): vscode.CodeAction {
        const action = new vscode.CodeAction(title, vscode.CodeActionKind.QuickFix);
        action.command = {
            command: 'devit.runRecipeId',
            title,
            arguments: [recipeId],
        };
        action.isPreferred = true;
        return action;
    }
}
