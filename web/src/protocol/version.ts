/**
 * Control-plane protocol version (docs/13). Mirrors
 * `crates/cicada-server/src/protocol.rs::PROTOCOL_VERSION`; the two bump
 * together, and the server refuses a mismatch at `hello` instead of guessing.
 * 1 = the stage-5 protocol.
 */
export const PROTOCOL_VERSION = 1;
