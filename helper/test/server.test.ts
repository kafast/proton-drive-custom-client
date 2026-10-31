// Tests of the protocol server against an in-memory backend.
// Run with `bun test` inside `helper/` (needs no SDK and no account).

import { describe, expect, test } from 'bun:test';

import type { DownloadSource, DriveBackend, UploadSink } from '../src/backend';
import { HelperError, type WireNode } from '../src/protocol';
import { fromBase64, serve, Server, toBase64 } from '../src/server';

class MemoryBackend implements DriveBackend {
    files = new Map<string, Uint8Array>();
    aborted = 0;
    slowNode = false;

    sdkVersion() {
        return 'memory';
    }
    async node(id: string): Promise<WireNode> {
        if (this.slowNode) {
            await new Promise((resolve) => setTimeout(resolve, 20));
        }
        if (id === 'root') {
            return { id, name: 'root', type: 'folder' };
        }
        const content = this.files.get(id);
        if (!content) {
            throw new HelperError({ kind: 'not_found', node: id });
        }
        return { id, parent: 'root', name: id, type: 'file', revision: 'r', size: content.length, modified: 1 };
    }
    async listChildren() {
        return [];
    }
    async createFolder(parent: string, name: string): Promise<WireNode> {
        return { id: name, parent, name, type: 'folder' };
    }
    async openUpload(_parent: string, name: string, size: number): Promise<UploadSink> {
        const parts: Uint8Array[] = [];
        return {
            write: async (data) => {
                parts.push(data);
            },
            commit: async () => {
                const content = Buffer.concat(parts);
                if (content.length !== size) {
                    throw new HelperError({ kind: 'transfer', message: 'size' });
                }
                this.files.set(name, new Uint8Array(content));
                return this.node(name);
            },
            abort: async () => {
                this.aborted++;
            },
        };
    }
    async openRevision(file: string, size: number, modified: number) {
        return this.openUpload('root', file, size, modified);
    }
    async openDownload(file: string): Promise<DownloadSource> {
        const content = this.files.get(file) ?? new Uint8Array();
        let position = 0;
        return {
            read: async (max) => {
                const data = content.slice(position, position + Math.min(max, 2));
                position += data.length;
                return { data, eof: position >= content.length };
            },
            close: async () => {},
        };
    }
    async readRange(file: string, offset: number, length: number) {
        return (this.files.get(file) ?? new Uint8Array()).slice(offset, offset + length);
    }
    rename = this.createFolder;
    move = this.createFolder;
    async trash() {}
    restore = this.node;
    async deletePermanently() {}
    async eventsSince(_root: string, cursor: string | null) {
        return { events: cursor ? [{ type: 'gap' as const }] : [], cursor: 'c1' };
    }
}

async function call(server: Server, id: number, method: string, params: Record<string, unknown> = {}) {
    const line = await server.handleLine(JSON.stringify({ id, method, params }));
    return JSON.parse(line ?? 'null');
}

describe('server', () => {
    test('hello names protocol and SDK version', async () => {
        const server = new Server(new MemoryBackend());
        expect(await call(server, 1, 'hello', { protocol: 1 })).toEqual({
            id: 1,
            ok: { protocol: 1, sdk_version: 'memory' },
        });
    });

    test('errors keep their kind', async () => {
        const server = new Server(new MemoryBackend());
        expect(await call(server, 2, 'node', { node: 'x' })).toEqual({
            id: 2,
            error: { kind: 'not_found', node: 'x' },
        });
        expect((await call(server, 3, 'nope')).error.kind).toBe('rejected');
        expect((await call(server, 4, 'node', {})).error.kind).toBe('rejected');
    });

    test('account methods need an account backend', async () => {
        const server = new Server(new MemoryBackend());
        expect((await call(server, 1, 'session')).error.kind).toBe('rejected');
    });

    test('foreign lines are ignored', async () => {
        const server = new Server(new MemoryBackend());
        expect(await server.handleLine('not json')).toBeNull();
        expect(await server.handleLine('{"method":"hello"}')).toBeNull();
    });

    test('upload in chunks, then download in pieces', async () => {
        const backend = new MemoryBackend();
        const server = new Server(backend);
        const { ok: open } = await call(server, 1, 'upload_open', { parent: 'root', name: 'f', size: 5, modified: 1 });
        await call(server, 2, 'upload_write', { upload: open.upload, data: toBase64(new TextEncoder().encode('hel')) });
        await call(server, 3, 'upload_write', { upload: open.upload, data: toBase64(new TextEncoder().encode('lo')) });
        expect((await call(server, 4, 'upload_commit', { upload: open.upload })).ok.size).toBe(5);

        const { ok: download } = await call(server, 5, 'download_open', { file: 'f' });
        let text = '';
        for (let id = 6; ; id++) {
            const { ok } = await call(server, id, 'download_read', { download: download.download, max: 1024 });
            text += new TextDecoder().decode(fromBase64(ok.data));
            if (ok.eof) {
                break;
            }
        }
        expect(text).toBe('hello');
        expect((await call(server, 20, 'download_range', { file: 'f', offset: 1, length: 3 })).ok.data).toBe(
            toBase64(new TextEncoder().encode('ell')),
        );
    });

    test('abort forgets the upload and a wrong size fails', async () => {
        const backend = new MemoryBackend();
        const server = new Server(backend);
        const { ok: first } = await call(server, 1, 'upload_open', { parent: 'root', name: 'a', size: 1, modified: 1 });
        await call(server, 2, 'upload_abort', { upload: first.upload });
        expect(backend.aborted).toBe(1);
        expect((await call(server, 3, 'upload_write', { upload: first.upload, data: '' })).error.kind).toBe('transfer');

        const { ok: second } = await call(server, 4, 'upload_open', { parent: 'root', name: 'b', size: 9, modified: 1 });
        expect((await call(server, 5, 'upload_commit', { upload: second.upload })).error.kind).toBe('transfer');
    });

    test('events pass root and cursor', async () => {
        const server = new Server(new MemoryBackend());
        expect((await call(server, 1, 'events_since', { root: 'root', cursor: null })).ok).toEqual({
            events: [],
            cursor: 'c1',
        });
        expect((await call(server, 2, 'events_since', { root: 'root', cursor: 'c0' })).ok.events).toEqual([
            { type: 'gap' },
        ]);
    });

    test('serve answers overlapping requests by id', async () => {
        const backend = new MemoryBackend();
        backend.slowNode = true;
        const out: string[] = [];
        async function* lines() {
            yield JSON.stringify({ id: 1, method: 'node', params: { node: 'root' } });
            yield JSON.stringify({ id: 2, method: 'hello', params: {} });
        }
        await serve(new Server(backend), lines(), (line) => out.push(line));
        const ids = out.map((line) => JSON.parse(line).id);
        expect(ids).toEqual([2, 1]);
    });
});
