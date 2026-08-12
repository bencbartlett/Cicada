/**
 * Control-plane protocol version (docs/13). The client sends this on hello;
 * the server rejects mismatches instead of guessing. Version 0 = pre-protocol
 * stage-0 scaffold; stage 5 bumps it when the first real message ships.
 */
export const PROTOCOL_VERSION = 0;
