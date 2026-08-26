/**
 * Control-plane protocol version (docs/13). Mirrors
 * `crates/cicada-server/src/protocol.rs::PROTOCOL_VERSION`; the two bump
 * together, and the server refuses a mismatch at `hello` instead of guessing.
 * 1 = the stage-5 protocol.
 */
export const PROTOCOL_VERSION = 1;

/**
 * `GET /api/catalog` format (docs/13 §HTTP surface). Mirrors
 * `crates/cicada-server/src/catalog.rs::CATALOG_FORMAT`; the two bump
 * together, and `fetchCatalog` refuses a body of any other format instead
 * of reading a shape it does not know — a format-2 engine (no `sub`, no
 * `subgroups`) under this app would otherwise render the category-only
 * menu the server's bump exists to prevent. 3 = v0.1 wave 5 (C2c): `sub`
 * per node and the `subgroups` table.
 */
export const CATALOG_FORMAT = 3;
