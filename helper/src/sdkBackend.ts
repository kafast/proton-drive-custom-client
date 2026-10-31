// DriveBackend on Proton's official Drive SDK.
//
// This file is compiled inside the pinned SDK checkout (see build.sh): it is
// copied to `cli/src/protonsync/` there, so `../init` is Proton's own CLI
// wiring (HTTP client, crypto, caches, credentials store) and
// `@protontech/drive-sdk` resolves to the SDK sources.

import {
    ConnectionError,
    DecryptionError,
    DriveEventType,
    IntegrityError,
    type NodeEntity,
    NodeType,
    NodeWithSameNameExistsValidationError,
    RateLimitedError,
    ServerError,
    ValidationError,
} from '@protontech/drive-sdk';

import type { init } from '../init';
import type { AccountBackend, DownloadSource, DriveBackend, UploadSink } from './backend';
import { HelperError, type WireError, type WireEvent, type WireNode } from './protocol';

type Initialized = Awaited<ReturnType<typeof init>>;

/** Proton's error code for "does not exist". */
const NOT_EXISTS = 2501;
const HTTP_UNAUTHORIZED = 401;
const HTTP_NOT_FOUND = 404;
const DEFAULT_MEDIA_TYPE = 'application/octet-stream';
/** How long `login_web_wait` blocks before answering "not yet". */
const LOGIN_WAIT_MS = 30_000;

export class SdkBackend implements DriveBackend, AccountBackend {
    private pendingLogin: Promise<void> | null = null;
    private loginError: unknown = null;

    constructor(
        private readonly app: Initialized,
        private readonly version: string,
    ) {}

    sdkVersion(): string {
        return this.version;
    }

    // --- account -------------------------------------------------------

    async session(): Promise<{ logged_in: boolean; user: string | null }> {
        if (!this.app.auth.isLoggedIn()) {
            return { logged_in: false, user: null };
        }
        const address = await guard({}, () => this.app.addresses.getOwnPrimaryAddress());
        return { logged_in: true, user: address.email };
    }

    async loginWebStart(): Promise<{ url: string }> {
        this.loginError = null;
        let resolveUrl: (url: string) => void = () => {};
        const url = new Promise<string>((resolve) => {
            resolveUrl = resolve;
        });
        this.pendingLogin = this.app.auth
            .authViaWeb((signInUrl) => resolveUrl(signInUrl))
            .then(async () => {
                await this.app.eventsManager.startSubscriptions();
            })
            .catch((error: unknown) => {
                this.loginError = error;
            });
        return { url: await url };
    }

    async loginWebWait(): Promise<{ done: boolean; user: string | null }> {
        if (!this.pendingLogin) {
            throw new HelperError({ kind: 'rejected', message: 'no browser login started' });
        }
        const finished = await Promise.race([
            this.pendingLogin.then(() => true),
            new Promise<boolean>((resolve) => setTimeout(() => resolve(false), LOGIN_WAIT_MS)),
        ]);
        if (!finished) {
            return { done: false, user: null };
        }
        this.pendingLogin = null;
        if (this.loginError) {
            throw new HelperError({ kind: 'rejected', message: messageOf(this.loginError) });
        }
        const { user } = await this.session();
        return { done: true, user };
    }

    async logout(): Promise<void> {
        await this.app.auth.logout();
        await this.app.clearCaches();
    }

    async myFilesRoot(): Promise<WireNode> {
        return toWire(await guard({}, () => this.app.sdk.getMyFilesRootFolder()));
    }

    async sharedWithMe(): Promise<WireNode[]> {
        const nodes: WireNode[] = [];
        for await (const node of this.app.sdk.iterateSharedNodesWithMe()) {
            const wire = tryToWire(node);
            if (wire) {
                // Shared roots belong to someone else's tree; protonsync treats them as roots.
                delete wire.parent;
                nodes.push(wire);
            }
        }
        return nodes;
    }

    // --- drive ---------------------------------------------------------

    async node(id: string): Promise<WireNode> {
        const node = await this.entity(id);
        if (node.trashTime) {
            throw new HelperError({ kind: 'not_found', node: id });
        }
        return toWire(node);
    }

    async listChildren(folder: string): Promise<WireNode[]> {
        const parent = await this.entity(folder);
        if (parent.type === NodeType.File || parent.type === NodeType.Photo) {
            throw new HelperError({ kind: 'not_a_folder', node: folder });
        }
        const children: WireNode[] = [];
        await guard({ node: folder }, async () => {
            for await (const child of this.app.sdk.iterateFolderChildren(folder)) {
                const wire = child.trashTime ? null : tryToWire(child);
                if (wire) {
                    children.push(wire);
                }
            }
        });
        return children;
    }

