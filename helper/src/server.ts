// Reads requests (one JSON line each), calls the backend and writes responses.
// Requests are handled concurrently; each response carries its request's id.

import type { AccountBackend, DownloadSource, DriveBackend, UploadSink } from './backend';
import { CHUNK_BYTES, HelperError, PROTOCOL_VERSION, type Request, type Response, type WireError } from './protocol';

type Params = Record<string, unknown>;

export class Server {
    private readonly uploads = new Map<string, UploadSink>();
    private readonly downloads = new Map<string, DownloadSource>();
    private nextHandle = 1;

    constructor(
        private readonly backend: DriveBackend,
        private readonly account: AccountBackend | null = null,
        private readonly toWireError: (error: unknown) => WireError = defaultWireError,
    ) {}

    /** Handles one line; returns the response line (without newline) or null for garbage. */
    async handleLine(line: string): Promise<string | null> {
        let request: Request;
        try {
            request = JSON.parse(line) as Request;
        } catch {
            return null;
        }
        if (typeof request?.id !== 'number' || typeof request.method !== 'string') {
            return null;
        }
        let response: Response;
        try {
            const ok = await this.dispatch(request.method, request.params ?? {});
            response = { id: request.id, ok: ok ?? null };
        } catch (error) {
            response = { id: request.id, error: this.toWireError(error) };
        }
        return JSON.stringify(response);
    }

    private async dispatch(method: string, p: Params): Promise<unknown> {
        const b = this.backend;
        switch (method) {
            case 'hello':
                return { protocol: PROTOCOL_VERSION, sdk_version: b.sdkVersion() };
            case 'node':
                return b.node(text(p, 'node'));
            case 'list_children':
                return b.listChildren(text(p, 'folder'));
            case 'create_folder':
                return b.createFolder(text(p, 'parent'), text(p, 'name'));
            case 'upload_open':
                return this.keepUpload(
                    await b.openUpload(text(p, 'parent'), text(p, 'name'), num(p, 'size'), num(p, 'modified')),
                );
            case 'revision_open':
                return this.keepUpload(await b.openRevision(text(p, 'file'), num(p, 'size'), num(p, 'modified')));
            case 'upload_write':
                await this.upload(p).write(fromBase64(text(p, 'data')));
                return null;
            case 'upload_commit': {
                const sink = this.upload(p);
                this.uploads.delete(text(p, 'upload'));
                return sink.commit();
            }
            case 'upload_abort': {
                const sink = this.uploads.get(text(p, 'upload'));
                this.uploads.delete(text(p, 'upload'));
                await sink?.abort();
                return null;
            }
            case 'download_open': {
                const handle = this.handle();
                this.downloads.set(handle, await b.openDownload(text(p, 'file')));
                return { download: handle };
            }
            case 'download_read': {
                const max = Math.min(num(p, 'max'), CHUNK_BYTES);
                const { data, eof } = await this.download(p).read(max);
                return { data: toBase64(data), eof };
            }
            case 'download_close': {
                const source = this.downloads.get(text(p, 'download'));
                this.downloads.delete(text(p, 'download'));
                await source?.close();
                return null;
            }
            case 'download_range': {
                const length = Math.min(num(p, 'length'), CHUNK_BYTES);
                return { data: toBase64(await b.readRange(text(p, 'file'), num(p, 'offset'), length)) };
            }
            case 'rename':
                return b.rename(text(p, 'node'), text(p, 'name'));
            case 'move':
                return b.move(text(p, 'node'), text(p, 'parent'));
            case 'trash':
                await b.trash(text(p, 'node'));
                return null;
            case 'restore':
                return b.restore(text(p, 'node'));
            case 'delete':
                await b.deletePermanently(text(p, 'node'));
                return null;
            case 'events_since':
                return b.eventsSince(text(p, 'root'), typeof p.cursor === 'string' ? p.cursor : null);
            case 'session':
                return this.requireAccount().session();
            case 'login_web_start':
                return this.requireAccount().loginWebStart();
            case 'login_web_wait':
                return this.requireAccount().loginWebWait();
            case 'logout':
                await this.requireAccount().logout();
                return null;
            case 'my_files_root':
                return this.requireAccount().myFilesRoot();
            case 'shared_with_me':
                return this.requireAccount().sharedWithMe();
            default:
                throw new HelperError({ kind: 'rejected', message: `unknown method ${method}` });
        }
    }

    private requireAccount(): AccountBackend {
        if (!this.account) {
            throw new HelperError({ kind: 'rejected', message: 'this helper has no account' });
        }
        return this.account;
    }

    private handle(): string {
        return `h${this.nextHandle++}`;
    }

    private keepUpload(sink: UploadSink): { upload: string } {
        const handle = this.handle();
        this.uploads.set(handle, sink);
        return { upload: handle };
    }

    private upload(p: Params): UploadSink {
        const sink = this.uploads.get(text(p, 'upload'));
        if (!sink) {
            throw new HelperError({ kind: 'transfer', message: 'unknown upload handle' });
        }
        return sink;
    }

    private download(p: Params): DownloadSource {
        const source = this.downloads.get(text(p, 'download'));
        if (!source) {
            throw new HelperError({ kind: 'transfer', message: 'unknown download handle' });
        }
        return source;
    }
}

/** Serves requests from `input` lines and writes each answer through `write`. */
export async function serve(
    server: Server,
    input: AsyncIterable<string>,
    write: (line: string) => void,
): Promise<void> {
    const running = new Set<Promise<void>>();
    for await (const line of input) {
        const task = server.handleLine(line).then((answer) => {
            if (answer !== null) {
                write(answer + '\n');
            }
        });
        running.add(task);
        void task.finally(() => running.delete(task));
    }
    await Promise.allSettled(running);
}

function defaultWireError(error: unknown): WireError {
    if (error instanceof HelperError) {
        return error.wire;
    }
    return { kind: 'unavailable', message: error instanceof Error ? error.message : String(error) };
}

function text(p: Params, key: string): string {
    const value = p[key];
    if (typeof value !== 'string') {
        throw new HelperError({ kind: 'rejected', message: `parameter ${key} must be a string` });
    }
    return value;
}

function num(p: Params, key: string): number {
    const value = p[key];
    if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
        throw new HelperError({ kind: 'rejected', message: `parameter ${key} must be a non-negative number` });
    }
    return value;
}

export function toBase64(bytes: Uint8Array): string {
    return Buffer.from(bytes).toString('base64');
}

export function fromBase64(text: string): Uint8Array {
    return new Uint8Array(Buffer.from(text, 'base64'));
}
