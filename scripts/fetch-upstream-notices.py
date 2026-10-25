#!/usr/bin/env python3
"""Maintainer tool: recover workspace-level notices omitted from published crates.
Review the resulting pinned URLs and texts before committing license-overrides.json.
"""
import argparse, hashlib, json, pathlib, re, subprocess, urllib.request, urllib.error

ROOT=pathlib.Path(__file__).resolve().parents[1]
CACHE={}
def download(url):
    if url not in CACHE:
        try:
            request=urllib.request.Request(url,headers={'User-Agent':'ImageFeedback-license-audit'})
            with urllib.request.urlopen(request,timeout=20) as response: CACHE[url]=response.read()
        except urllib.error.HTTPError as error:
            if error.code!=404: raise
            CACHE[url]=None
    return CACHE[url]

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--inventory',required=True);args=parser.parse_args()
    inventory=json.loads(pathlib.Path(args.inventory).read_text(encoding='utf-8'))
    missing=set(inventory['missing_license_files'])
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version=1','--manifest-path',str(ROOT/'Cargo.toml')],cwd=ROOT,text=True,encoding='utf-8'))
    manifest=ROOT/'packaging/license-overrides.json'
    records=json.loads(manifest.read_text(encoding='utf-8')) if manifest.exists() else {}
    for package in metadata['packages']:
        key=f'{package["name"]} {package["version"]}'
        if key not in missing: continue
        if key in records:
            print(f'{key}: preserving reviewed override')
            continue
        repo=(package.get('repository') or '').replace('http://github.com/','https://github.com/').removesuffix('.git').rstrip('/')
        match=re.fullmatch(r'https://github.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)',repo)
        if not match: raise SystemExit(f'Review non-GitHub package manually: {key}: {repo}')
        base=pathlib.Path(package['manifest_path']).parent
        vcs=base/'.cargo_vcs_info.json'
        info=json.loads(vcs.read_text(encoding='utf-8')) if vcs.exists() else {}
        sha=info.get('git',{}).get('sha1')
        if not sha:
            data=download(f'https://api.github.com/repos/{match[1]}/commits/HEAD')
            sha=json.loads(data)['sha']
        entries=[]
        for name in ['LICENSE','LICENSE-MIT','LICENSE-APACHE','LICENSE.md','LICENSE.txt','COPYING','COPYING.md','COPYRIGHT']:
            url=f'https://raw.githubusercontent.com/{match[1]}/{sha}/{name}'
            data=download(url)
            if data is None:continue
            if len(data)>2*1024*1024:raise SystemExit(f'Unexpected notice size: {url}')
            relative=pathlib.Path('licenses/third-party')/f'{package["name"]}-{package["version"]}'/name
            path=ROOT/relative;path.parent.mkdir(parents=True,exist_ok=True)
            if path.exists() and path.read_bytes()!=data:raise SystemExit(f'Refusing to overwrite changed notice: {path}')
            path.write_bytes(data)
            entries.append({'path':relative.as_posix(),'source':url,'sha256':hashlib.sha256(data).hexdigest()})
        if not entries:raise SystemExit(f'No upstream notices found: {key}')
        records[key]=entries
        manifest.parent.mkdir(parents=True,exist_ok=True)
        manifest.write_text(json.dumps(records,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
        print(f'{key}: recovered {len(entries)} pinned notices')
    manifest.parent.mkdir(parents=True,exist_ok=True)
    manifest.write_text(json.dumps(records,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')

if __name__=='__main__':main()
