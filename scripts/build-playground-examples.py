#!/usr/bin/env python3
# Generate doc/examples.js from tests/docs/*.loft
# Run: python3 scripts/build-playground-examples.py > doc/examples.js

import os, json, re, sys

doc_dir = os.path.join(os.path.dirname(__file__), '..', 'tests', 'docs')
examples = []
for f in sorted(os.listdir(doc_dir)):
    if not f.endswith('.loft') or not os.path.isfile(os.path.join(doc_dir, f)):
        continue
    path = os.path.join(doc_dir, f)
    with open(path) as fh:
        content = fh.read()
    m = re.search(r'@NAME:\s*(.+)', content)
    name = m.group(1).strip() if m else f.replace('.loft', '')
    lines = content.split('\n')
    code_lines = []
    in_header = True
    for line in lines:
        if in_header and (line.startswith('//') or line.strip() == ''):
            continue
        in_header = False
        code_lines.append(line)
    code = '\n'.join(code_lines).strip()
    key = name.lower().replace(' ', '_')
    examples.append((key, name, code))

print('// Auto-generated from tests/docs/*.loft — do not edit manually.')
print('// Regenerate with: python3 scripts/build-playground-examples.py > doc/examples.js')
print('export const EXAMPLES = {')
for key, name, code in examples:
    print(f'  {json.dumps(key)}: {json.dumps(code)},')
print('};')
print('export const EXAMPLE_LIST = [')
for key, name, code in examples:
    print(f'  {{ key: {json.dumps(key)}, name: {json.dumps(name)} }},')
print('];')

print(f'// {len(examples)} examples', file=sys.stderr)