    async createFolder(parent: string, name: string): Promise<WireNode> {
        return toWire(await guard({ node: parent, name }, () => this.app.sdk.createFolder(parent, name)));
    }

    async openUpload(parent: string, name: string, size: number, modified: number): Promise<UploadSink> {
        const uploader = await guard({ node: parent, name }, () =>
            this.app.sdk.getFileUploader(parent, name, metadata(size, modified)),
        );
        return this.sink(uploader, size, { node: parent, name });
    }

    async openRevision(file: string, size: number, modified: number): Promise<UploadSink> {
        const uploader = await guard({ node: file }, () =>
            this.app.sdk.getFileRevisionUploader(file, metadata(size, modified)),
        );
        return this.sink(uploader, size, { node: file });
    }

    private sink(
        uploader: { uploadFromStream(stream: ReadableStream, thumbnails: []): Promise<{ completion(): Promise<{ nodeUid: string }> }> },
        size: number,
        context: Context,
    ): UploadSink {
        const pipe = new TransformStream<Uint8Array, Uint8Array>();
        const writer = pipe.writable.getWriter();
        const finished = uploader.uploadFromStream(pipe.readable, []).then((controller) => controller.completion());
        // Errors are reported at commit; keep the promise from counting as unhandled.
        finished.catch(() => {});
        let written = 0;
        return {
            write: async (data) => {
                written += data.length;
                await writer.write(data);
            },
            commit: async () => {
                if (written !== size) {
                    await writer.abort(new Error('size mismatch')).catch(() => {});
                    throw new HelperError({ kind: 'transfer', message: `expected ${size} bytes, got ${written}` });
                }
                await writer.close();
                const { nodeUid } = await guard(context, () => finished);
                return this.node(nodeUid);
            },
            abort: async () => {
                await writer.abort(new Error('aborted by protonsync')).catch(() => {});
                await finished.catch(() => {});
            },
        };
    }

    async openDownload(file: string): Promise<DownloadSource> {
        const stream = await this.seekable(file);
        return {
            read: async (max) => {
                if (max <= 0) {
                    return { data: new Uint8Array(), eof: false };
                }
                const { value, done } = await guard({ node: file }, () => stream.read(max));
                return { data: value, eof: done || value.length < max };
            },
            close: async () => {
                await stream.cancel().catch(() => {});
            },
        };
    }

    async readRange(file: string, offset: number, length: number): Promise<Uint8Array> {
        const node = await this.node(file);
        if (node.type !== 'file') {
            throw new HelperError({ kind: 'not_a_file', node: file });
        }
        if (offset > node.size) {
            throw new HelperError({ kind: 'invalid_range', offset, size: node.size });
        }
        const wanted = Math.min(length, node.size - offset);
        if (wanted === 0) {
            return new Uint8Array();
        }
        const stream = await this.seekable(file);
        try {
            await stream.seek(offset);
            return (await guard({ node: file }, () => stream.read(wanted))).value;
        } finally {
            await stream.cancel().catch(() => {});
        }
    }

    async rename(id: string, name: string): Promise<WireNode> {
        return toWire(await guard({ node: id, name }, () => this.app.sdk.renameNode(id, name)));
    }

    async move(id: string, parent: string): Promise<WireNode> {
        await single(id, this.app.sdk.moveNodes([id], parent));
        return this.node(id);
    }

    async trash(id: string): Promise<void> {
        await this.node(id);
        await single(id, this.app.sdk.trashNodes([id]));
    }

    async restore(id: string): Promise<WireNode> {
        await single(id, this.app.sdk.restoreNodes([id]));
        return this.node(id);
    }

    async deletePermanently(id: string): Promise<void> {
        const node = await this.entity(id);
        if (!node.trashTime) {
            throw new HelperError({ kind: 'rejected', message: `${id} is not in the trash` });
        }
        await single(id, this.app.sdk.deleteNodes([id]));
    }

    async eventsSince(root: string, cursor: string | null): Promise<{ events: WireEvent[]; cursor: string }> {
        const scope = (await this.entity(root)).treeEventScopeId;
        const events: WireEvent[] = [];
        let latest = cursor;
        await guard({}, async () => {
            for await (const event of this.app.sdk.iterateEvents(scope, cursor ?? undefined)) {
                latest = event.eventId === 'none' ? latest : event.eventId;
                if (cursor === null) {
                    continue; // Only the starting point is wanted.
                }
                const wire = toWireEvent(event);
                if (wire) {
                    events.push(wire);
                }
            }
        });
        if (latest === null) {
            throw new HelperError({ kind: 'unavailable', message: 'no event id received' });
        }
        return { events, cursor: latest };
    }

