// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2c — the generic descriptor-driven reader: reconstruct a delivered loft value
// from wasm linear memory using ONLY its layout descriptor + the store base address, with no
// serialization. This MIRRORS `Stores::read_via_descriptor` (src/database/descriptor.rs) — keep
// the two in lockstep; the node harness (tests/deliver_wasm.rs) is the byte-parity gate.
//
// Addressing (Store::checked_offset):  addr(rec, pos) = storeBase + rec*8 + pos
//   * a RECORD field recurses at (rec, pos + field.pos) in the SAME record;
//   * a VECTOR field holds a child record-INDEX vRec (len at vRec*8+4, elem i at vRec*8+8+size*i);
//   * an ARRAY (by-ref vector) holds per-element record-indices (each element data at elmRec*8+8);
//   * TEXT is an interned record: the field holds a string id, len at id*8+4, UTF-8 at id*8+8;
//   * store-internal kinds (Ref / ChildRec / Iterated) are out of this subset — cursor-walked in
//     Phase 3 — exactly as `read_via_descriptor` refuses them.
//
// `desc` is the parsed `LayoutDesc::to_json` blob: {nodes:{<id>:node}, names, sizes}.
// `mem` is the WebAssembly.Memory; its `.buffer` MUST be re-read on each deliver (memory.grow
// detaches the old ArrayBuffer — §5 borrow contract). Read (or copy out) within the deliver call.

export function readLoftValue(mem, storeBase, desc, typeId, rec, pos) {
  const view = new DataView(mem.buffer);
  const nodes = desc.nodes;
  const sizeOf = (id) => (desc.sizes && desc.sizes[id] != null ? +desc.sizes[id] : 0);
  const dec = new TextDecoder();

  function read(typeId, rec, pos) {
    const node = nodes[typeId];
    if (!node) throw new Error(`deliver: type ${typeId} not in descriptor`);
    const at = storeBase + rec * 8 + pos;
    switch (node.kind) {
      case "base":
        return readBase(node.base, at);
      case "byte":
        return view.getUint8(at);
      case "short":
      case "shortraw":
        return view.getInt16(at, true);
      case "int":
        return view.getInt32(at, true);
      case "record":
      case "enumvalue": {
        const o = {};
        for (const f of node.fields) {
          if (f.name === "enum" || f.pos === 65535) continue; // read_data skips the disc/absent fields
          o[f.name] = read(f.content, rec, pos + f.pos);
        }
        return o;
      }
      case "enum": {
        const disc = view.getUint8(at);
        const v = node.variants[disc];
        return v ? v.name : disc;
      }
      case "vector": {
        const vRec = view.getUint32(at, true);
        if (vRec === 0) return [];
        const len = view.getUint32(storeBase + vRec * 8 + 4, true);
        const size = sizeOf(node.elem);
        const fast = scalarFastLane(nodes[node.elem], storeBase + vRec * 8 + 8, len);
        if (fast) return fast; // zero-copy typed-array VIEW over wasm memory
        const a = new Array(len);
        for (let i = 0; i < len; i++) a[i] = read(node.elem, vRec, 8 + size * i);
        return a;
      }
      case "array": {
        const vRec = view.getUint32(at, true);
        if (vRec === 0) return [];
        const len = view.getUint32(storeBase + vRec * 8 + 4, true);
        const a = new Array(len);
        for (let i = 0; i < len; i++) {
          const elmRec = view.getUint32(storeBase + vRec * 8 + 8 + 4 * i, true);
          a[i] = read(node.elem, elmRec, 8);
        }
        return a;
      }
      case "flatarray": {
        // @PLN105 Phase 3 — a keyed collection pre-flattened at deliver time: the data record is
        // FIXED in the descriptor (`node.data`), not read from the value's bytes (a top-level hash
        // or a keyed struct field). Otherwise identical to `array`.
        const dRec = node.data;
        if (dRec === 0) return [];
        const len = view.getUint32(storeBase + dRec * 8 + 4, true);
        const a = new Array(len);
        for (let i = 0; i < len; i++) {
          const elmRec = view.getUint32(storeBase + dRec * 8 + 8 + 4 * i, true);
          a[i] = read(node.elem, elmRec, 8);
        }
        return a;
      }
      case "ref":
      case "childrec":
      case "iterated":
        throw new Error(
          `deliver: type ${typeId} is store-internal (${node.kind}) — cursor-walked in Phase 3`,
        );
      default:
        throw new Error(`deliver: unknown node kind ${node.kind}`);
    }
  }

  function readBase(base, at) {
    switch (base) {
      case "integer":
      case "long":
        return view.getBigInt64(at, true); // loft integer/long are i64
      case "single":
        return view.getFloat32(at, true);
      case "float":
        return view.getFloat64(at, true);
      case "boolean":
        return view.getUint8(at) !== 0;
      case "character":
        return view.getUint32(at, true);
      case "text": {
        const strRec = view.getUint32(at, true);
        if (strRec === 0 || strRec > 0x7fffffff) return null; // STRING_NULL
        const len = view.getUint32(storeBase + strRec * 8 + 4, true);
        return dec.decode(new Uint8Array(mem.buffer, storeBase + strRec * 8 + 8, len));
      }
      default:
        throw new Error(`deliver: unknown base ${base}`);
    }
  }

  // The scalar-vector fast lane — the zero-copy win: a scalar element type maps the whole vector
  // to a typed-array VIEW straight over wasm memory (no per-element decode, no intermediate array).
  // Valid only during the borrow; copy out (Array.from / .slice()) to retain past the deliver call.
  function scalarFastLane(elem, byteBase, len) {
    if (!elem) return null;
    if (elem.kind === "int") return new Int32Array(mem.buffer, byteBase, len);
    if (elem.kind === "base") {
      switch (elem.base) {
        case "single":
          return new Float32Array(mem.buffer, byteBase, len);
        case "float":
          return new Float64Array(mem.buffer, byteBase, len);
        case "integer":
        case "long":
          return new BigInt64Array(mem.buffer, byteBase, len);
      }
    }
    return null;
  }

  return read(typeId, rec, pos);
}
