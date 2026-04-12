/* tslint:disable */
/* eslint-disable */

/**
 * Run a loft program supplied as a JSON array of `{name, content}` file objects.
 *
 * Returns a JSON string: `{"output": "...", "diagnostics": [...], "success": true|false}`.
 *
 * The default standard library files are embedded in the binary; user files
 * are taken from `files_json`.  Any `use <id>;` statement is resolved against
 * files whose name matches `<id>.loft` in the supplied file list.
 *
 * # Errors
 * Returns a JSON error object if `files_json` cannot be parsed.
 *
 * When compiled with `--features wasm` and exported via `wasm-bindgen`, this
 * function is callable from JavaScript as:
 * ```js
 * const result = JSON.parse(loft.compile_and_run(JSON.stringify([
 *   {name: 'main.loft', content: 'fn main() { println("hi") }'}
 * ])));
 * ```
 */
export function compile_and_run(files_json: string): string;

/**
 * Start a game session: parse, compile, execute until the first frame yield.
 * Returns JSON `{"ok":true}` on success or `{"ok":false,"error":"..."}` on failure.
 */
export function compile_and_start(files_json: string): string;

/**
 * Resume execution after a frame yield.  Returns JSON:
 * `{"running":true}` — yielded again, call on next requestAnimationFrame
 * `{"running":false,"output":"..."}` — program finished
 * `{"running":false,"error":"..."}` — program crashed
 */
export function resume_frame(): string;

/**
 * Entry point called by each Worker Thread.  The JS worker loop calls
 * this with the function index and element range.  The worker reads from the
 * shared WASM memory (Store heap) and writes results directly back.
 *
 * This is a no-op stub until the wasm-threads feature build is available.
 * The actual implementation needs access to the shared State, which requires
 * the wasm-threads + atomics build flags.
 */
export function worker_entry(_fn_index: number, _start: number, _end: number): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly compile_and_run: (a: number, b: number) => [number, number];
    readonly compile_and_start: (a: number, b: number) => [number, number];
    readonly resume_frame: () => [number, number];
    readonly worker_entry: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