    // --- helpers -------------------------------------------------------

    private async entity(id: string): Promise<NodeEntity> {
        // Node UIDs look like `volume~node`; anything else cannot exist.
        if (!id.includes('~')) {
            throw new HelperError({ kind: 'not_found', node: id });
        }
        return guard({ node: id }, () => this.app.sdk.getNode(id));
    }

    private async seekable(file: string) {
        const node = await this.node(file);
        if (node.type !== 'file') {
            throw new HelperError({ kind: 'not_a_file', node: file });
        }
        const downloader = await guard({ node: file }, () => this.app.sdk.getFileDownloader(file));
        return downloader.getSeekableStream();
    }
}

type Context = { node?: string; name?: string };

function metadata(size: number, modified: number) {
    return {
        mediaType: DEFAULT_MEDIA_TYPE,
        expectedSize: size,
        modificationTime: new Date(modified * 1000),
    };
}

/** Runs `action` and turns SDK errors into wire errors. */
async function guard<T>(context: Context, action: () => Promise<T>): Promise<T> {
    try {
        return await action();
    } catch (error) {
        throw new HelperError(wireErrorOf(error, context));
    }
}

/** Waits for the single result of a batch operation (move, trash, …). */
async function single(id: string, results: AsyncGenerator<{ uid: string; ok: boolean; error?: Error }>) {
    await guard({ node: id }, async () => {
        for await (const result of results) {
            if (!result.ok) {
                throw result.error ?? new Error(`operation on ${id} failed`);
            }
        }
    });
}

export function wireErrorOf(error: unknown, context: Context): WireError {
    if (error instanceof HelperError) {
        return error.wire;
    }
    if (error instanceof NodeWithSameNameExistsValidationError) {
        return { kind: 'already_exists', name: context.name ?? '' };
    }
    if (error instanceof RateLimitedError || error instanceof ConnectionError) {
        return { kind: 'unavailable', message: error.message };
    }
    if (error instanceof ServerError) {
        if (error.statusCode === HTTP_UNAUTHORIZED) {
            return { kind: 'unauthorized' };
        }
        if ((error.code === NOT_EXISTS || error.statusCode === HTTP_NOT_FOUND) && context.node) {
            return { kind: 'not_found', node: context.node };
        }
        return { kind: 'rejected', message: error.message };
    }
    if (error instanceof ValidationError) {
        return { kind: 'rejected', message: error.message };
    }
    if (error instanceof IntegrityError || error instanceof DecryptionError) {
        return { kind: 'transfer', message: error.message };
    }
    return { kind: 'unavailable', message: messageOf(error) };
}

function messageOf(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

function toWire(node: NodeEntity): WireNode {
    const wire = tryToWire(node);
    if (!wire) {
        throw new HelperError({ kind: 'rejected', message: `node ${node.uid} cannot be decrypted` });
    }
    return wire;
}

/** Converts a node; `null` for nodes whose name cannot be decrypted. */
function tryToWire(node: NodeEntity): WireNode | null {
    if (!node.name.ok) {
        return null;
    }
    const base = { id: node.uid, name: node.name.value, ...(node.parentUid ? { parent: node.parentUid } : {}) };
    if (node.type === NodeType.File || node.type === NodeType.Photo) {
        const revision = node.activeRevision;
        if (!revision) {
            return null;
        }
        const modified = revision.claimedModificationTime ?? node.modificationTime;
        return {
            ...base,
            type: 'file',
            revision: revision.uid,
            size: revision.claimedSize ?? 0,
            modified: Math.floor(modified.getTime() / 1000),
            ...(revision.claimedDigests?.sha1 ? { sha1: revision.claimedDigests.sha1.toLowerCase() } : {}),
        };
    }
    return { ...base, type: 'folder' };
}

function toWireEvent(event: { type: DriveEventType; nodeUid?: string; isTrashed?: boolean }): WireEvent | null {
    switch (event.type) {
        case DriveEventType.NodeCreated:
            return { type: event.isTrashed ? 'trashed' : 'created', node: event.nodeUid ?? '' };
        case DriveEventType.NodeUpdated:
            return { type: event.isTrashed ? 'trashed' : 'updated', node: event.nodeUid ?? '' };
        case DriveEventType.NodeDeleted:
            return { type: 'trashed', node: event.nodeUid ?? '' };
        case DriveEventType.TreeRefresh:
        case DriveEventType.TreeRemove:
            return { type: 'gap' };
        default:
            return null;
    }
}
