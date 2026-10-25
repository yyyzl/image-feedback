#!/usr/bin/env python3
"""Collect actual Cargo package notices into a portable, self-contained HTML file."""
import argparse, hashlib, html, json, pathlib, subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1', '--filter-platform', args.target, '--manifest-path', str(root / 'Cargo.toml')], cwd=root, text=True, encoding='utf-8'))
    tree = subprocess.check_output(['cargo', 'tree', '--locked', '--target', args.target, '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}', '--manifest-path', str(root / 'Cargo.toml')], cwd=root, text=True, encoding='utf-8')
    actual = {(parts[0], parts[1][1:]) for line in tree.splitlines() if len(parts := line.split()) > 1 and parts[1].startswith('v')}
    resolved = {package['id'] for package in metadata['packages'] if (package['name'], package['version']) in actual}
    override_file = root / 'packaging/license-overrides.json'
    overrides = json.loads(override_file.read_text(encoding='utf-8')) if override_file.exists() else {}
    sections, inventory, missing = [], [], []
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['id'] not in resolved:
            continue
        base = pathlib.Path(package['manifest_path']).parent.resolve()
        files = set()
        if package.get('license_file'):
            path = pathlib.Path(package['license_file'])
            files.add((base / path).resolve())
        for pattern in ('LICENSE*', 'LICENCE*', 'COPYING*', 'NOTICE*', 'license*', 'licence*', 'licenses/*', 'LICENSES/*'):
            files.update(path.resolve() for path in base.glob(pattern) if path.is_file())
        files = sorted(path for path in files if path.is_file() and path.is_relative_to(base))
        name = f"{package['name']} {package['version']}"
        record = {key: package.get(key) for key in ('name', 'version', 'license', 'authors', 'repository')}
        record['license_files'] = [str(path.relative_to(base)).replace('\\', '/') for path in files]
        recovered = []
        for item in overrides.get(name, []):
            path = (root / item['path']).resolve()
            if not path.is_relative_to(root) or hashlib.sha256(path.read_bytes()).hexdigest() != item['sha256']:
                raise SystemExit(f'Pinned upstream notice mismatch: {name}')
            recovered.append((item['path'], path.read_text(encoding='utf-8'), item['source']))
        record['recovered_notices'] = overrides.get(name, [])
        inventory.append(record)
        if not files and not recovered:
            missing.append(name)
        body = ''.join(f'<h3>{html.escape(str(path.relative_to(base)))}</h3><pre>{html.escape(path.read_text(encoding="utf-8", errors="replace"))}</pre>' for path in files)
        body += ''.join(f'<h3>{html.escape(label)}</h3><p>{html.escape(source)}</p><pre>{html.escape(text)}</pre>' for label, text, source in recovered)
        sections.append(f'<details><summary>{html.escape(name)} — {html.escape(package.get("license") or "see source notices")}</summary><p>{html.escape(", ".join(package.get("authors", [])))}</p>{body}</details>')
    # Full Slint license chosen for desktop distribution, in addition to crate notices.
    slint = (root / 'licenses/slint-royalty-free-2.0.md').read_text(encoding='utf-8')
    output = pathlib.Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text('<!doctype html><meta charset="utf-8"><title>Image Feedback — Third-party licenses</title><style>body{max-width:1000px;margin:40px auto;font:15px system-ui}summary{padding:10px;cursor:pointer}pre{white-space:pre-wrap}details{border-bottom:1px solid #ddd}</style><h1>Image Feedback: dependency notices</h1><p>Application code is MIT. Dependencies retain their own licenses. This file contains package-provided license and notice texts; it is not a legal opinion.</p><h2>Slint Royalty-free Desktop License 2.0</h2><pre>' + html.escape(slint) + '</pre><h2>Cargo packages</h2>' + ''.join(sections), encoding='utf-8')
    output.with_suffix('.json').write_text(json.dumps({'target': args.target, 'packages': inventory, 'missing_license_files': missing}, ensure_ascii=False, indent=2), encoding='utf-8')
    if missing:
        raise SystemExit('Missing package-provided notices; review before release: ' + ', '.join(missing))
    print(f'Collected notices for {len(inventory)} packages: {output.name}')


if __name__ == '__main__':
    main()
