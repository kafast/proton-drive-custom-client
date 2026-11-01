// What the server needs from Proton Drive. The real implementation wraps the
// official SDK (sdkBackend.ts); tests use an in-memory fake.

import type { WireEvent, WireNode } from './protocol';

export interface DriveBackend {
    sdkVersion(): string;

    node(id: string): Promise<WireNode>;
    listChildren(folder: string): Promise<WireNode[]>;
    createFolder(parent: string, name: string): Promise<WireNode>;

    /** Starts an upload of a new file; returns a sink for the content. */
    openUpload(parent: string, name: string, size: number, modified: number): Promise<UploadSink>;
    /** Starts an upload of a new revision of `file`. */
    openRevision(file: string, size: number, modified: number): Promise<UploadSink>;

    /** Opens the whole content of a file for sequential reading. */
    openDownload(file: string): Promise<DownloadSource>;
    /** Reads part of a file; fewer bytes at the end, `invalid_range` beyond it. */
    readRange(file: string, offset: number, length: number): Promise<Uint8Array>;

    rename(id: string, name: string): Promise<WireNode>;
    move(id: string, parent: string): Promise<WireNode>;
    trash(id: string): Promise<void>;
    restore(id: string): Promise<WireNode>;
    deletePermanently(id: string): Promise<void>;

    /** Events of the volume that contains `root`, after `cursor`. */
    eventsSince(root: string, cursor: string | null): Promise<{ events: WireEvent[]; cursor: string }>;
}

export interface UploadSink {
    write(data: Uint8Array): Promise<void>;
    /** Finishes the upload; fails with `transfer` if the size is wrong. */
    commit(): Promise<WireNode>;
    /** Gives up; nothing may remain at Proton. */
    abort(): Promise<void>;
}

export interface DownloadSource {
    /** Up to `max` bytes; an empty array with `eof` at the end. */
    read(max: number): Promise<{ data: Uint8Array; eof: boolean }>;
    close(): Promise<void>;
}

/** Sign-in and entry points; only the real helper has them. */
export interface AccountBackend {
    session(): Promise<{ logged_in: boolean; user: string | null }>;
    /** Starts a browser login (session fork); returns the link to open. */
    loginWebStart(): Promise<{ url: string }>;
    /** Waits a while for the browser login; `done: false` means "ask again". */
    loginWebWait(): Promise<{ done: boolean; user: string | null }>;
    logout(): Promise<void>;
    myFilesRoot(): Promise<WireNode>;
    sharedWithMe(): Promise<WireNode[]>;
}
