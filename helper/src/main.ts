// Entry point of the protonsync SDK helper (ADR 0025).
//
// Started by the protonsync daemon; speaks JSON lines on stdin/stdout.
// stdout belongs to the protocol, so every console output goes to stderr.
//
// Environment (set by the daemon):
// - PROTON_DRIVE_CACHE_DIR: data directory of the helper (session, caches)
// - PROTON_DRIVE_CREDENTIALS_STORE: `keychain` (desktop) or `unsafe_file` (server)

import { init } from '../init';
import { SdkBackend, wireErrorOf } from './sdkBackend';
import { serve, Server } from './server';

declare const APP_VERSION: string;
declare const SDK_VERSION: string;

// Keep stdout clean for the protocol.
console.log = console.error;
console.info = console.error;
console.debug = console.error;

async function* stdinLines(): AsyncGenerator<string> {
    const decoder = new TextDecoder();
    let buffered = '';
    for await (const chunk of Bun.stdin.stream()) {
        buffered += decoder.decode(chunk, { stream: true });
        let newline = buffered.indexOf('\n');
        while (newline >= 0) {
            yield buffered.slice(0, newline);
            buffered = buffered.slice(newline + 1);
            newline = buffered.indexOf('\n');
        }
    }
}

const app = await init({
    appVersion: APP_VERSION,
    sdkVersion: SDK_VERSION,
    clientUidPrefix: 'protonsync',
    enablePersistedEvents: false,
    enableConsoleLog: false,
    enableMetrics: false,
});
const backend = new SdkBackend(app, SDK_VERSION);
const server = new Server(backend, backend, (error) => wireErrorOf(error, {}));
await serve(server, stdinLines(), (line) => {
    process.stdout.write(line);
});
await app.dispose();
process.exit(0);
