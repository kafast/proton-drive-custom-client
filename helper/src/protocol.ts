// Wire format of the SDK helper (ADR 0025, docs/sdk-helfer-protokoll.md).
// Mirrors crates/adapter-sdk-helper/src/protocol.rs; keep both in sync.

/** Version of the protocol; the Rust side checks it in `hello`. */
export const PROTOCOL_VERSION = 1;

/** Largest piece of file content per message (1 MiB before base64). */
export const CHUNK_BYTES = 1024 * 1024;

export type Request = {
    id: number;
    method: string;
    params?: Record<string, unknown>;
};

export type Response = { id: number; ok: unknown } | { id: number; error: WireError };

export type WireError =
    | { kind: 'not_found'; node: string }
    | { kind: 'already_exists'; name: string }
    | { kind: 'not_a_folder'; node: string }
    | { kind: 'not_a_file'; node: string }
    | { kind: 'invalid_range'; offset: number; size: number }
    | { kind: 'unauthorized' }
    | { kind: 'invalid_move' | 'transfer' | 'rejected' | 'unavailable'; message: string };

/** An error that crosses the wire with its kind intact. */
export class HelperError extends Error {
    constructor(readonly wire: WireError) {
        super('message' in wire ? wire.message : wire.kind);
    }
}

export type WireNode = {
    id: string;
    parent?: string;
    name: string;
} & (
    | { type: 'folder' }
    | { type: 'file'; revision: string; size: number; modified: number; sha1?: string }
);

export type WireEvent =
    | { type: 'created' | 'updated' | 'trashed'; node: string }
    | { type: 'gap' };
