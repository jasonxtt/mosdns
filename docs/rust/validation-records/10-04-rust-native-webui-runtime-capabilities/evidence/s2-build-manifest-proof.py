"""Exercise the real Cargo build script with tiny independent filesystem fixtures."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

source = Path('/root/mosdns-rust-webui-20261004/source')
binary = max((source/'rust/target/debug/build').glob('mosdns-native-host-*/build-script-build'), key=lambda p: p.stat().st_mtime)
results = {}
with tempfile.TemporaryDirectory(prefix='native-manifest-') as tmp:
    root = Path(tmp)
    crate = root/'rust/native-host'
    crate.mkdir(parents=True)
    www = root/'coremain/www'
    asset = www/'assets/nested/fonts/proof.json'
    asset.parent.mkdir(parents=True)
    asset.write_text('{"proof":true}')
    for name in ['log.html', 'log1.html']:
        (www/name).write_text('<script src="/assets/nested/fonts/proof.json?v=test"></script>')
    out = root/'out'
    out.mkdir()
    env = dict(os.environ, CARGO_MANIFEST_DIR=str(crate), OUT_DIR=str(out))
    env.pop('MOSDNS_BUILD_VERSION', None)
    def run():
        return subprocess.run([str(binary)], env=env, capture_output=True, text=True)
    good = run()
    assert good.returncode == 0, good.stderr
    first = (out/'embedded_ui.rs').read_text()
    assert '/assets/nested/fonts/proof.json' in first
    assert run().returncode == 0
    assert (out/'embedded_ui.rs').read_text() == first
    results['nested_deterministic_manifest'] = True
    data = asset.read_bytes()
    asset.unlink()
    assert run().returncode != 0
    results['missing_referenced_asset_rejected'] = True
    asset.write_bytes(data)
    html = (www/'log1.html').read_bytes()
    (www/'log1.html').unlink()
    assert run().returncode != 0
    results['missing_root_rejected'] = True
    (www/'log1.html').write_bytes(html)
    asset.unlink()
    asset.symlink_to(www/'log.html')
    assert run().returncode != 0
    results['file_symlink_rejected'] = True
    asset.unlink()
    asset.write_bytes(data)
    linked = www/'assets/linked'
    linked.symlink_to(asset.parent, target_is_directory=True)
    assert run().returncode != 0
    results['directory_symlink_rejected'] = True
    linked.unlink()
    hidden = www/'assets/private.key'
    hidden.write_text('test-only-fixture')
    assert run().returncode != 0
    results['unexpected_key_rejected'] = True
    hidden.unlink()
    assert run().returncode == 0
print(json.dumps(results, indent=2))
