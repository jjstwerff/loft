#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""
grammar_gen.py — @PLN85 fuzz-proof generator, the FULL ownership grammar.

Widens ownership_fuzz.py past the churn axis (mutating `0..N` bounds) to the full
composition cross-product that the over-free class lives on:

  shape (source × delivery) × value × churn

Every cell emits a COMPLETE, self-checking loft program grounded in a real over-free
probe shape, so the generated programs are valid loft (the compiler is the validity
oracle — invalid cells are dropped by the runner as agreed-compile-errors). Each program
asserts the structural invariant the over-free bug violates: the SOURCE container length
stays constant and the DELIVERED view has the known length K. A borrowed-view over-free
corrupts one of those → the program's own assert fails (a DIVERGENCE / signal under churn).

Axes:
  shape  : field_return | field_local | field_reassign | match_return | if_return | elem_accumulate
           (= source {field-view, element-view, match-arm, if-arm} × delivery {direct, local, reassign, accumulate})
  value  : struct (vector<E> of a 2-field struct) | scalar (vector<integer>)
  churn  : none | heavy   (the filler() slot-recycling loop — the over-free only bites under reuse)

Usage:
  grammar_gen.py --out <dir>          # write the cross-product as <dir>/<shape>_<value>_<churn>.loft
  grammar_gen.py --out <dir> --list   # also print the manifest
Then judge with the runner:
  ownership_fuzz.py --corpus <dir>                 # interp fast-loop + native replay on flagged
