"""Print the crates linked into a Rust binary with their licenses.

Reads `cargo metadata --format-version 1 --locked` JSON on stdin and follows
only normal (non-dev, non-build) dependencies from the root package.
"""
import json
import sys

meta = json.load(sys.stdin)
nodes = {n['id']: n for n in meta['resolve']['nodes']}
seen, stack = set(), [meta['resolve']['root']]
while stack:
    pkg = stack.pop()
    if pkg in seen:
        continue
    seen.add(pkg)
    stack += [d['pkg'] for d in nodes[pkg]['deps']
              if any(k['kind'] is None for k in d['dep_kinds'])]
print('Crates linked into eitype and their licenses:\n')
for p in sorted((p for p in meta['packages'] if p['id'] in seen), key=lambda p: p['name']):
    print(f"{p['name']} {p['version']}: {p.get('license') or 'see crate'}")