"""
import argparse, itertools, os, pathlib

K = 3  # source length — known, so the asserts are hand-computed

VALUES = {
    # element type E, its default, and how to build element `<var>`
    "struct": {
        "decl": 'struct E { hp: integer not null, name: text }\nfn e_default() -> E { E{hp:0, name:""} }',
        "et": "E",
        "default": "e_default()",
        "el": lambda var: 'E{hp:' + var + ', name:"e"}',
    },
    "scalar": {
        "decl": "",
        "et": "integer",
        "default": "0",
        "el": lambda var: var,
    },
}

CHURN = {
    "none": "",
    "heavy": "acc=0; for f in 0..8 { acc += filler(7); }",
}


def filler(v):
    return ("fn filler(n: integer) -> integer { es: vector<" + v["et"]
            + "> = []; for j in 0..n { es += [" + v["el"]("j") + "]; } return len(es); }")


def build(target, n, v):
    return "for k in 0.." + str(n) + " { " + target + " += [" + v["el"]("k") + "]; }"


def header(v):
    return (v["decl"] + ("\n" if v["decl"] else "") + filler(v) + "\n")


# Each shape returns the full program text for (value v, churn snippet c).
def field_return(v, c):
    return (header(v)
        + "struct Box { rows: vector<" + v["et"] + "> }\n"
        + "fn deliver(b: Box) -> vector<" + v["et"] + "> { b.rows }\n"
        + "fn main() {\n  b = Box { rows: [] }; " + build("b.rows", K, v) + "\n"
        + "  for i in 0..8 { r = deliver(b); " + c
        + ' assert(len(b.rows)==' + str(K) + ', "src i{i}={len(b.rows)}");'
        + ' assert(len(r)==' + str(K) + ', "del i{i}={len(r)}"); }\n'
        + '  println("ok");\n}\n')


def field_local(v, c):
    return (header(v)
        + "struct Box { rows: vector<" + v["et"] + "> }\n"
        + "fn deliver(b: Box) -> vector<" + v["et"] + "> { out: vector<" + v["et"] + "> = b.rows; out }\n"
        + "fn main() {\n  b = Box { rows: [] }; " + build("b.rows", K, v) + "\n"
        + "  for i in 0..8 { r = deliver(b); " + c
        + ' assert(len(b.rows)==' + str(K) + ', "src i{i}={len(b.rows)}");'
        + ' assert(len(r)==' + str(K) + ', "del i{i}={len(r)}"); }\n'
        + '  println("ok");\n}\n')


def field_reassign(v, c):
    return (header(v)
        + "struct Box { rows: vector<" + v["et"] + "> }\n"
        + "fn rows(b: Box) -> vector<" + v["et"] + "> { b.rows }\n"
        + "fn deliver(b: Box, c: Box) -> vector<" + v["et"]
        + "> { best = rows(b); o = rows(c); if len(o) > len(best) { best = o; } best }\n"
        + "fn main() {\n  b = Box { rows: [] }; b.rows += [" + v["el"]("0") + "];\n"
        + "  c = Box { rows: [] }; " + build("c.rows", K, v) + "\n"
        + "  for i in 0..8 { r = deliver(b, c); " + c
        + ' assert(len(b.rows)==1, "b i{i}={len(b.rows)}");'
        + ' assert(len(c.rows)==' + str(K) + ', "c i{i}={len(c.rows)}");'
        + ' assert(len(r)==' + str(K) + ', "del i{i}={len(r)}"); }\n'
        + '  println("ok");\n}\n')


def match_return(v, c):
    return (header(v)
        + "enum Cell { Empty, Filled { items: vector<" + v["et"] + "> } }\n"
        + "fn deliver(e: Cell) -> vector<" + v["et"] + "> { match e { Filled { items } => { items }, _ => { [] } } }\n"
        + "fn main() {\n  inner: vector<" + v["et"] + "> = []; " + build("inner", K, v) + "\n"
        + "  cell = Filled { items: inner };\n"
        + "  for i in 0..8 { r = deliver(cell); " + c
        + ' assert(len(r)==' + str(K) + ', "del i{i}={len(r)}"); }\n'
        + '  println("ok");\n}\n')


def if_return(v, c):
    return (header(v)
        + "struct Box { rows: vector<" + v["et"] + "> }\n"
        + "fn deliver(b: Box, cond: boolean) -> vector<" + v["et"] + "> { if cond { b.rows } else { [] } }\n"
        + "fn main() {\n  b = Box { rows: [] }; " + build("b.rows", K, v) + "\n"
        + "  for i in 0..8 { r = deliver(b, true); " + c
        + ' assert(len(b.rows)==' + str(K) + ', "src i{i}={len(b.rows)}");'
        + ' assert(len(r)==' + str(K) + ', "del i{i}={len(r)}"); }\n'
        + '  println("ok");\n}\n')


def elem_accumulate(v, c):
    return (header(v)
        + "fn pick(t: vector<" + v["et"] + ">, i: integer) -> " + v["et"] + " { t[i] ?? " + v["default"] + " }\n"
        + "fn collect(t: vector<" + v["et"] + ">) -> vector<" + v["et"]
        + "> { out: vector<" + v["et"] + "> = []; for i in 0..len(t) { out += [pick(t, i)]; } out }\n"
        + "fn main() {\n  t: vector<" + v["et"] + "> = []; " + build("t", K, v) + "\n"
        + "  for i in 0..8 { r = collect(t); " + c
        + ' assert(len(t)==' + str(K) + ', "src i{i}={len(t)}");'
        + ' assert(len(r)==' + str(K) + ', "del i{i}={len(r)}"); }\n'
        + '  println("ok");\n}\n')


SHAPES = {
    "field_return": field_return, "field_local": field_local, "field_reassign": field_reassign,
    "match_return": match_return, "if_return": if_return, "elem_accumulate": elem_accumulate,
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()
    outdir = pathlib.Path(args.out)
    outdir.mkdir(parents=True, exist_ok=True)
    n = 0
    for shape, value, churn in itertools.product(SHAPES, VALUES, CHURN):
        src = SHAPES[shape](VALUES[value], CHURN[churn])
        name = f"{shape}__{value}__{churn}.loft"
        (outdir / name).write_text(src)
        n += 1
        if args.list:
            print(name)
    print(f"# generated {n} programs ({len(SHAPES)} shapes × {len(VALUES)} values × {len(CHURN)} churn) -> {outdir}")


if __name__ == "__main__":
    main()
